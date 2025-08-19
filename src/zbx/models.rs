use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct ZbxError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ZbxEnvelope<T> {
    pub jsonrpc: String,
    // IMPORTANT: pas de #[serde(default)] sur Option<T>
    pub result: Option<T>,
    pub error: Option<ZbxError>,
    pub id: Value,
}

#[derive(Debug, Deserialize)]
pub struct Problem {
    pub eventid: String,
    pub clock: String,    // epoch sec as string
    pub severity: String, // numeric string 0..5
    pub name: String,
    #[serde(default)]
    pub objectid: Option<String>,
}

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
}

#[derive(Serialize)]
pub struct RpcRequest<'a> {
    pub jsonrpc: &'static str,
    pub method: &'a str,
    pub params: Value,
    pub id: u32,
    pub auth: &'a str,
}
