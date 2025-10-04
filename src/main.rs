use anyhow::Result;
use chrono::{Local, TimeZone};
use config::Config;
use domain::severity::Severity;
use ui::notify::AckControls;
use ui::notify::{ToastUrgency, compute_timeout, send_toast};
use util::time::fmt_epoch_local;
use zbx::ZbxClient;

#[cfg(target_os = "linux")]
use tokio::runtime::Handle;
mod config;
mod domain;
mod ui;
mod util;
mod zbx;

/// Construit l’URL "ouvrir" à partir d’un format ENV/Fichier, ex:
///   zbx_open_url_fmt="https://...&filter_eventid={eventid}"
fn make_open_url(fmt: Option<&str>, eventid: &str) -> Option<String> {
    fmt.map(|f| f.replace("{eventid}", eventid))
}

fn urgency_for_severity(sev: Severity) -> ToastUrgency {
    match sev {
        Severity::Disaster | Severity::High => ToastUrgency::Critical,
        Severity::Average | Severity::Warning => ToastUrgency::Normal,
        Severity::Information | Severity::NotClassified => ToastUrgency::Low,
        Severity::Unknown(_) => ToastUrgency::Normal,
    }
}

#[tokio::main]
async fn main() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .try_init();

    if let Err(err) = run().await {
        eprintln!("Error: {:#}", err);
        let mut source = err.source();
        while let Some(cause) = source {
            eprintln!("  Caused by: {cause}");
            source = cause.source();
        }
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    // --- Config unifiée (ENV > fichier > défauts)
    let cfg = Config::load()?;

    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let Some(cmd) = args.get(0).map(|s| s.as_str()) {
        #[cfg(target_os = "windows")]
        if cmd == "toast-test" {
            run_toast_test(&cfg, &args)?;
            println!("Toast test envoyé.");
            return Ok(());
        }

        #[cfg(not(target_os = "windows"))]
        if cmd == "toast-test" {
            println!("La commande 'toast-test' est disponible uniquement sous Windows.");
            return Ok(());
        }
    }

    // --- Client Zabbix
    let client = ZbxClient::new(&cfg.url, &cfg.token)?;
    // --- Mode CLI de test : `alerting ack <eventid> [message]` ou `alerting unack <eventid> [message]`
    if let Some(cmd) = args.get(0).map(|s| s.as_str()) {
        match cmd {
            "ack" => {
                let eid = args
                    .get(1)
                    .expect("usage: alerting ack <eventid> [message]");
                let msg = args.get(2).cloned();
                client.ack_event(eid, msg).await?;
                println!("ACK OK for {}", eid);
                return Ok(());
            }
            "unack" => {
                let eid = args
                    .get(1)
                    .expect("usage: alerting unack <eventid> [message]");
                let msg = args.get(2).cloned();
                client.unack_event(eid, msg).await?;
                println!("UNACK OK for {}", eid);
                return Ok(());
            }
            _ => {}
        }
    }

    // --- Timeout notifications
    let timeout = compute_timeout(
        cfg.notify_sticky,
        cfg.notify_timeout_ms,
        cfg.notify_timeout_default,
    );

    // 1) Récupérer les problèmes actifs selon ACK_FILTER
    let problems = client.active_problems(cfg.limit, cfg.ack_filter).await?;

    // 2) Résoudre les hôtes (parallélisé) pour TOUS les problèmes récupérés
    let eventids: Vec<String> = problems.iter().map(|p| p.eventid.clone()).collect();
    let hosts = client
        .resolve_hosts_concurrent(&eventids, cfg.concurrency)
        .await?;

    // 3) Zipper + filtrer hôtes désactivés
    let mut rows: Vec<_> = problems
        .into_iter()
        .zip(hosts.into_iter())
        .filter_map(|(p, hm)| {
            let hm = hm?;
            match hm.status {
                Some(1) => None,    // désactivé -> on ignore
                _ => Some((p, hm)), // activé ou inconnu -> on garde
            }
        })
        .collect();

    // 4) Limiter aux MAX_NOTIF (tri par ack, sévérité, horodatage)
    rows.sort_unstable_by(|(a, _), (b, _)| {
        (a.acknowledged as u8)
            .cmp(&(b.acknowledged as u8))
            .then(b.severity.cmp(&a.severity))
            .then(b.clock.cmp(&a.clock))
    });
    if rows.len() > cfg.max_notif {
        rows.truncate(cfg.max_notif);
    }

    // 5) Affichage + notifications
    for (p, hm) in rows {
        let host = hm.display_name.as_str();
        let sev = Severity::from(p.severity);
        let when = Local
            .timestamp_opt(p.clock, 0)
            .single()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| format!("(horodatage invalide: {})", p.clock));
        let when_local = fmt_epoch_local(p.clock);

        let ack_mark = if p.acknowledged { "ACK" } else { "UNACK" };
        println!(
            "Problem #{} | {} | Host: {} | Severity: {} ({}) | Name: {} | At: {}",
            p.eventid, ack_mark, host, p.severity, sev, p.name, when
        );

        // Notification : ignorer les acquittées si notify_acked = false
        if p.acknowledged && !cfg.notify_acked {
            continue;
        }

        let summary = if p.acknowledged {
            format!("Zabbix: [ACK] {sev} – {host}")
        } else {
            format!("Zabbix: {sev} – {host}")
        };
        let body = format!("{}\nEvent: {}\nQuand: {}", p.name, p.eventid, when_local);
        let urgency = urgency_for_severity(sev);
        let action_url = make_open_url(cfg.zbx_open_url_fmt.as_deref(), &p.eventid);

        #[cfg(target_os = "linux")]
        let ack_controls = Some(AckControls {
            handle: Handle::current(),
            client: client.clone(),
            eventid: p.eventid.clone(),
            ask_message: true,           // ouvre un prompt texte (facultatif)
            allow_unack: p.acknowledged, // si déjà ACK, proposer "Unack"
            ack_label: None,
            unack_label: None,
        });

        #[cfg(target_os = "windows")]
        let ack_controls = if p.acknowledged {
            None
        } else {
            Some(AckControls {
                client: client.clone(),
                eventid: p.eventid.clone(),
                ask_message: true,
                ack_label: Some("Valider".to_string()),
            })
        };

        #[cfg(all(not(target_os = "linux"), not(target_os = "windows")))]
        let ack_controls = None::<AckControls>;

        let _ = send_toast(
            &summary,
            &body,
            urgency,
            timeout,
            &cfg.notify_appname,
            cfg.notify_icon.as_deref(),
            None,
            action_url.as_deref(),
            &cfg.notify_open_label,
            ack_controls,
        );
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn run_toast_test(cfg: &Config, args: &[String]) -> Result<()> {
    let summary = args
        .get(1)
        .cloned()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Alerting Test".to_string());
    let body = if args.len() > 2 {
        args[2..].join(" ")
    } else {
        summary.clone()
    };

    let timeout = compute_timeout(
        cfg.notify_sticky,
        cfg.notify_timeout_ms,
        cfg.notify_timeout_default,
    );

    let ack_controls = None::<AckControls>;

    send_toast(
        &summary,
        &body,
        ToastUrgency::Normal,
        timeout,
        &cfg.notify_appname,
        cfg.notify_icon.as_deref(),
        None,
        None,
        &cfg.notify_open_label,
        ack_controls,
    )
}
