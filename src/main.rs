use anyhow::Result;
use chrono::{Local, TimeZone};
use notify_rust::{Urgency};
use std::{env, path::PathBuf};

mod config;
mod domain;
mod util;
mod zbx;
mod ui;

use config::Config;
use domain::severity::Severity;
use util::time::fmt_epoch_local;
use zbx::ZbxClient;
use ui::notify::{compute_timeout, send_toast};

/// Parse booléen d’ENV (1/true/yes/y, insensible à la casse).
fn env_bool(name: &str) -> bool {
    match env::var(name) {
        Ok(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "y"),
        Err(_) => false,
    }
}

/// Construit l’URL "ouvrir" à partir d’un format ENV, ex:
///   ZBX_OPEN_URL_FMT="https://...&filter_eventid={eventid}"
fn make_open_url(fmt: Option<&str>, eventid: &str) -> Option<String> {
    fmt.map(|f| f.replace("{eventid}", eventid))
}

/// Mappe la sévérité vers l’urgence de la notif.
fn urgency_for_severity(sev: Severity) -> Urgency {
    match sev {
        Severity::Disaster | Severity::High => Urgency::Critical,
        Severity::Average | Severity::Warning => Urgency::Normal,
        Severity::Information | Severity::NotClassified => Urgency::Low,
        Severity::Unknown(_) => Urgency::Normal,
    }
}

/// Ne garder que `max` problèmes les plus **sévères**, puis les plus **récents**.
fn pick_top(problems: &mut Vec<zbx::types::Problem>, max: usize) {
    problems.sort_unstable_by(|a, b| {
        b.severity
            .cmp(&a.severity)   // sévérité décroissante
            .then(b.clock.cmp(&a.clock)) // plus récent d'abord
    });
    if problems.len() > max {
        problems.truncate(max);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // --- Config API/Zabbix
    let cfg = Config::from_env()?;
    let client = ZbxClient::new(&cfg.url, &cfg.token)?;

    // --- Config notifications via ENV
    let appname = env::var("NOTIFY_APPNAME").unwrap_or_else(|_| "Innlog Agent".to_string());
    let sticky = env_bool("NOTIFY_STICKY");
    let timeout_ms = env::var("NOTIFY_TIMEOUT_MS").ok().and_then(|s| s.parse().ok());
    let timeout_default = env_bool("NOTIFY_TIMEOUT_DEFAULT");
    let timeout = compute_timeout(sticky, timeout_ms, timeout_default);
    let icon_path: Option<PathBuf> = env::var("NOTIFY_ICON").ok().map(PathBuf::from);
    let open_fmt = env::var("ZBX_OPEN_URL_FMT").ok();
    let open_label = env::var("NOTIFY_OPEN_LABEL").unwrap_or_else(|_| "Ouvrir".to_string());

    // 1) Récupérer les problèmes
    let mut problems = client.recent_problems(cfg.limit).await?;

    // 2) Ne garder que les N derniers “actifs” (tri: sévérité desc, horodatage desc)
    pick_top(&mut problems, cfg.max_notif);

    // 3) Résoudre les hôtes (parallélisé) UNIQUEMENT pour ces N éléments
    let eventids: Vec<String> = problems.iter().map(|p| p.eventid.clone()).collect();
    let hosts = client
        .resolve_hosts_concurrent(&eventids, cfg.concurrency)
        .await?;

    // 4) Affichage + notifications (seulement ces N éléments)
    for (p, host_opt) in problems.into_iter().zip(hosts.into_iter()) {
        let host = host_opt.as_deref().unwrap_or("-");
        let sev = Severity::from(p.severity);
        let when = Local
            .timestamp_opt(p.clock, 0)
            .single()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| format!("(horodatage invalide: {})", p.clock));
        let when_local = fmt_epoch_local(p.clock);

        // Console (pour logs)
        println!(
            "Problem #{} | Host: {} | Severity: {} ({}) | Name: {} | At: {}",
            p.eventid, host, p.severity, sev, p.name, when
        );

        // Notification
        let summary = format!("Zabbix: {sev} – {host}");
        let body = format!("{}\nEvent: {}\nQuand: {}", p.name, p.eventid, when_local);
        let urgency = urgency_for_severity(sev);
        let action_url = make_open_url(open_fmt.as_deref(), &p.eventid);
        let icon_ref = icon_path.as_deref();

        // IMPORTANT : ne pas bloquer l’exécution (send_toast spawne un thread pour wait_for_action)
        let _ = send_toast(
            &summary,
            &body,
            urgency,
            timeout,
            &appname,
            icon_ref,
            None,                  // replace_id (voir note ci-dessous)
            action_url.as_deref(), // bouton "Ouvrir"
            &open_label,
        );
    }

    Ok(())
}
