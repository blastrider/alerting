#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod config;
pub mod error;
pub mod telemetry;
pub mod types;
pub mod zbx_client;

pub type Result<T> = std::result::Result<T, error::Error>;
