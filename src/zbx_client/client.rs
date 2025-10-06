use std::time::{Duration, Instant};

use backoff::ExponentialBackoffBuilder;
use backoff::backoff::Backoff;
use reqwest::StatusCode;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::time::sleep;
use tracing::{debug, warn};
use url::Url;
use uuid::Uuid;

use crate::Result;
use crate::error::{Error, ZbxError};

use super::rpc::{RpcEnvelope, RpcRequest, body_preview};

const MAX_ATTEMPTS: usize = 3;
const CORRELATION_HEADER: &str = "x-correlation-id";

#[derive(Clone)]
pub struct ZbxClient {
    http: reqwest::Client,
    base: Url,
    token: SecretString,
    timeout: Duration,
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

    pub(super) async fn call<T>(&self, method: &str, params: Value) -> Result<T>
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
}
