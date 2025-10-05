use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Zabbix(#[from] ZbxError),
    #[error(transparent)]
    Notify(#[from] NotifyError),
    #[error("telemetry initialization failed: {0}")]
    Telemetry(String),
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read configuration file {path}")]
    FileRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse configuration: {0}")]
    Parse(String),
    #[error("missing required configuration field: {field}")]
    MissingField { field: &'static str },
    #[error("invalid configuration for {field}: {message}")]
    InvalidField {
        field: &'static str,
        message: String,
    },
    #[error("configuration error: {0}")]
    Other(String),
}

#[derive(Debug, Error)]
pub enum ZbxError {
    #[error("failed to build HTTP client")]
    Client {
        #[source]
        source: reqwest::Error,
    },
    #[error("request failed: {source}")]
    Request {
        #[source]
        source: reqwest::Error,
    },
    #[error("unexpected HTTP status: {status}")]
    HttpStatus { status: reqwest::StatusCode },
    #[error("invalid JSON payload: {message}")]
    Json { message: String },
    #[error("invalid field {field}: {message}")]
    InvalidField {
        field: &'static str,
        message: String,
    },
    #[error("Zabbix API error {code}: {message}")]
    Api { code: i64, message: String },
    #[error("missing field in API response: {field}")]
    MissingField { field: &'static str },
    #[error("retry budget exhausted")]
    RetryExhausted {
        #[source]
        source: Box<ZbxError>,
    },
}

#[derive(Debug, Error)]
pub enum NotifyError {
    #[error("notification backend failed")]
    Backend,
    #[error("invalid notification payload: {0}")]
    InvalidPayload(String),
}

impl From<reqwest::Error> for ZbxError {
    fn from(source: reqwest::Error) -> Self {
        if source.is_status() {
            if let Some(status) = source.status() {
                return Self::HttpStatus { status };
            }
        }
        Self::Request { source }
    }
}

impl Error {
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            Self::Zabbix(ZbxError::Request { .. })
                | Self::Zabbix(ZbxError::HttpStatus { .. })
                | Self::Zabbix(ZbxError::Json { .. })
        )
    }
}
