use serde::Deserialize;

use crate::error::{Error, ZbxError};
use crate::types::Severity;

#[derive(Debug, Clone)]
pub struct Problem {
    pub event_id: String,
    pub clock: i64,
    pub last_change: i64,
    pub name: String,
    pub severity: Severity,
    pub acknowledged: bool,
}

#[derive(Debug, Clone)]
pub struct HostMeta {
    pub host: Option<String>,
    pub display_name: String,
    pub status: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawProblem {
    #[serde(rename = "eventid")]
    pub(crate) event_id: String,
    #[serde(deserialize_with = "deserialize_i64")]
    pub(crate) clock: i64,
    #[serde(
        default,
        rename = "lastchange",
        deserialize_with = "deserialize_opt_i64"
    )]
    pub(crate) last_change: Option<i64>,
    #[serde(deserialize_with = "deserialize_u8")]
    pub(crate) severity: u8,
    pub(crate) name: String,
    #[serde(default, deserialize_with = "deserialize_bool")]
    pub(crate) acknowledged: bool,
}

impl TryFrom<RawProblem> for Problem {
    type Error = Error;

    fn try_from(value: RawProblem) -> std::result::Result<Self, Error> {
        let severity = Severity::from_zabbix(value.severity as i64).ok_or_else(|| {
            Error::Zabbix(ZbxError::InvalidField {
                field: "severity",
                message: format!("unexpected severity code {}", value.severity),
            })
        })?;
        Ok(Self {
            event_id: value.event_id,
            clock: value.clock,
            last_change: value.last_change.unwrap_or(value.clock),
            name: value.name,
            severity,
            acknowledged: value.acknowledged,
        })
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct EventWithHosts {
    #[serde(default)]
    pub(crate) hosts: Vec<HostRow>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HostRow {
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_u8")]
    status: Option<u8>,
}

impl From<HostRow> for HostMeta {
    fn from(value: HostRow) -> Self {
        let HostRow { host, name, status } = value;
        let display_name = name
            .clone()
            .or_else(|| host.clone())
            .unwrap_or_else(|| "<unknown host>".to_string());
        Self {
            host,
            display_name,
            status,
        }
    }
}

fn deserialize_i64<'de, D>(de: D) -> std::result::Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(de)?;
    s.parse::<i64>().map_err(serde::de::Error::custom)
}

fn deserialize_u8<'de, D>(de: D) -> std::result::Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(de)?;
    s.parse::<u8>().map_err(serde::de::Error::custom)
}

fn deserialize_opt_i64<'de, D>(de: D) -> std::result::Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum MaybeI64 {
        Int(i64),
        Str(String),
        Null,
    }

    match MaybeI64::deserialize(de)? {
        MaybeI64::Int(value) => Ok(Some(value)),
        MaybeI64::Str(value) => value
            .parse::<i64>()
            .map(Some)
            .map_err(serde::de::Error::custom),
        MaybeI64::Null => Ok(None),
    }
}

fn deserialize_bool<'de, D>(de: D) -> std::result::Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Boolish {
        Bool(bool),
        Int(i64),
        Str(String),
        Null,
    }

    Ok(match Boolish::deserialize(de)? {
        Boolish::Bool(value) => value,
        Boolish::Int(value) => value != 0,
        Boolish::Str(value) => matches!(value.as_str(), "1" | "true" | "TRUE"),
        Boolish::Null => false,
    })
}

fn deserialize_opt_u8<'de, D>(de: D) -> std::result::Result<Option<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum MaybeU8 {
        Int(u8),
        Str(String),
        Null,
    }

    Ok(match MaybeU8::deserialize(de)? {
        MaybeU8::Int(value) => Some(value),
        MaybeU8::Str(value) => Some(value.parse::<u8>().map_err(serde::de::Error::custom)?),
        MaybeU8::Null => None,
    })
}
