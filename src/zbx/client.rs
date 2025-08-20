use anyhow::{bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::sync::Arc;
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
            AckFilter::Unack => { params["acknowledged"] = json!(false); }
            AckFilter::Ack   => { params["acknowledged"] = json!(true);  }
            AckFilter::All   => { /* ne rien ajouter */ }
        }
        self.call("problem.get", params, 1).await
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
            display_name: h.name.clone().or(h.host.clone()).unwrap_or_else(|| "-".into()),
            status: h.status, // Some(0|1) ou None
        });

    Ok(meta)
}

}
