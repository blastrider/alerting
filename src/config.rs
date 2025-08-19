use anyhow::{Context, Result};
use std::env;

/// Configuration simple lue depuis l'environnement.
/// LIMIT et CONCURRENCY sont optionnels.
#[derive(Debug, Clone)]
pub struct Config {
    pub url: String,
    pub token: String,
    pub limit: u32,
    pub concurrency: usize,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let url = env::var("ZBX_URL")
            .unwrap_or_else(|_| "https://zabbix.example.com/api_jsonrpc.php".to_string());
        let token = env::var("ZBX_TOKEN")
            .context("Veuillez définir ZBX_TOKEN dans l'environnement")?;
        let limit = env::var("LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
        let concurrency = env::var("CONCURRENCY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8);

        Ok(Self {
            url,
            token,
            limit,
            concurrency,
        })
    }
}
