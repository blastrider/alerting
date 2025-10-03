use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Erreur Zabbix enveloppée par JSON-RPC.
#[derive(Debug, Deserialize)]
pub struct ZbxError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<String>,
}

/// Enveloppe JSON-RPC générique.
#[derive(Debug, Deserialize)]
pub struct ZbxEnvelope<T> {
    #[allow(dead_code)]
    pub jsonrpc: String,
    pub result: Option<T>, // ne pas mettre #[serde(default)] ici
    pub error: Option<ZbxError>,
    #[allow(dead_code)]
    pub id: Value,
}

/// Problème Zabbix tel que renvoyé par `problem.get`.
#[derive(Debug, Deserialize)]
pub struct Problem {
    pub eventid: String,
    #[serde(deserialize_with = "de_i64_from_str")]
    pub clock: i64, // epoch sec en String -> i64
    #[serde(deserialize_with = "de_u8_from_str")]
    pub severity: u8, // "0".."5" -> u8
    pub name: String,
    #[serde(rename = "objectid")]
    #[allow(dead_code)]
    pub _objectid: Option<String>, // non utilisé dans l'affichage
    #[serde(default, deserialize_with = "de_bool_from_str_or_int")]
    pub acknowledged: bool, // "0"/"1" ou 0/1 -> bool
}

/// `event.get` avec hôtes.
#[derive(Debug, Deserialize)]
pub struct EventWithHosts {
    #[serde(default)]
    pub hosts: Vec<Host>,
}

#[derive(Debug, Deserialize)]
pub struct Host {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    // Zabbix peut renvoyer "0"/"1" (string) ou 0/1 (int) selon versions/widgets.
    #[serde(default, deserialize_with = "de_opt_u8_from_str_or_int")]
    pub status: Option<u8>, // 0 = enabled, 1 = disabled
}

/// Métadonnées d’hôte utilisées par le binaire (nom affichable + statut).
#[derive(Debug, Clone)]
pub struct HostMeta {
    pub display_name: String,
    pub status: Option<u8>,
}

/// Payload interne (privé au client) pour JSON-RPC.
#[derive(Serialize)]
pub(crate) struct RpcRequest<'a> {
    pub jsonrpc: &'static str,
    pub method: &'a str,
    pub params: Value,
    pub id: u32,
    pub auth: &'a str,
}

// ----------- helpers de désérialisation -----------

fn de_i64_from_str<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse::<i64>()
        .map_err(|e| de::Error::custom(format!("clock invalide '{s}': {e}")))
}

fn de_u8_from_str<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    s.parse::<u8>()
        .map_err(|e| de::Error::custom(format!("severity invalide '{s}': {e}")))
}

// --- helper: Option<u8> <- string/int/null ---
fn de_opt_u8_from_str_or_int<'de, D>(de: D) -> Result<Option<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum U8OrStrOrNull {
        U8(u8),
        Str(String),
        Null,
    }
    match U8OrStrOrNull::deserialize(de)? {
        U8OrStrOrNull::U8(v) => Ok(Some(v)),
        U8OrStrOrNull::Str(s) => s.parse::<u8>().map(Some).map_err(de::Error::custom),
        U8OrStrOrNull::Null => Ok(None),
    }
}

fn de_bool_from_str_or_int<'de, D>(de: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Boolish {
        B(bool),
        U(u8),
        S(String),
        Null,
    }
    Ok(match Boolish::deserialize(de)? {
        Boolish::B(b) => b,
        Boolish::U(n) => n != 0,
        Boolish::S(s) => match s.as_str() {
            "1" | "true" | "True" | "TRUE" => true,
            _ => false,
        },
        Boolish::Null => false,
    })
}
