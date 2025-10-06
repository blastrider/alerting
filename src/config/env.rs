use std::time::Duration;

use humantime::parse_duration;

use crate::error::ConfigError;

pub(super) fn env_string(key: &'static str) -> std::result::Result<Option<String>, ConfigError> {
    match std::env::var(key) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(err) => Err(ConfigError::Other(err.to_string())),
    }
}

pub(super) fn env_parse<T>(key: &'static str) -> std::result::Result<Option<T>, ConfigError>
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

pub(super) fn env_bool(key: &'static str) -> std::result::Result<Option<bool>, ConfigError> {
    env_parse::<bool>(key)
}

pub(super) fn env_duration(
    key: &'static str,
) -> std::result::Result<Option<Duration>, ConfigError> {
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
