use anyhow::{Context, Result};
use serde::Deserialize;
use std::{fs, path::Path};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FileConfig {
    pub url: Option<String>,
    pub token: Option<String>,
    pub limit: Option<u32>,
    pub concurrency: Option<usize>,
    pub max_notif: Option<usize>,
}

impl FileConfig {
    /// Charge le fichier TOML si présent, sinon Ok(None)
    pub fn load_from(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let txt = fs::read_to_string(path)
            .with_context(|| format!("Lecture fichier config: {}", path.display()))?;
        let cfg: FileConfig = toml::from_str(&txt)
            .with_context(|| format!("Parse TOML: {}", path.display()))?;
        Ok(Some(cfg))
    }
}