use std::path::PathBuf;
use std::time::Duration;

use clap::{ArgAction, Parser};
use humantime::parse_duration;

#[allow(clippy::struct_excessive_bools)]
#[derive(Parser, Debug)]
#[command(author, version, about = "Alerting bridge for Zabbix", long_about = None)]
pub struct Cli {
    /// Chemin du fichier de configuration TOML.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Exécute une seule itération de poll/push puis quitte.
    #[arg(long, action = ArgAction::SetTrue)]
    pub once: bool,

    /// Force l'intervalle de poll (ex. "30s").
    #[arg(long, value_parser = parse_duration)]
    pub interval: Option<Duration>,

    /// Limite maximale de notifications par itération.
    #[arg(long, value_parser = clap::value_parser!(usize))]
    pub max_notif: Option<usize>,

    /// Autorise les URLs HTTP non chiffrées.
    #[arg(long, action = ArgAction::SetTrue)]
    pub insecure: bool,

    /// N'émet pas de notifications, logue uniquement ce qui serait envoyé.
    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,

    /// Utilise un layer JSON pour les logs (`--features json-logs`).
    #[arg(long, action = ArgAction::SetTrue)]
    pub json_logs: bool,

    /// Filtre de logs explicite (ex. "alerting=debug").
    #[arg(long, value_name = "FILTER")]
    pub log_filter: Option<String>,

    /// Envoie une notification de test (Windows uniquement) et quitte.
    #[cfg(target_os = "windows")]
    #[arg(long, value_name = "TEXTE")]
    pub test_toast: Option<String>,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}
