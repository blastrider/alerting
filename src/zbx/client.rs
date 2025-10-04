use anyhow::{Context, Result, bail};
use reqwest::blocking as reqb;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::{sync::Semaphore, task::JoinSet};

use super::types::{EventWithHosts, HostMeta, Problem, RpcRequest, ZbxEnvelope};

/// Client Zabbix minimaliste et clonable.
#[derive(Clone)]
pub struct ZbxClient {
    http: reqwest::Client,
    url: String,
    token: String,
}
#[derive(Clone, Copy, Debug)]
pub enum AckFilter {
    Unack,
    Ack,
    All,
}

impl ZbxClient {
    pub fn new(url: &str, token: &str) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json-rpc"),
        );
        #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
        let mut builder = reqwest::Client::builder()
            .default_headers(headers)
            .http1_only() // ← force HTTP/1.1
            .user_agent(concat!("alerting/", env!("CARGO_PKG_VERSION")))
            .pool_idle_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(15))
            .danger_accept_invalid_certs(true);

        let http = builder.build().context("building HTTP client")?;
        Ok(Self {
            http,
            url: url.to_string(),
            token: token.to_string(),
        })
    }

    async fn call<T: DeserializeOwned>(&self, method: &str, params: Value, id: u32) -> Result<T> {
        let payload = RpcRequest {
            jsonrpc: "2.0",
            method,
            params,
            id,
            auth: &self.token,
        };
        let resp = self
            .http
            .post(&self.url)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("HTTP POST send to {} failed", self.url))?;

        let status = resp.status();
        let env: ZbxEnvelope<T> = resp
            .json()
            .await
            .with_context(|| format!("Decoding JSON response (HTTP {status})"))?;

        if let Some(err) = env.error {
            bail!(
                "Zabbix API error {}: {}{}",
                err.code,
                err.message,
                err.data.map(|d| format!(" – {d}")).unwrap_or_default()
            );
        }
        env.result
            .ok_or_else(|| anyhow::anyhow!("Zabbix API: missing result field"))
    }

    /// Résout les hôtes pour une liste d'eventids, avec limite de parallélisme.
    pub async fn resolve_hosts_concurrent(
        &self,
        eventids: &[String],
        max_concurrency: usize,
    ) -> Result<Vec<Option<HostMeta>>> {
        let sem = Arc::new(Semaphore::new(max_concurrency.max(1)));
        let mut joins = JoinSet::new();

        for (idx, eid) in eventids.iter().cloned().enumerate() {
            let sem = Arc::clone(&sem);
            let this = self.clone();

            joins.spawn(async move {
                // Si la semaphore est fermée, on renvoie None proprement.
                let permit = match sem.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => return (idx, None),
                };
                let _keep_alive = permit; // garde le slot jusqu'à la fin de la tâche
                let meta = match this.host_meta_for_event(&eid).await {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("(warn) event {}: échec host_meta_for_event: {e}", &eid);
                        None
                    }
                };
                (idx, meta)
            });
        }

        let mut out: Vec<Option<HostMeta>> = vec![None; eventids.len()];
        while let Some(res) = joins.join_next().await {
            let (idx, host) = res?; // propage JoinError en anyhow::Error
            out[idx] = host;
        }
        Ok(out)
    }

    /// Problèmes actifs (non résolus), avec filtre ACK.
    pub async fn active_problems(&self, limit: u32, ack: AckFilter) -> Result<Vec<Problem>> {
        let mut params = json!({
            "output": ["eventid","name","severity","clock","objectid","acknowledged"],
            "recent": false, // UNRESOLVED seulement (cf. doc)
            "limit": limit,
            "sortfield": ["eventid"],
            "sortorder": "DESC"
        });
        match ack {
            AckFilter::Unack => {
                params["acknowledged"] = json!(false);
            }
            AckFilter::Ack => {
                params["acknowledged"] = json!(true);
            }
            AckFilter::All => { /* ne rien ajouter */ }
        }
        self.call("problem.get", params, 1).await
    }

    /// Appel générique event.acknowledge (bitmask d'actions).
    async fn event_update(
        &self,
        eventids: &[&str],
        action: i32,
        message: Option<&str>,
    ) -> Result<Value> {
        // Convertit chaque eventid en entier si possible (sinon string)
        let ids_json: Vec<Value> = eventids
            .iter()
            .map(|e| {
                e.parse::<i64>()
                    .map(Value::from)
                    .unwrap_or_else(|_| Value::from(*e))
            })
            .collect();
        let mut params = json!({ "eventids": ids_json, "action": action });
        if let Some(msg) = message {
            if !msg.is_empty() {
                // si on met un message, ajouter aussi le bit 'add message' si absent
                // (4) : ack+msg => 6, unack+msg => 20, etc.  :contentReference[oaicite:3]{index=3}
                params["message"] = json!(msg);
            }
        }
        let res = self.call::<Value>("event.acknowledge", params, 777).await?;
        eprintln!("[zbx] event.acknowledge OK: {}", res);
        Ok(res)
    }

    /// Ack simple ou avec message (bitmask: 2 [+4 si message]).
    pub async fn ack_event(&self, eventid: &str, message: Option<String>) -> Result<()> {
        let has_msg = message.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
        let action = if has_msg { 2 + 4 } else { 2 };
        let _ = self
            .event_update(&[eventid], action, message.as_deref())
            .await?;
        eprintln!("[zbx] ACK sent eid={} msg={}", eventid, has_msg);
        Ok(())
    }

    /// Unack simple ou avec message (bitmask: 16 [+4 si message]).  :contentReference[oaicite:4]{index=4}
    pub async fn unack_event(&self, eventid: &str, message: Option<String>) -> Result<()> {
        let has_msg = message.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
        let action = if has_msg { 16 + 4 } else { 16 };
        let _ = self
            .event_update(&[eventid], action, message.as_deref())
            .await?;
        eprintln!("[zbx] UNACK sent eid={} msg={}", eventid, has_msg);
        Ok(())
    }

    pub async fn host_meta_for_event(&self, eventid: &str) -> Result<Option<HostMeta>> {
        let params = json!({
            "output": ["eventid","clock"],
            "selectHosts": ["host","name","status"], // on récupère aussi le status
            "eventids": eventid
        });

        let result: Vec<EventWithHosts> = self.call("event.get", params, 2).await?;
        let meta = result
            .first()
            .and_then(|e| e.hosts.first())
            .map(|h| HostMeta {
                display_name: h
                    .name
                    .clone()
                    .or(h.host.clone())
                    .unwrap_or_else(|| "-".into()),
                status: h.status, // Some(0|1) ou None
            });

        Ok(meta)
    }

    fn call_blocking<T: DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
        id: u32,
    ) -> Result<T> {
        // headers "application/json-rpc"
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/json-rpc"),
        );

        // client HTTP/1.1 avec timeouts
        #[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
        let mut builder = reqb::Client::builder()
            .default_headers(headers)
            .http1_only()
            .user_agent(concat!("alerting/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .danger_accept_invalid_certs(true);

        let http = builder.build().context("building blocking HTTP client")?;

        let payload = RpcRequest {
            jsonrpc: "2.0",
            method,
            params,
            id,
            auth: &self.token,
        };

        let resp = http
            .post(&self.url)
            .json(&payload)
            .send()
            .with_context(|| format!("HTTP POST (blocking) to {} failed", self.url))?;

        let status = resp.status();
        let env: ZbxEnvelope<T> = resp
            .json()
            .with_context(|| format!("Decoding JSON response (HTTP {status}) [blocking]"))?;

        if let Some(err) = env.error {
            bail!(
                "Zabbix API error {}: {}{}",
                err.code,
                err.message,
                err.data.map(|d| format!(" – {d}")).unwrap_or_default()
            );
        }
        env.result
            .ok_or_else(|| anyhow::anyhow!("Zabbix API: missing result field [blocking]"))
    }

    fn event_update_blocking(
        &self,
        eventids: &[&str],
        action: i32,
        message: Option<&str>,
    ) -> Result<Value> {
        // eventids en entiers si possible (sinon strings)
        let ids_json: Vec<Value> = eventids
            .iter()
            .map(|e| {
                e.parse::<i64>()
                    .map(Value::from)
                    .unwrap_or_else(|_| Value::from(*e))
            })
            .collect();

        let mut params = json!({ "eventids": ids_json, "action": action });
        if let Some(msg) = message {
            if !msg.is_empty() {
                params["message"] = json!(msg);
            }
        }
        let res = self.call_blocking::<Value>("event.acknowledge", params, 1777)?;
        eprintln!("[zbx/blocking] event.acknowledge OK: {}", res);
        Ok(res)
    }

    pub fn ack_event_blocking(&self, eventid: &str, message: Option<String>) -> Result<()> {
        let has_msg = message.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
        let action = if has_msg { 2 + 4 } else { 2 }; // 6 = ack + comment
        let _ = self.event_update_blocking(&[eventid], action, message.as_deref())?;
        eprintln!("[zbx/blocking] ACK sent eid={} msg={}", eventid, has_msg);
        Ok(())
    }

    pub fn unack_event_blocking(&self, eventid: &str, message: Option<String>) -> Result<()> {
        let has_msg = message.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
        let action = if has_msg { 16 + 4 } else { 16 }; // 20 = unack + comment
        let _ = self.event_update_blocking(&[eventid], action, message.as_deref())?;
        eprintln!("[zbx/blocking] UNACK sent eid={} msg={}", eventid, has_msg);
        Ok(())
    }
}
