use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::time::Duration;

use secrecy::SecretString;
use url::Url;

use crate::Result;
use crate::error::Error as AlertError;
use crate::types::AckFilter;

mod defaults;
mod env;
mod raw;
mod serde;

pub(crate) use serde::HumantimeDuration;

const MAX_NOTIF_BOUNDS: RangeInclusive<usize> = 1..=100;
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
    /// Load configuration from a file and the environment.
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration file cannot be read, parsed,
    /// when environment overrides are invalid, or when the resulting values
    /// fail validation.
    pub fn from_env_and_file(path: impl AsRef<Path>) -> Result<Self> {
        let mut raw = raw::load(path).map_err(AlertError::from)?;
        raw.apply_env_overrides().map_err(AlertError::from)?;
        raw.validate_and_build()
    }
}

impl RateLimit {
    #[must_use]
    pub const fn allows(&self, count: usize, candidate: usize) -> bool {
        candidate == 0 || count < self.max_events
    }
}

#[cfg(test)]
mod tests {
    use super::RateLimit;
    use std::time::Duration;

    #[test]
    fn allows_first_candidate_even_when_full() {
        let bucket = RateLimit {
            max_events: 1,
            per: Duration::from_secs(1),
        };
        assert!(bucket.allows(0, 0));
        assert!(!bucket.allows(1, 1));
    }
}
