<<<<<<< HEAD
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use alerting::Result;
use alerting::config::{Config, NotifySettings};
use alerting::error::{ConfigError, Error as AlertError};
use alerting::telemetry::init_tracing;
use alerting::types::Severity;
use alerting::zbx_client::{HostMeta, Problem, ZbxClient};

use async_channel::{Receiver, Sender, TrySendError, bounded};
use clap::{ArgAction, Parser};
use humantime::parse_duration;
use lru::LruCache;
use std::error::Error as StdError;
use tokio::time::sleep;
use tracing::{error, info, warn};

const DEFAULT_CONFIG: &str = "config.toml";
#[derive(Parser, Debug)]
#[command(author, version, about = "Alerting bridge for Zabbix", long_about = None)]
struct Cli {
    /// Chemin du fichier de configuration TOML.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Exécute une seule itération de poll/push puis quitte.
    #[arg(long, action = ArgAction::SetTrue)]
    once: bool,

    /// Force l'intervalle de poll (ex. "30s").
    #[arg(long, value_parser = parse_duration)]
    interval: Option<Duration>,

    /// Limite maximale de notifications par itération.
    #[arg(long, value_parser = clap::value_parser!(usize))]
    max_notif: Option<usize>,

    /// Autorise les URLs HTTP non chiffrées.
    #[arg(long, action = ArgAction::SetTrue)]
    insecure: bool,

    /// N'émet pas de notifications, logue uniquement ce qui serait envoyé.
    #[arg(long, action = ArgAction::SetTrue)]
    dry_run: bool,

    /// Utilise un layer JSON pour les logs (`--features json-logs`).
    #[arg(long, action = ArgAction::SetTrue)]
    json_logs: bool,

    /// Filtre de logs explicite (ex. "alerting=debug").
    #[arg(long, value_name = "FILTER")]
    log_filter: Option<String>,

    /// Envoie une notification de test (Windows uniquement) et quitte.
    #[cfg(target_os = "windows")]
    #[arg(long, value_name = "TEXTE")]
    test_toast: Option<String>,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
=======
#[path = "alerting/app.rs"]
mod app;
#[path = "alerting/cli.rs"]
mod cli;
#[path = "alerting/notifier/mod.rs"]
mod notifier;
#[path = "alerting/rate_limit.rs"]
mod rate_limit;

use std::error::Error as StdError;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = cli::Cli::parse_args();
    match app::run(cli).await {
>>>>>>> feat/hardening-observability-ci
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            report_error(&err);
            std::process::ExitCode::from(1)
        }
    }
}

<<<<<<< HEAD
async fn run(cli: Cli) -> Result<()> {
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
        notify_backends::send_toast(
            summary,
            &body,
            ToastUrgency::Normal,
            ToastTimeout::Milliseconds(5_000),
            "",
            config.notify.icon.as_deref(),
            None,
            &config.notify.open_label,
            None,
        )
        .map_err(AlertError::from)?;
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
            _ = tokio::signal::ctrl_c() => {
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
            _ = tokio::signal::ctrl_c() => {
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

async fn poll_once(
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

async fn run_notifier(
    rx: Receiver<NotificationItem>,
    notify: NotifySettings,
    client: ZbxClient,
    dry_run: bool,
) {
    while let Ok(item) = rx.recv().await {
        if dry_run {
            info!(
                event_id = %item.problem.event_id,
                host = item.host.as_ref().map(|h| h.display_name.as_str()).unwrap_or("<unknown>"),
                severity = ?item.problem.severity,
                "dry-run: would emit notification"
            );
            continue;
        }

        if let Err(err) = send_notification(&notify, &client, &item) {
            error!(error = %err, event_id = %item.problem.event_id, "failed to send notification");
        }
    }
}

fn send_notification(
    notify: &NotifySettings,
    client: &ZbxClient,
    item: &NotificationItem,
) -> Result<()> {
    let severity = item.problem.severity;
    let urgency = match severity {
        Severity::Disaster | Severity::High => ToastUrgency::Critical,
        Severity::Average | Severity::Warning => ToastUrgency::Normal,
        Severity::Info => ToastUrgency::Low,
    };

    let timeout_ms = notify.timeout.and_then(|dur| u128_to_u32(dur.as_millis()));
    let timeout = compute_timeout(notify.sticky, timeout_ms, notify.default_timeout);

    let host_label = item
        .host
        .as_ref()
        .map(|h| h.display_name.as_str())
        .unwrap_or("<unknown>");

    let summary = format!("{severity:?} – {host_label}");
    let body = format!(
        "Event #{} {}\n{}",
        item.problem.event_id,
        if item.problem.acknowledged {
            "[ACK]"
        } else {
            "[UNACK]"
        },
        item.problem.name
    );

    let open_url = item.open_url.clone();

    #[cfg(not(target_os = "linux"))]
    let _ = client;

    #[cfg(target_os = "linux")]
    let ack_action =
        (!item.problem.acknowledged).then(|| AckAction::new(client, &item.problem.event_id));
    #[cfg(not(target_os = "linux"))]
    let ack_action = None;

    notify_backends::send_toast(
        &summary,
        &body,
        urgency,
        timeout,
        &notify.appname,
        notify.icon.as_deref(),
        open_url.as_deref(),
        &notify.open_label,
        ack_action,
    )
    .map_err(AlertError::from)?;
    Ok(())
}

struct NotificationItem {
    problem: Problem,
    host: Option<HostMeta>,
    open_url: Option<String>,
}

#[derive(Clone)]
struct AckAction {
    client: ZbxClient,
    event_id: String,
}

impl AckAction {
    fn new(client: &ZbxClient, event_id: &str) -> Self {
        Self {
            client: client.clone(),
            event_id: event_id.to_string(),
        }
    }

    fn spawn_with_message(self, message: Option<String>) {
        let AckAction { client, event_id } = self;
        let message_clone = message.clone();
        tokio::spawn(async move {
            match client.ack_event(&event_id, message_clone).await {
                Ok(()) => {
                    if let Some(msg) = message {
                        tracing::info!(%event_id, message = %msg, "event acknowledged from toast");
                    } else {
                        tracing::info!(%event_id, "event acknowledged from toast");
                    }
                }
                Err(err) => {
                    tracing::warn!(%event_id, error = %err, "failed to acknowledge event from toast")
                }
            }
        });
    }
}

struct LeakyBucket {
    window: Duration,
    max: usize,
    samples: VecDeque<Instant>,
}

impl LeakyBucket {
    fn new(max: usize, window: Duration) -> Self {
        Self {
            window,
            max,
            samples: VecDeque::with_capacity(max.max(1)),
        }
    }

    fn try_acquire(&mut self, now: Instant) -> bool {
        while let Some(front) = self.samples.front() {
            if now.duration_since(*front) > self.window {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        if self.samples.len() >= self.max {
            return false;
        }
        self.samples.push_back(now);
        true
    }
}

mod notify_backends {
    use alerting::error::NotifyError;

    #[cfg(target_os = "linux")]
    pub fn send_toast(
        summary: &str,
        body: &str,
        urgency: super::ToastUrgency,
        timeout: super::ToastTimeout,
        appname: &str,
        icon: Option<&std::path::Path>,
        open_url: Option<&str>,
        open_label: &str,
        ack_action: Option<super::AckAction>,
    ) -> std::result::Result<(), NotifyError> {
        linux::send_toast(
            summary, body, urgency, timeout, appname, icon, open_url, open_label, ack_action,
        )
    }

    #[cfg(not(target_os = "linux"))]
    pub fn send_toast(
        summary: &str,
        body: &str,
        urgency: super::ToastUrgency,
        timeout: super::ToastTimeout,
        appname: &str,
        icon: Option<&std::path::Path>,
        open_url: Option<&str>,
        open_label: &str,
        _ack_action: Option<super::AckAction>,
    ) -> std::result::Result<(), NotifyError> {
        #[cfg(target_os = "windows")]
        {
            return windows::send_toast(
                summary,
                body,
                urgency,
                timeout,
                appname,
                icon,
                open_url,
                open_label,
                _ack_action,
            );
        }

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = (
                summary,
                body,
                urgency,
                timeout,
                appname,
                icon,
                open_url,
                open_label,
                _ack_action,
            );
            Err(NotifyError::Backend)
        }
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use alerting::error::NotifyError;
        use notify_rust::{Notification, Timeout as LibTimeout, Urgency as LibUrgency};
        use std::path::Path;
        use std::process::{Command, Stdio};
        use tracing::trace;

        use super::super::{AckAction, ToastTimeout, ToastUrgency};

        pub fn send_toast(
            summary: &str,
            body: &str,
            urgency: ToastUrgency,
            timeout: ToastTimeout,
            appname: &str,
            icon: Option<&Path>,
            open_url: Option<&str>,
            open_label: &str,
            ack_action: Option<AckAction>,
        ) -> std::result::Result<(), NotifyError> {
            let mut builder = Notification::new();
            builder
                .summary(summary)
                .body(body)
                .appname(appname)
                .urgency(map_urgency(urgency))
                .timeout(map_timeout(timeout));

            if let Some(icon_path) = icon {
                builder.icon(&icon_path.to_string_lossy());
            }

            const ACK_KEY: &str = "ack";
            const OPEN_KEY: &str = "open";
            const DISMISS_KEY: &str = "dismiss";
            const ACK_LABEL: &str = "Acquitter";

            if ack_action.is_some() {
                builder.action(ACK_KEY, ACK_LABEL);
            }

            if open_url.is_some() {
                builder.action(OPEN_KEY, open_label);
            }

            builder.action(DISMISS_KEY, "Ignorer");

            let handle = builder.show().map_err(|_| NotifyError::Backend)?;
            let open = open_url.map(|url| url.to_string());
            let ack = ack_action.clone();

            handle.wait_for_action(move |action| match action {
                OPEN_KEY => {
                    if let Some(url) = open.as_deref() {
                        let _ = Command::new("xdg-open")
                            .arg(url)
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .spawn();
                    }
                }
                ACK_KEY => {
                    if let Some(ack_action) = ack.clone() {
                        trace!("ack action triggered from toast");
                        let message = prompt_ack_message();
                        ack_action.spawn_with_message(message);
                    }
                }
                _ => {}
            });
            Ok(())
        }

        fn map_urgency(urgency: ToastUrgency) -> LibUrgency {
            match urgency {
                ToastUrgency::Low => LibUrgency::Low,
                ToastUrgency::Normal => LibUrgency::Normal,
                ToastUrgency::Critical => LibUrgency::Critical,
            }
        }

        fn map_timeout(timeout: ToastTimeout) -> LibTimeout {
            match timeout {
                ToastTimeout::Default => LibTimeout::Default,
                ToastTimeout::Never => LibTimeout::Never,
                ToastTimeout::Milliseconds(ms) => LibTimeout::Milliseconds(ms),
            }
        }

        fn prompt_ack_message() -> Option<String> {
            let output = Command::new("zenity")
                .arg("--entry")
                .arg("--title")
                .arg("Acquitter l'evenement")
                .arg("--text")
                .arg("Message d'acquittement (laisser vide pour aucun)")
                .output();

            let output = match output {
                Ok(out) => out,
                Err(err) => {
                    trace!(error = %err, "failed to launch zenity for ack message");
                    return None;
                }
            };

            if !output.status.success() {
                return None;
            }

            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if text.is_empty() { None } else { Some(text) }
        }
    }

    #[cfg(target_os = "windows")]
    mod windows {
        use alerting::error::NotifyError;
        use std::path::Path;
        use windows::UI::Notifications::{NotificationSetting, ToastNotificationManager};
        use windows::core::HSTRING;
        use winrt_notification::{Duration as WinDuration, LoopableSound, Scenario, Sound, Toast};

        use super::super::{ToastTimeout, ToastUrgency};

        pub fn send_toast(
            summary: &str,
            body: &str,
            urgency: ToastUrgency,
            timeout: ToastTimeout,
            appname: &str,
            _icon: Option<&Path>,
            _open_url: Option<&str>,
            _open_label: &str,
            _ack_action: Option<super::super::AckAction>,
        ) -> std::result::Result<(), NotifyError> {
            let app_id = if appname.trim().is_empty() {
                Toast::POWERSHELL_APP_ID
            } else {
                appname
            };
            let timeout_kind = match timeout {
                ToastTimeout::Never => "never",
                ToastTimeout::Default => "default",
                ToastTimeout::Milliseconds(_) => "custom",
            };
            tracing::debug!(
                summary,
                app_id,
                timeout = timeout_kind,
                urgency = ?urgency,
                "sending windows toast"
            );

            match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id)) {
                Ok(notifier) => {
                    if let Ok(setting) = notifier.Setting() {
                        tracing::debug!(
                            setting = ?setting,
                            "windows toast notification setting"
                        );
                        if setting != NotificationSetting::Enabled {
                            tracing::warn!(
                                ?setting,
                                "toast notifications are disabled for this app"
                            );
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "failed to query toast manager");
                }
            }

            let toast = Toast::new(app_id)
                .title(summary)
                .text1(body)
                .duration(match timeout {
                    ToastTimeout::Never => WinDuration::Long,
                    _ => WinDuration::Short,
                })
                .scenario(match urgency {
                    ToastUrgency::Critical => Scenario::Alarm,
                    ToastUrgency::Normal => Scenario::Reminder,
                    ToastUrgency::Low => Scenario::IncomingCall,
                })
                .sound(match urgency {
                    ToastUrgency::Critical => Some(Sound::Loop(LoopableSound::Alarm)),
                    ToastUrgency::Normal => Some(Sound::Default),
                    ToastUrgency::Low => Some(Sound::Reminder),
                });

            if let Err(err) = toast.show() {
                tracing::warn!(error = %err, "windows toast failed");
                return Err(NotifyError::Backend);
            }
            tracing::debug!("windows toast displayed");
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ToastUrgency {
    Low,
    Normal,
    Critical,
}

#[derive(Clone, Copy, Debug)]
pub enum ToastTimeout {
    Default,
    Never,
    Milliseconds(u32),
}

fn compute_timeout(sticky: bool, timeout_ms: Option<u32>, default_timeout: bool) -> ToastTimeout {
    if sticky {
        ToastTimeout::Never
    } else if let Some(ms) = timeout_ms {
        ToastTimeout::Milliseconds(ms)
    } else if default_timeout {
        ToastTimeout::Default
    } else {
        ToastTimeout::Milliseconds(5_000)
    }
}

fn u128_to_u32(value: u128) -> Option<u32> {
    if value > u32::MAX as u128 {
        None
    } else {
        Some(value as u32)
    }
}

fn report_error(err: &AlertError) {
=======
fn report_error(err: &alerting::error::Error) {
>>>>>>> feat/hardening-observability-ci
    eprintln!("Error: {err}");
    let mut source: Option<&dyn StdError> = err.source();
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
}
<<<<<<< HEAD

#[cfg(test)]
mod tests {
    use super::LeakyBucket;
    use super::{NotificationItem, poll_once};
    use alerting::config::{Config, NotifySettings, RateLimit};
    use alerting::types::AckFilter;
    use alerting::zbx_client::ZbxClient;
    use lru::LruCache;
    use secrecy::SecretString;
    use std::num::NonZeroUsize;
    use std::time::{Duration, Instant};
    use url::Url;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn leaky_bucket_respects_capacity() {
        let mut bucket = LeakyBucket::new(2, Duration::from_secs(5));
        let now = Instant::now();
        assert!(bucket.try_acquire(now));
        assert!(bucket.try_acquire(now));
        assert!(!bucket.try_acquire(now));
    }

    #[test]
    fn leaky_bucket_drains_over_time() {
        let mut bucket = LeakyBucket::new(1, Duration::from_secs(1));
        let now = Instant::now();
        assert!(bucket.try_acquire(now));
        assert!(!bucket.try_acquire(now));
        let later = now.checked_add(Duration::from_secs(2)).unwrap();
        assert!(bucket.try_acquire(later));
    }

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

        let (tx, rx) = async_channel::bounded::<NotificationItem>(4);
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
=======
>>>>>>> feat/hardening-observability-ci
