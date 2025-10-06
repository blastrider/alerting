use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alerting::Result;
use alerting::config::Config;
use alerting::error::{ConfigError, Error as AlertError};
use alerting::telemetry::init_tracing;
use alerting::zbx_client::{Problem, ZbxClient};
use async_channel::{Sender, TrySendError, bounded};
use lru::LruCache;
use tokio::signal;
use tokio::time::sleep;
use tracing::{info, warn};

use super::cli::Cli;
use super::notifier::{NotificationItem, run_notifier};
use super::rate_limit::LeakyBucket;

const DEFAULT_CONFIG: &str = "config.toml";

pub async fn run(cli: Cli) -> Result<()> {
    init_tracing(cli.log_filter.as_deref(), cli.json_logs)?;

    let config_path = cli.config.unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG));
    let mut config = Config::from_env_and_file(&config_path)?;

    if let Some(interval) = cli.interval {
        config.poll_interval = interval;
    }
    if let Some(max_notif) = cli.max_notif {
        if !(1..=100).contains(&max_notif) {
            return Err(AlertError::from(ConfigError::InvalidField {
                field: "cli.max_notif",
                message: "value must be between 1 and 100".to_string(),
            }));
        }
        config.max_notif = max_notif;
    }

    #[cfg(target_os = "windows")]
    if let Some(mut body) = cli.test_toast {
        let summary = "Test Alerting";
        if body.trim().is_empty() {
            body = "Toast de test déclenché par --test-toast".to_string();
        }
        super::notifier::send_test_toast(
            summary,
            &body,
            &config.notify.appname,
            config.notify.icon.as_deref(),
            &config.notify.open_label,
        )?;
        info!("notification de test envoyée, arrêt du programme");
        return Ok(());
    }

    let client = ZbxClient::new(
        config.base_url.clone(),
        config.token.clone(),
        config.http_request_timeout,
        config.http_connect_timeout,
        cli.insecure,
    )?;

    let (tx, rx) = bounded(config.queue_capacity);
    let notifier_client = client.clone();
    let notifier = tokio::spawn(run_notifier(
        rx,
        config.notify.clone(),
        notifier_client,
        cli.dry_run,
    ));

    let mut dedup = LruCache::new(
        NonZeroUsize::new(config.dedup_cache_size).expect("dedup cache size validated to be > 0"),
    );
    let mut bucket = LeakyBucket::new(config.rate_limit.max_events, config.rate_limit.per);

    loop {
        let iteration_start = Instant::now();
        tokio::select! {
            biased;
            _ = signal::ctrl_c() => {
                info!("shutdown signal received, stopping loop");
                break;
            }
            res = poll_once(&client, &config, &mut dedup, &mut bucket, &tx) => {
                res?;
            }
        }

        if cli.once {
            break;
        }

        let elapsed = iteration_start.elapsed();
        let sleep_dur = config
            .poll_interval
            .checked_sub(elapsed)
            .unwrap_or_default();

        if sleep_dur.is_zero() {
            continue;
        }

        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("shutdown signal received, stopping loop");
                break;
            }
            _ = sleep(sleep_dur) => {}
        }
    }

    tx.close();
    if let Err(err) = notifier.await {
        warn!(error = %err, "notifier task terminated unexpectedly");
    }

    Ok(())
}

pub(super) async fn poll_once(
    client: &ZbxClient,
    config: &Config,
    dedup: &mut LruCache<(String, i64), ()>,
    bucket: &mut LeakyBucket,
    tx: &Sender<NotificationItem>,
) -> Result<()> {
    let problems = client
        .active_problems(config.limit, config.ack_filter)
        .await?;

    let event_ids: Vec<String> = problems.iter().map(|p| p.event_id.clone()).collect();
    let hosts = client.resolve_hosts(&event_ids, config.concurrency).await?;

    let mut rows: Vec<_> = problems.into_iter().zip(hosts.into_iter()).collect();

    rows.sort_unstable_by(|(a, _), (b, _)| {
        (a.acknowledged as u8)
            .cmp(&(b.acknowledged as u8))
            .then(b.severity.cmp(&a.severity))
            .then(b.clock.cmp(&a.clock))
    });
    if rows.len() > config.max_notif {
        rows.truncate(config.max_notif);
    }

    for (problem, host) in rows {
        if problem.acknowledged && !config.notify.notify_acked {
            continue;
        }

        let key = (problem.event_id.clone(), problem.last_change);
        if dedup.contains(&key) {
            debug_dup(&problem);
            continue;
        }
        dedup.put(key, ());

        let now = Instant::now();
        if !bucket.try_acquire(now) {
            warn!(event_id = %problem.event_id, "dropping notification due to rate limit");
            continue;
        }

        let latency = compute_latency_ms(problem.clock);
        let host_label = host
            .as_ref()
            .map(|h| h.display_name.as_str())
            .unwrap_or("<unknown>");

        info!(
            event_id = %problem.event_id,
            host = host_label,
            severity = ?problem.severity,
            ack_state = problem.acknowledged,
            latency_ms = latency.unwrap_or_default(),
            "queueing notification"
        );

        let open_url = config
            .open_url_fmt
            .as_deref()
            .map(|fmt| fmt.replace("{eventid}", problem.event_id.as_str()));

        let item = NotificationItem {
            problem,
            host,
            open_url,
        };

        match tx.try_send(item) {
            Ok(()) => {}
            Err(TrySendError::Full(item)) => {
                warn!(
                    "notification queue full; dropping event {}",
                    item.problem.event_id
                );
            }
            Err(TrySendError::Closed(_)) => break,
        }
    }

    Ok(())
}

fn compute_latency_ms(clock: i64) -> Option<u128> {
    if clock < 0 {
        return None;
    }
    let event_time = UNIX_EPOCH.checked_add(Duration::from_secs(clock as u64))?;
    SystemTime::now()
        .duration_since(event_time)
        .ok()
        .map(|d| d.as_millis())
}

fn debug_dup(problem: &Problem) {
    tracing::debug!(
        event_id = %problem.event_id,
        last_change = problem.last_change,
        "duplicate notification skipped"
    );
}

#[cfg(test)]
mod tests {
    use super::super::notifier::NotificationItem;
    use super::super::rate_limit::LeakyBucket;
    use super::poll_once;
    use alerting::config::{Config, NotifySettings, RateLimit};
    use alerting::types::AckFilter;
    use alerting::zbx_client::ZbxClient;
    use async_channel::bounded;
    use lru::LruCache;
    use secrecy::SecretString;
    use std::num::NonZeroUsize;
    use std::time::Duration;
    use url::Url;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn poll_once_skips_duplicate_events() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(body_string_contains("problem.get"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jsonrpc": "2.0",
                "result": [
                    {
                        "eventid": "77",
                        "clock": "1700000000",
                        "lastchange": "1700000001",
                        "severity": "3",
                        "name": "Duplicate",
                        "acknowledged": "0"
                    },
                    {
                        "eventid": "77",
                        "clock": "1700000000",
                        "lastchange": "1700000001",
                        "severity": "3",
                        "name": "Duplicate",
                        "acknowledged": "0"
                    }
                ],
                "id": 1
            })))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(body_string_contains("event.get"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jsonrpc": "2.0",
                "result": [
                    {
                        "hosts": [
                            { "host": "srv", "name": "Srv", "status": "0" }
                        ]
                    }
                ],
                "id": 1
            })))
            .mount(&server)
            .await;

        let config = Config {
            base_url: Url::parse(&server.uri()).unwrap(),
            token: SecretString::from("token"),
            limit: 10,
            concurrency: 2,
            ack_filter: AckFilter::All,
            max_notif: 10,
            queue_capacity: 4,
            dedup_cache_size: 8,
            rate_limit: RateLimit {
                max_events: 10,
                per: Duration::from_secs(60),
            },
            poll_interval: Duration::from_millis(10),
            open_url_fmt: None,
            notify: NotifySettings {
                appname: "test".into(),
                sticky: false,
                timeout: None,
                default_timeout: false,
                icon: None,
                open_label: "Open".into(),
                notify_acked: true,
            },
            http_connect_timeout: Duration::from_millis(100),
            http_request_timeout: Duration::from_millis(200),
        };

        let client = ZbxClient::new(
            config.base_url.clone(),
            config.token.clone(),
            config.http_request_timeout,
            config.http_connect_timeout,
            true,
        )
        .unwrap();

        let (tx, rx) = bounded::<NotificationItem>(4);
        let mut dedup = LruCache::new(NonZeroUsize::new(config.dedup_cache_size).unwrap());
        let mut bucket = LeakyBucket::new(10, Duration::from_secs(60));

        poll_once(&client, &config, &mut dedup, &mut bucket, &tx)
            .await
            .unwrap();

        tx.close();
        let mut items = Vec::new();
        while let Ok(item) = rx.try_recv() {
            items.push(item);
        }
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].problem.event_id, "77");
    }
}
