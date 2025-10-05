use std::sync::Arc;
use std::time::{Duration, Instant};

use backoff::ExponentialBackoffBuilder;
use backoff::backoff::Backoff;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::{debug, warn};
use url::Url;
use uuid::Uuid;

use crate::Result;
use crate::error::{Error, ZbxError};
use crate::types::{AckFilter, Severity};

const MAX_ATTEMPTS: usize = 3;
const BODY_PREVIEW_LIMIT: usize = 256;
const CORRELATION_HEADER: &str = "x-correlation-id";

#[derive(Clone)]
pub struct ZbxClient {
    http: reqwest::Client,
    base: Url,
    token: SecretString,
    timeout: Duration,
}

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

impl ZbxClient {
    pub fn new(
        base: Url,
        token: SecretString,
        timeout: Duration,
        connect_timeout: Duration,
        insecure_http: bool,
    ) -> Result<Self> {
        if base.scheme() != "https" && !insecure_http {
            return Err(Error::Config(crate::error::ConfigError::InvalidField {
                field: "zabbix.url",
                message: "only https URLs are accepted without --insecure".to_string(),
            }));
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json-rpc"),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("application/json"),
        );

        let mut builder = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(connect_timeout)
            .timeout(timeout)
            .user_agent(concat!("alerting/", env!("CARGO_PKG_VERSION")))
            .pool_idle_timeout(Duration::from_secs(30));

        if !insecure_http {
            builder = builder.https_only(true);
        }

        let http = builder
            .build()
            .map_err(|err| ZbxError::Client { source: err })?;

        Ok(Self {
            http,
            base,
            token,
            timeout,
        })
    }

    async fn call<T>(&self, method: &str, params: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let mut backoff = ExponentialBackoffBuilder::new()
            .with_initial_interval(Duration::from_millis(200))
            .with_multiplier(2.0)
            .with_randomization_factor(0.25)
            .with_max_interval(Duration::from_secs(2))
            .with_max_elapsed_time(Some(self.timeout))
            .build();

        let params = params;
        for attempt in 1..=MAX_ATTEMPTS {
            let correlation_id = Uuid::now_v7().to_string();
            let started = Instant::now();
            let payload = RpcRequest {
                jsonrpc: "2.0",
                method,
                params: params.clone(),
                id: attempt as u64,
                auth: self.token.expose_secret(),
            };
            let request = self
                .http
                .post(self.base.clone())
                .header(CORRELATION_HEADER, &correlation_id)
                .json(&payload);

            let response = match request.send().await {
                Ok(resp) => resp,
                Err(err) => {
                    let zerr = ZbxError::from(err);
                    if attempt == MAX_ATTEMPTS {
                        return Err(ZbxError::RetryExhausted {
                            source: Box::new(zerr),
                        }
                        .into());
                    }
                    if let Some(delay) = backoff.next_backoff() {
                        warn!(
                            method,
                            %correlation_id,
                            attempt,
                            delay_ms = delay.as_millis() as u64,
                            error = %zerr,
                            "retrying after transport error"
                        );
                        sleep(delay).await;
                        continue;
                    }
                    return Err(zerr.into());
                }
            };

            let status = response.status();
            if status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT {
                let zerr = ZbxError::HttpStatus { status };
                if attempt == MAX_ATTEMPTS {
                    return Err(ZbxError::RetryExhausted {
                        source: Box::new(zerr),
                    }
                    .into());
                }
                if let Some(delay) = backoff.next_backoff() {
                    warn!(
                        method,
                        %correlation_id,
                        attempt,
                        delay_ms = delay.as_millis() as u64,
                        status = %status,
                        "retrying after server error"
                    );
                    sleep(delay).await;
                    continue;
                }
                return Err(zerr.into());
            }

            if !status.is_success() {
                return Err(ZbxError::HttpStatus { status }.into());
            }

            let body = match response.bytes().await {
                Ok(body) => body,
                Err(err) => {
                    let zerr = ZbxError::from(err);
                    if attempt == MAX_ATTEMPTS {
                        return Err(ZbxError::RetryExhausted {
                            source: Box::new(zerr),
                        }
                        .into());
                    }
                    if let Some(delay) = backoff.next_backoff() {
                        warn!(
                            method,
                            %correlation_id,
                            attempt,
                            delay_ms = delay.as_millis() as u64,
                            error = %zerr,
                            "retrying after body read error"
                        );
                        sleep(delay).await;
                        continue;
                    }
                    return Err(zerr.into());
                }
            };

            let envelope: RpcEnvelope<T> = match serde_json::from_slice(&body) {
                Ok(env) => env,
                Err(err) => {
                    let preview = body_preview(&body);
                    let zerr = ZbxError::Json {
                        message: format!(
                            "error decoding response body: {err}; body preview: {preview}"
                        ),
                    };
                    if attempt == MAX_ATTEMPTS {
                        return Err(ZbxError::RetryExhausted {
                            source: Box::new(zerr),
                        }
                        .into());
                    }
                    if let Some(delay) = backoff.next_backoff() {
                        warn!(
                            method,
                            %correlation_id,
                            attempt,
                            delay_ms = delay.as_millis() as u64,
                            error = %zerr,
                            "retrying after JSON decode error"
                        );
                        sleep(delay).await;
                        continue;
                    }
                    return Err(zerr.into());
                }
            };

            if let Some(err) = envelope.error {
                let mut message = err.message;
                if let Some(data) = err.data {
                    message.push_str(&format!(" – {data}"));
                }
                return Err(ZbxError::Api {
                    code: err.code,
                    message,
                }
                .into());
            }

            if let Some(result) = envelope.result {
                debug!(
                    method,
                    %correlation_id,
                    attempt,
                    latency_ms = started.elapsed().as_millis() as u64,
                    "zabbix call succeeded"
                );
                return Ok(result);
            }

            return Err(ZbxError::MissingField { field: "result" }.into());
        }
        unreachable!("retry loop should have returned before reaching this point")
    }

    pub async fn active_problems(&self, limit: u32, ack: AckFilter) -> Result<Vec<Problem>> {
        let mut params = json!({
            "output": ["eventid","name","severity","clock","lastchange","acknowledged"],
            "recent": false,
            "limit": limit,
            "sortfield": ["eventid"],
            "sortorder": "DESC"
        });
        match ack {
            AckFilter::Acked => params["acknowledged"] = json!(true),
            AckFilter::Unacked => params["acknowledged"] = json!(false),
            AckFilter::All => {}
        }

        let raw: Vec<RawProblem> = self.call("problem.get", params).await?;
        let problems = raw
            .into_iter()
            .map(Problem::try_from)
            .collect::<std::result::Result<Vec<_>, Error>>()?;
        Ok(problems)
    }

    pub async fn ack_event(&self, eventid: &str, message: Option<String>) -> Result<()> {
        self.event_update(eventid, true, message).await
    }

    pub async fn unack_event(&self, eventid: &str, message: Option<String>) -> Result<()> {
        self.event_update(eventid, false, message).await
    }

    async fn event_update(&self, eventid: &str, ack: bool, message: Option<String>) -> Result<()> {
        let mut params = json!({
            "eventids": [eventid],
        });
        let action = if ack { 2 } else { 16 };
        params["action"] = json!(action);
        if let Some(msg) = message.as_deref() {
            if !msg.is_empty() {
                params["message"] = json!(msg);
                params["action"] = json!(action + 4);
            }
        }
        let _: serde_json::Value = self.call("event.acknowledge", params).await?;
        Ok(())
    }

    pub async fn resolve_hosts(
        &self,
        event_ids: &[String],
        concurrency: usize,
    ) -> Result<Vec<Option<HostMeta>>> {
        let concurrency = concurrency.max(1);
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let mut tasks: JoinSet<(usize, Result<Option<HostMeta>>)> = JoinSet::new();

        for (idx, event_id) in event_ids.iter().cloned().enumerate() {
            let client = self.clone();
            let semaphore = Arc::clone(&semaphore);
            tasks.spawn(async move {
                let permit = semaphore.acquire_owned().await;
                let _permit = match permit {
                    Ok(p) => p,
                    Err(_) => return (idx, Ok(None)),
                };
                let res = client.host_meta_for_event(&event_id).await;
                (idx, res)
            });
        }

        let mut out: Vec<Option<HostMeta>> = vec![None; event_ids.len()];
        while let Some(res) = tasks.join_next().await {
            match res {
                Ok((idx, host)) => {
                    out[idx] = host?;
                }
                Err(join_err) => {
                    warn!(error = %join_err, "host resolution task failed");
                }
            }
        }
        Ok(out)
    }

    async fn host_meta_for_event(&self, eventid: &str) -> Result<Option<HostMeta>> {
        let params = json!({
            "selectHosts": ["host", "name", "status"],
            "eventids": [eventid],
        });
        let raw: Vec<EventWithHosts> = self.call("event.get", params).await?;
        Ok(raw
            .into_iter()
            .flat_map(|evt| evt.hosts.into_iter())
            .next()
            .map(HostMeta::from))
    }
}

fn body_preview(body: &[u8]) -> String {
    if body.is_empty() {
        return "<empty>".to_string();
    }
    let end = body.len().min(BODY_PREVIEW_LIMIT);
    let mut preview = String::from_utf8_lossy(&body[..end]).to_string();
    if body.len() > BODY_PREVIEW_LIMIT {
        preview.push_str("...");
    }
    preview.replace('\n', "\\n")
}

#[derive(Debug, Deserialize)]
struct RawProblem {
    eventid: String,
    #[serde(deserialize_with = "deserialize_i64")]
    clock: i64,
    #[serde(
        default,
        rename = "lastchange",
        deserialize_with = "deserialize_opt_i64"
    )]
    last_change: Option<i64>,
    #[serde(deserialize_with = "deserialize_u8")]
    severity: u8,
    name: String,
    #[serde(default, deserialize_with = "deserialize_bool")]
    acknowledged: bool,
}

impl TryFrom<RawProblem> for Problem {
    type Error = Error;

    fn try_from(value: RawProblem) -> Result<Self> {
        let severity = Severity::from_zabbix(value.severity as i64).ok_or_else(|| {
            Error::Zabbix(ZbxError::InvalidField {
                field: "severity",
                message: format!("unexpected severity code {}", value.severity),
            })
        })?;
        Ok(Self {
            event_id: value.eventid,
            clock: value.clock,
            last_change: value.last_change.unwrap_or(value.clock),
            name: value.name,
            severity,
            acknowledged: value.acknowledged,
        })
    }
}

#[derive(Debug, Deserialize)]
struct EventWithHosts {
    #[serde(default)]
    hosts: Vec<HostRow>,
}

#[derive(Debug, Deserialize)]
struct HostRow {
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

#[derive(Debug, Deserialize)]
struct RpcEnvelope<T> {
    #[allow(dead_code)]
    jsonrpc: String,
    result: Option<T>,
    error: Option<RpcError>,
    #[allow(dead_code)]
    id: Value,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<String>,
}

#[derive(Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'static str,
    method: &'a str,
    params: Value,
    id: u64,
    auth: &'a str,
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
