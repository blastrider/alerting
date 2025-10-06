use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use serde::Deserialize;
use serde_with::serde_as;
use url::Url;

use crate::Result;
use crate::error::ConfigError;
use crate::types::AckFilter;

use super::defaults::{
    default_ack_filter, default_concurrency, default_dedup_cache_size, default_limit,
    default_max_notif, default_notify_appname, default_open_label, default_poll_interval,
    default_queue_bound, default_rate_limit_max, default_rate_limit_window,
};
use super::env::{env_bool, env_duration, env_parse, env_string};
use super::{
    Config, DEFAULT_CONNECT_TIMEOUT, DEFAULT_HTTP_TIMEOUT, HumantimeDuration, MAX_NOTIF_BOUNDS,
    NotifySettings, RateLimit,
};

pub(super) fn load(path: impl AsRef<Path>) -> std::result::Result<RawConfig, ConfigError> {
    let mut builder = ::config::Config::builder();
    let path = path.as_ref();
    builder = builder.add_source(::config::File::from(path).required(false));
    builder = builder.add_source(
        ::config::Environment::with_prefix("ALERTING")
            .separator("__")
            .try_parsing(true),
    );

    builder
        .build()
        .map_err(|err| ConfigError::Other(err.to_string()))?
        .try_deserialize()
        .map_err(|err| ConfigError::Parse(err.to_string()))
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub(super) struct RawConfig {
    #[serde(default)]
    pub(super) zabbix: RawZabbix,
    #[serde(default)]
    pub(super) notify: RawNotify,
    #[serde(default)]
    pub(super) app: RawApp,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub(super) struct RawZabbix {
    pub(super) url: Option<String>,
    pub(super) token: Option<String>,
    #[serde(default = "default_limit")]
    pub(super) limit: u32,
    #[serde(default = "default_concurrency")]
    pub(super) concurrency: usize,
    #[serde(default)]
    pub(super) ack_filter: Option<String>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub(super) struct RawNotify {
    #[serde(default = "default_notify_appname")]
    pub(super) appname: String,
    #[serde(default)]
    pub(super) sticky: bool,
    #[serde(default)]
    #[serde_as(as = "Option<HumantimeDuration>")]
    pub(super) timeout: Option<Duration>,
    #[serde(default)]
    pub(super) default_timeout: bool,
    #[serde(default)]
    pub(super) icon: Option<PathBuf>,
    #[serde(default = "default_open_label")]
    pub(super) open_label: String,
    #[serde(default)]
    pub(super) notify_acked: bool,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub(super) struct RawApp {
    #[serde(default = "default_max_notif")]
    pub(super) max_notif: usize,
    #[serde(default = "default_queue_bound")]
    pub(super) queue_bound: usize,
    #[serde(default = "default_dedup_cache_size")]
    pub(super) dedup_cache_size: usize,
    #[serde(default = "default_rate_limit_max")]
    pub(super) rate_limit_max: usize,
    #[serde(default = "default_rate_limit_window")]
    #[serde_as(as = "HumantimeDuration")]
    pub(super) rate_limit_window: Duration,
    #[serde(default = "default_poll_interval")]
    #[serde_as(as = "HumantimeDuration")]
    pub(super) poll_interval: Duration,
    #[serde(default)]
    pub(super) open_url_fmt: Option<String>,
}

impl RawConfig {
    pub(super) fn apply_env_overrides(&mut self) -> std::result::Result<(), ConfigError> {
        if let Some(url) = env_string("ZBX_URL")? {
            self.zabbix.url = Some(url);
        }
        if let Some(token) = env_string("ZBX_TOKEN")? {
            self.zabbix.token = Some(token);
        }
        if let Some(limit) = env_parse::<u32>("LIMIT")? {
            self.zabbix.limit = limit;
        }
        if let Some(concurrency) = env_parse::<usize>("CONCURRENCY")? {
            self.zabbix.concurrency = concurrency;
        }
        if let Some(filter) = env_string("ACK_FILTER")? {
            self.zabbix.ack_filter = Some(filter);
        }
        if let Some(max_notif) = env_parse::<usize>("MAX_NOTIF")? {
            self.app.max_notif = max_notif;
        }
        if let Some(queue) = env_parse::<usize>("NOTIFY_QUEUE_BOUND")? {
            self.app.queue_bound = queue;
        }
        if let Some(dedup) = env_parse::<usize>("DEDUPE_CACHE_SIZE")? {
            self.app.dedup_cache_size = dedup;
        }
        if let Some(rate_max) = env_parse::<usize>("RATE_LIMIT_MAX")? {
            self.app.rate_limit_max = rate_max;
        }
        if let Some(rate_window) = env_duration("RATE_LIMIT_WINDOW")? {
            self.app.rate_limit_window = rate_window;
        }
        if let Some(interval) = env_duration("POLL_INTERVAL")? {
            self.app.poll_interval = interval;
        }
        if let Some(fmt) = env_string("ZBX_OPEN_URL_FMT")? {
            self.app.open_url_fmt = Some(fmt);
        }
        if let Some(appname) = env_string("NOTIFY_APPNAME")? {
            self.notify.appname = appname;
        }
        if let Some(sticky) = env_bool("NOTIFY_STICKY")? {
            self.notify.sticky = sticky;
        }
        if let Some(timeout) = env_duration("NOTIFY_TIMEOUT")? {
            self.notify.timeout = Some(timeout);
        }
        if let Some(default_timeout) = env_bool("NOTIFY_TIMEOUT_DEFAULT")? {
            self.notify.default_timeout = default_timeout;
        }
        if let Some(icon) = env_string("NOTIFY_ICON")? {
            self.notify.icon = Some(PathBuf::from(icon));
        }
        if let Some(open_label) = env_string("NOTIFY_OPEN_LABEL")? {
            self.notify.open_label = open_label;
        }
        if let Some(notify_acked) = env_bool("NOTIFY_ACKED")? {
            self.notify.notify_acked = notify_acked;
        }
        Ok(())
    }

    pub(super) fn validate_and_build(self) -> Result<Config> {
        let url_str = self.zabbix.url.ok_or(ConfigError::MissingField {
            field: "zabbix.url",
        })?;
        let token = self.zabbix.token.ok_or(ConfigError::MissingField {
            field: "zabbix.token",
        })?;
        if token.trim().is_empty() {
            return Err(ConfigError::InvalidField {
                field: "zabbix.token",
                message: "token cannot be empty".to_string(),
            }
            .into());
        }
        let base_url = Url::parse(&url_str).map_err(|err| ConfigError::InvalidField {
            field: "zabbix.url",
            message: err.to_string(),
        })?;

        let ack_src = self.zabbix.ack_filter.unwrap_or_else(default_ack_filter);
        let ack_filter = AckFilter::from_str(&ack_src.to_ascii_lowercase()).map_err(|err| {
            ConfigError::InvalidField {
                field: "zabbix.ack_filter",
                message: err,
            }
        })?;

        if !MAX_NOTIF_BOUNDS.contains(&self.app.max_notif) {
            return Err(ConfigError::InvalidField {
                field: "app.max_notif",
                message: format!(
                    "expected between {} and {}, got {}",
                    MAX_NOTIF_BOUNDS.start(),
                    MAX_NOTIF_BOUNDS.end(),
                    self.app.max_notif
                ),
            }
            .into());
        }
        if self.app.queue_bound == 0 {
            return Err(ConfigError::InvalidField {
                field: "app.queue_bound",
                message: "queue bound must be greater than zero".to_string(),
            }
            .into());
        }
        if self.app.dedup_cache_size == 0 {
            return Err(ConfigError::InvalidField {
                field: "app.dedup_cache_size",
                message: "dedup cache size must be greater than zero".to_string(),
            }
            .into());
        }
        if self.app.rate_limit_max == 0 {
            return Err(ConfigError::InvalidField {
                field: "app.rate_limit_max",
                message: "rate limit must allow at least one event".to_string(),
            }
            .into());
        }
        if self.app.rate_limit_window.is_zero() {
            return Err(ConfigError::InvalidField {
                field: "app.rate_limit_window",
                message: "window duration must be greater than zero".to_string(),
            }
            .into());
        }
        if self.app.poll_interval.is_zero() {
            return Err(ConfigError::InvalidField {
                field: "app.poll_interval",
                message: "poll interval must be greater than zero".to_string(),
            }
            .into());
        }

        Ok(Config {
            base_url,
            token: token.into(),
            limit: self.zabbix.limit,
            concurrency: self.zabbix.concurrency.max(1),
            ack_filter,
            max_notif: self.app.max_notif,
            queue_capacity: self.app.queue_bound,
            dedup_cache_size: self.app.dedup_cache_size,
            rate_limit: RateLimit {
                max_events: self.app.rate_limit_max,
                per: self.app.rate_limit_window,
            },
            poll_interval: self.app.poll_interval,
            open_url_fmt: self.app.open_url_fmt,
            notify: NotifySettings {
                appname: self.notify.appname,
                sticky: self.notify.sticky,
                timeout: self.notify.timeout,
                default_timeout: self.notify.default_timeout,
                icon: self.notify.icon,
                open_label: self.notify.open_label,
                notify_acked: self.notify.notify_acked,
            },
            http_connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            http_request_timeout: DEFAULT_HTTP_TIMEOUT,
        })
    }
}

impl Default for RawZabbix {
    fn default() -> Self {
        Self {
            url: None,
            token: None,
            limit: default_limit(),
            concurrency: default_concurrency(),
            ack_filter: Some(default_ack_filter()),
        }
    }
}

impl Default for RawNotify {
    fn default() -> Self {
        Self {
            appname: default_notify_appname(),
            sticky: false,
            timeout: None,
            default_timeout: false,
            icon: None,
            open_label: default_open_label(),
            notify_acked: false,
        }
    }
}

impl Default for RawApp {
    fn default() -> Self {
        Self {
            max_notif: default_max_notif(),
            queue_bound: default_queue_bound(),
            dedup_cache_size: default_dedup_cache_size(),
            rate_limit_max: default_rate_limit_max(),
            rate_limit_window: default_rate_limit_window(),
            poll_interval: default_poll_interval(),
            open_url_fmt: None,
        }
    }
}
