use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::zbx::AckFilter;

/// Configuration finale, prête à l'emploi.
#[derive(Debug, Clone)]
pub struct Config {
    // Zabbix API
    pub url: String,
    pub token: String,
    pub limit: u32,
    pub concurrency: usize,
    pub ack_filter: AckFilter,

    // Sélection / volume
    pub max_notif: usize,

    // Notifications
    pub notify_appname: String,
    pub notify_sticky: bool,
    pub notify_timeout_ms: Option<u32>,
    pub notify_timeout_default: bool,
    pub notify_icon: Option<PathBuf>,
    pub notify_open_label: String,
    pub notify_acked: bool,

    // Action "Ouvrir"
    pub zbx_open_url_fmt: Option<String>,
}

// -----------------------------
// Chargement fichier (TOML)
// -----------------------------

#[derive(Debug, Clone, Deserialize, Default)]
struct FileConfig {
    zabbix: Option<Zabbix>,
    notify: Option<Notify>,
    app: Option<App>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct Zabbix {
    url: Option<String>,
    token: Option<String>,
    limit: Option<u32>,
    concurrency: Option<usize>,
    ack_filter: Option<String>,   // "unack" | "ack" | "all"
    open_url_fmt: Option<String>, // ex: "https://...&filter_eventid={eventid}"
}

#[derive(Debug, Clone, Deserialize, Default)]
struct Notify {
    appname: Option<String>,       // NOTIFY_APPNAME
    sticky: Option<bool>,          // NOTIFY_STICKY
    timeout_ms: Option<u32>,       // NOTIFY_TIMEOUT_MS
    timeout_default: Option<bool>, // NOTIFY_TIMEOUT_DEFAULT
    icon: Option<String>,          // NOTIFY_ICON (chemin)
    open_label: Option<String>,    // NOTIFY_OPEN_LABEL
    notify_acked: Option<bool>,    // NOTIFY_ACKED
}

#[derive(Debug, Clone, Deserialize, Default)]
struct App {
    max_notif: Option<usize>, // MAX_NOTIF
}

impl FileConfig {
    fn load_from(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let txt = fs::read_to_string(path)
            .with_context(|| format!("Lecture fichier config: {}", path.display()))?;
        let cfg: FileConfig =
            toml::from_str(&txt).with_context(|| format!("Parse TOML: {}", path.display()))?;
        Ok(Some(cfg))
    }
}

// -----------------------------
// Merge ENV > fichier > défauts
// -----------------------------

impl Config {
    pub fn load() -> Result<Self> {
        // 0) Fichier
        let cfg_path = env::var("CONFIG_FILE").unwrap_or_else(|_| "config.toml".to_string());
        let file_cfg = FileConfig::load_from(Path::new(&cfg_path))?.unwrap_or_default();

        // Accès rapides
        let z = file_cfg.zabbix.unwrap_or_default();
        let n = file_cfg.notify.unwrap_or_default();
        let a = file_cfg.app.unwrap_or_default();

        // 1) Zabbix
        let url = pick_str(
            "ZBX_URL",
            z.url,
            "https://zabbix.example.com/api_jsonrpc.php",
        );
        let token = env::var("ZBX_TOKEN")
            .ok()
            .or(z.token)
            .context("Token requis (ZBX_TOKEN ou config.toml [zabbix].token)")?;
        let limit = pick_parse_u32("LIMIT", z.limit, 20);
        let concurrency = pick_parse_usize("CONCURRENCY", z.concurrency, 8);
        let ack_filter = pick_ack_filter(z.ack_filter);

        // 2) App
        let max_notif = pick_parse_usize("MAX_NOTIF", a.max_notif, 5);

        // 3) Notifications
        let notify_appname = pick_str("NOTIFY_APPNAME", n.appname, "Alerting-Agent");
        let notify_sticky = pick_bool("NOTIFY_STICKY", n.sticky, false);
        let notify_timeout_ms = pick_opt_parse_u32("NOTIFY_TIMEOUT_MS", n.timeout_ms);
        let notify_timeout_default = pick_bool("NOTIFY_TIMEOUT_DEFAULT", n.timeout_default, false);
        let notify_icon = pick_opt_path("NOTIFY_ICON", n.icon);
        let notify_open_label = pick_str("NOTIFY_OPEN_LABEL", n.open_label, "Ouvrir");
        let notify_acked = pick_bool("NOTIFY_ACKED", n.notify_acked, false);

        // 4) URL "Ouvrir"
        let zbx_open_url_fmt = env::var("ZBX_OPEN_URL_FMT").ok().or(z.open_url_fmt);

        Ok(Self {
            url,
            token,
            limit,
            concurrency,
            ack_filter,
            max_notif,
            notify_appname,
            notify_sticky,
            notify_timeout_ms,
            notify_timeout_default,
            notify_icon,
            notify_open_label,
            notify_acked,
            zbx_open_url_fmt,
        })
    }
}

// -----------------------------
// Helpers de merge / parse
// -----------------------------

fn pick_str(env_key: &str, file_val: Option<String>, default_: &str) -> String {
    env::var(env_key)
        .ok()
        .or(file_val)
        .unwrap_or_else(|| default_.to_string())
}
fn pick_bool(env_key: &str, file_val: Option<bool>, default_: bool) -> bool {
    match env::var(env_key) {
        Ok(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "y"),
        Err(_) => file_val.unwrap_or(default_),
    }
}
fn pick_parse_u32(env_key: &str, file_val: Option<u32>, default_: u32) -> u32 {
    env::var(env_key)
        .ok()
        .and_then(|s| s.parse().ok())
        .or(file_val)
        .unwrap_or(default_)
}
fn pick_parse_usize(env_key: &str, file_val: Option<usize>, default_: usize) -> usize {
    env::var(env_key)
        .ok()
        .and_then(|s| s.parse().ok())
        .or(file_val)
        .unwrap_or(default_)
}
fn pick_opt_parse_u32(env_key: &str, file_val: Option<u32>) -> Option<u32> {
    match env::var(env_key) {
        Ok(s) => s.parse().ok(),
        Err(_) => file_val,
    }
}
fn pick_opt_path(env_key: &str, file_val: Option<String>) -> Option<PathBuf> {
    env::var(env_key)
        .ok()
        .map(PathBuf::from)
        .or_else(|| file_val.map(PathBuf::from))
}
fn pick_ack_filter(file_val: Option<String>) -> AckFilter {
    let src = env::var("ACK_FILTER")
        .ok()
        .or(file_val)
        .unwrap_or_else(|| "unack".into());
    match src.to_ascii_lowercase().as_str() {
        "ack" => AckFilter::Ack,
        "all" => AckFilter::All,
        _ => AckFilter::Unack,
    }
}
