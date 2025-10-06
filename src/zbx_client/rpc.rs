use serde::{Deserialize, Serialize};
use serde_json::Value;

const BODY_PREVIEW_LIMIT: usize = 256;

#[derive(Debug, Deserialize)]
pub(super) struct RpcEnvelope<T> {
    #[allow(dead_code)]
    pub(crate) jsonrpc: String,
    pub(crate) result: Option<T>,
    pub(crate) error: Option<RpcError>,
    #[allow(dead_code)]
    pub(crate) id: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct RpcError {
    pub(crate) code: i64,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) data: Option<String>,
}

#[derive(Serialize)]
pub(super) struct RpcRequest<'a> {
    pub(crate) jsonrpc: &'static str,
    pub(crate) method: &'a str,
    pub(crate) params: Value,
    pub(crate) id: u64,
    pub(crate) auth: &'a str,
}

pub(super) fn body_preview(body: &[u8]) -> String {
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
