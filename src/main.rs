use anyhow::Result;

mod config;
mod domain;
mod util;
mod zbx;

use config::Config;
use domain::severity::Severity;
use util::time::fmt_epoch_local;
use zbx::ZbxClient;



#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::from_env()?;
    let client = ZbxClient::new(&cfg.url, &cfg.token)?;

    // 1) Récupérer les problèmes
    let problems = client.recent_problems(cfg.limit).await?;

    // 2) Résoudre les hôtes en parallèle (borne par CONCURRENCY)
    let eventids: Vec<String> = problems.iter().map(|p| p.eventid.clone()).collect();
    let hosts = client
        .resolve_hosts_concurrent(&eventids, cfg.concurrency)
        .await?;

    // 3) Affichage
    for (p, host_opt) in problems.into_iter().zip(hosts.into_iter()) {
        let host = host_opt.as_deref().unwrap_or("-");
        let sev = Severity::from(p.severity);
        let when = fmt_epoch_local(p.clock);

        println!(
            "Problem #{} | Host: {} | Severity: {} ({}) | Name: {} | At: {}",
            p.eventid,
            host,
            p.severity, // code numérique
            sev,        // libellé
            p.name,
            when
        );
    }

    Ok(())
}
