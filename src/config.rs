use std::path::{Path, PathBuf};
use std::time::Duration;

use humantime::{format_duration, parse_duration};
use secrecy::SecretString;
use serde::Deserialize;
use serde_with::{DeserializeAs, SerializeAs, serde_as};
use std::str::FromStr;
use url::Url;

use crate::Result;
use crate::error::ConfigError;
use crate::types::AckFilter;

const MAX_NOTIF_BOUNDS: std::ops::RangeInclusive<usize> = 1..=100;
const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct Config {
    pub base_url: Url,
    pub token: SecretString,
    pub limit: u32,
    pub concurrency: usize,
    pub ack_filter: AckFilter,
    pub max_notif: usize,
    pub queue_capacity: usize,
    pub dedup_cache_size: usize,
    pub rate_limit: RateLimit,
    pub poll_interval: Duration,
    pub open_url_fmt: Option<String>,
    pub notify: NotifySettings,
    pub http_connect_timeout: Duration,
    pub http_request_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct NotifySettings {
    pub appname: String,
    pub sticky: bool,
    pub timeout: Option<Duration>,
    pub default_timeout: bool,
    pub icon: Option<PathBuf>,
    pub open_label: String,
    pub notify_acked: bool,
}

#[derive(Debug, Clone)]
pub struct RateLimit {
    pub max_events: usize,
    pub per: Duration,
}

impl Config {
    pub fn from_env_and_file(path: impl AsRef<Path>) -> Result<Self> {
        let mut builder = ::config::Config::builder();
        let path = path.as_ref();
        builder = builder.add_source(::config::File::from(path).required(false));
        builder = builder.add_source(
            ::config::Environment::with_prefix("ALERTING")
                .separator("__")
                .try_parsing(true),
        );

        let mut raw: RawConfig = builder
            .build()
            .map_err(|err| ConfigError::Other(err.to_string()))?
            .try_deserialize()
            .map_err(|err| ConfigError::Parse(err.to_string()))?;

        raw.apply_env_overrides()?;
        raw.validate_and_build()
    }
}

impl RateLimit {
    pub fn allows(&self, count: usize, candidate: usize) -> bool {
        count + 1 <= self.max_events || candidate == 0
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
struct RawConfig {
    #[serde(default)]
    zabbix: RawZabbix,
    #[serde(default)]
    notify: RawNotify,
    #[serde(default)]
    app: RawApp,
}

#[serde_as]
#[derive(Debug, Deserialize)]
struct RawZabbix {
    url: Option<String>,
    token: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default = "default_concurrency")]
    concurrency: usize,
    #[serde(default)]
    ack_filter: Option<String>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
struct RawNotify {
    #[serde(default = "default_notify_appname")]
    appname: String,
    #[serde(default)]
    sticky: bool,
    #[serde(default)]
    #[serde_as(as = "Option<HumantimeDuration>")]
    timeout: Option<Duration>,
    #[serde(default)]
    default_timeout: bool,
    #[serde(default)]
    icon: Option<PathBuf>,
    #[serde(default = "default_open_label")]
    open_label: String,
    #[serde(default)]
    notify_acked: bool,
}

#[serde_as]
#[derive(Debug, Deserialize)]
struct RawApp {
    #[serde(default = "default_max_notif")]
    max_notif: usize,
    #[serde(default = "default_queue_bound")]
    queue_bound: usize,
    #[serde(default = "default_dedup_cache_size")]
    dedup_cache_size: usize,
    #[serde(default = "default_rate_limit_max")]
    rate_limit_max: usize,
    #[serde(default = "default_rate_limit_window")]
    #[serde_as(as = "HumantimeDuration")]
    rate_limit_window: Duration,
    #[serde(default = "default_poll_interval")]
    #[serde_as(as = "HumantimeDuration")]
    poll_interval: Duration,
    #[serde(default)]
    open_url_fmt: Option<String>,
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

impl RawConfig {
    fn apply_env_overrides(&mut self) -> std::result::Result<(), ConfigError> {
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

    fn validate_and_build(self) -> Result<Config> {
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

struct HumantimeDuration;

impl<'de> DeserializeAs<'de, Duration> for HumantimeDuration {
    fn deserialize_as<D>(deserializer: D) -> std::result::Result<Duration, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        parse_duration(&raw).map_err(serde::de::Error::custom)
    }
}

impl SerializeAs<Duration> for HumantimeDuration {
    fn serialize_as<S>(value: &Duration, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format_duration(*value).to_string())
    }
}

fn env_string(key: &'static str) -> std::result::Result<Option<String>, ConfigError> {
    match std::env::var(key) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(ConfigError::Other(err.to_string())),
    }
}

fn env_parse<T>(key: &'static str) -> std::result::Result<Option<T>, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    if let Some(value) = env_string(key)? {
        if value.trim().is_empty() {
            return Ok(None);
        }
        return value
            .trim()
            .parse::<T>()
            .map(Some)
            .map_err(|err| ConfigError::InvalidField {
                field: key,
                message: err.to_string(),
            });
    }
    Ok(None)
}

fn env_bool(key: &'static str) -> std::result::Result<Option<bool>, ConfigError> {
    env_parse::<bool>(key)
}

fn env_duration(key: &'static str) -> std::result::Result<Option<Duration>, ConfigError> {
    if let Some(value) = env_string(key)? {
        if value.trim().is_empty() {
            return Ok(None);
        }
        return parse_duration(value.trim())
            .map(Some)
            .map_err(|err| ConfigError::InvalidField {
                field: key,
                message: err.to_string(),
            });
    }
    Ok(None)
}

const fn default_limit() -> u32 {
    20
}

const fn default_concurrency() -> usize {
    4
}

fn default_ack_filter() -> String {
    "unacked".to_string()
}

fn default_notify_appname() -> String {
    "Alerting".to_string()
}

fn default_open_label() -> String {
    "Open".to_string()
}

const fn default_max_notif() -> usize {
    5
}

const fn default_queue_bound() -> usize {
    64
}

const fn default_dedup_cache_size() -> usize {
    256
}

const fn default_rate_limit_max() -> usize {
    3
}

const fn default_rate_limit_window() -> Duration {
    Duration::from_secs(5)
}

const fn default_poll_interval() -> Duration {
    Duration::from_secs(30)
}

#[cfg(test)]
mod tests {
    use super::HumantimeDuration;
    use serde::Deserialize;
    use serde_with::serde_as;
    use std::time::Duration;

    #[test]
    fn humantime_duration_parses_strings() {
        #[serde_as]
        #[derive(Deserialize)]
        struct Sample {
            #[serde_as(as = "Option<HumantimeDuration>")]
            duration: Option<Duration>,
        }

        let sample: Sample = serde_json::from_str(r#"{"duration":"5s"}"#).unwrap();
        assert_eq!(sample.duration, Some(Duration::from_secs(5)));
    }
}
