use anyhow::Result;
use chrono::{Local, TimeZone};
use notify_rust::Urgency;

use config::Config;
use domain::severity::Severity;
use util::time::fmt_epoch_local;
use zbx::ZbxClient;
use ui::notify::{compute_timeout, send_toast};
use ui::notify::AckControls;

mod config;
mod domain;
mod util;
mod zbx;
mod ui;

/// Construit l’URL "ouvrir" à partir d’un format ENV/Fichier, ex:
///   zbx_open_url_fmt="https://...&filter_eventid={eventid}"
fn make_open_url(fmt: Option<&str>, eventid: &str) -> Option<String> {
    fmt.map(|f| f.replace("{eventid}", eventid))
}

fn urgency_for_severity(sev: Severity) -> Urgency {
    match sev {
        Severity::Disaster | Severity::High => Urgency::Critical,
        Severity::Average | Severity::Warning => Urgency::Normal,
        Severity::Information | Severity::NotClassified => Urgency::Low,
        Severity::Unknown(_) => Urgency::Normal,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // --- Config unifiée (ENV > fichier > défauts)
    let cfg = Config::load()?;

    // --- Client Zabbix
    let client = ZbxClient::new(&cfg.url, &cfg.token)?;

    // --- Timeout notifications
    let timeout = compute_timeout(cfg.notify_sticky, cfg.notify_timeout_ms, cfg.notify_timeout_default);

    // 1) Récupérer les problèmes actifs selon ACK_FILTER
    let problems = client.active_problems(cfg.limit, cfg.ack_filter).await?;

    // 2) Résoudre les hôtes (parallélisé) pour TOUS les problèmes récupérés
    let eventids: Vec<String> = problems.iter().map(|p| p.eventid.clone()).collect();
    let hosts = client.resolve_hosts_concurrent(&eventids, cfg.concurrency).await?;

    // 3) Zipper + filtrer hôtes désactivés
    let mut rows: Vec<_> = problems
        .into_iter()
        .zip(hosts.into_iter())
        .filter_map(|(p, hm)| {
            let hm = hm?;
            match hm.status {
                Some(1) => None,        // désactivé -> on ignore
                _ => Some((p, hm)),      // activé ou inconnu -> on garde
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
        let when = Local.timestamp_opt(p.clock, 0).single()
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
            Some(AckControls{
                client: client.clone(),
                eventid: p.eventid.clone(),
                ask_message: true,                 // ouvre un prompt texte (facultatif)
                allow_unack: p.acknowledged,       // si déjà ACK, proposer "Unack"
                ack_label: None,
                unack_label: None,
            }),
        );
    }

    Ok(())
}
