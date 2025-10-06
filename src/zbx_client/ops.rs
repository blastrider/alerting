use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::warn;

use crate::Result;
use crate::error::Error;
use crate::types::AckFilter;

use super::ZbxClient;
use super::models::{EventWithHosts, HostMeta, Problem, RawProblem};

impl ZbxClient {
    /// Fetch all active problems.
    ///
    /// # Errors
    ///
    /// Returns an error when the RPC call fails, the response cannot be
    /// deserialised, or it misses expected fields.
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

    /// Acknowledge a Zabbix event.
    ///
    /// # Errors
    ///
    /// Propagates errors coming from the underlying RPC call.
    pub async fn ack_event(&self, eventid: &str, message: Option<String>) -> Result<()> {
        self.event_update(eventid, true, message).await
    }

    /// Remove an acknowledgement from a Zabbix event.
    ///
    /// # Errors
    ///
    /// Propagates errors coming from the underlying RPC call.
    pub async fn unack_event(&self, eventid: &str, message: Option<String>) -> Result<()> {
        self.event_update(eventid, false, message).await
    }

    /// Resolve host metadata for the provided events.
    ///
    /// # Errors
    ///
    /// Returns an error when the RPC call fails for any of the events.
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
                let Ok(_permit) = semaphore.acquire_owned().await else {
                    return (idx, Ok(None));
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

    async fn event_update(&self, eventid: &str, ack: bool, message: Option<String>) -> Result<()> {
        let mut params = json!({
            "eventids": [eventid],
        });
        let action = if ack { 2 } else { 16 };
        params["action"] = json!(action);
        if let Some(msg) = message.as_deref()
            && !msg.is_empty()
        {
            params["message"] = json!(msg);
            params["action"] = json!(action + 4);
        }
        let _: Value = self.call("event.acknowledge", params).await?;
        Ok(())
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
