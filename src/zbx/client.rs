use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::{sync::Semaphore, task::JoinSet};

use super::types::{EventWithHosts, Problem, RpcRequest, ZbxEnvelope};

/// Client Zabbix minimaliste et clonable.
#[derive(Clone)]
pub struct ZbxClient {
    http: reqwest::Client,
    url: String,
    token: String,
}

impl ZbxClient {
    pub fn new(url: &str, token: &str) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json-rpc"));
        let http = reqwest::Client::builder().default_headers(headers).build()?;
        Ok(Self { http, url: url.to_string(), token: token.to_string() })
    }

    async fn call<T: DeserializeOwned>(&self, method: &str, params: Value, id: u32) -> Result<T> {
        let payload = RpcRequest { jsonrpc: "2.0", method, params, id, auth: &self.token };

        let resp = self.http.post(&self.url).json(&payload).send()
            .await
            .with_context(|| format!("HTTP POST to {}", self.url))?;

        let status = resp.status();
        let env: ZbxEnvelope<T> = resp.json()
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
        env.result.ok_or_else(|| anyhow::anyhow!("Zabbix API: missing result field"))
    }

    /// Récupère les problèmes récents (tri décroissant par eventid).
    pub async fn recent_problems(&self, limit: u32) -> Result<Vec<Problem>> {
        let params = json!({
            "output": ["eventid","name","severity","clock","objectid"],
            "recent": true,
            "limit": limit,
            "sortfield": ["eventid"],
            "sortorder": "DESC"
        });
        self.call("problem.get", params, 1).await
    }

    /// Renvoie le nom d’hôte pour un eventid (None si absent).
    pub async fn host_for_event(&self, eventid: &str) -> Result<Option<String>> {
        let params = json!({
            "output": ["eventid","clock"],
            "selectHosts": ["host","name"],
            "eventids": eventid
        });

        let result: Vec<EventWithHosts> = self.call("event.get", params, 2).await?;
        let host = result
            .first()
            .and_then(|e| e.hosts.first())
            .and_then(|h| h.name.clone().or(h.host.clone())); // Option<String>
        Ok(host)
    }

    /// Résout les hôtes pour une liste d'eventids, avec limite de parallélisme.
    pub async fn resolve_hosts_concurrent(
        &self,
        eventids: &[String],
        max_concurrency: usize,
    ) -> Result<Vec<Option<String>>> {
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

                let host = this.host_for_event(&eid).await.unwrap_or(None);
                (idx, host)
            });
        }

        let mut out = vec![None; eventids.len()];
        while let Some(res) = joins.join_next().await {
            let (idx, host) = res?; // propage JoinError en anyhow::Error
            out[idx] = host;
        }
        Ok(out)
    }
}
