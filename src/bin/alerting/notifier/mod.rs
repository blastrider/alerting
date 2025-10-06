mod backends;

use alerting::Result;
use alerting::config::NotifySettings;
use alerting::error::Error as AlertError;
use alerting::types::Severity;
use alerting::zbx_client::{HostMeta, Problem, ZbxClient};
use async_channel::Receiver;
use std::convert::TryFrom;
#[cfg(target_os = "windows")]
use std::path::Path;
use tokio::task::JoinHandle;
use tracing::{error, info};

use backends::ToastParams;

pub async fn run_notifier(
    rx: Receiver<NotificationItem>,
    notify: NotifySettings,
    client: ZbxClient,
    dry_run: bool,
) {
    while let Ok(item) = rx.recv().await {
        if dry_run {
            info!(
                event_id = %item.problem.event_id,
                host = item.host.as_ref().map_or("<unknown>", |h| h.display_name.as_str()),
                severity = ?item.problem.severity,
                "dry-run: would emit notification"
            );
            continue;
        }

        if let Err(err) = send_notification(&notify, &client, &item) {
            error!(error = %err, event_id = %item.problem.event_id, "failed to send notification");
        }
    }
}

pub struct NotificationItem {
    pub(crate) problem: Problem,
    pub(crate) host: Option<HostMeta>,
    pub(crate) open_url: Option<String>,
}

#[derive(Clone)]
struct AckAction {
    client: ZbxClient,
    event_id: String,
}

impl AckAction {
    pub(crate) fn new(client: &ZbxClient, event_id: &str) -> Self {
        Self {
            client: client.clone(),
            event_id: event_id.to_string(),
        }
    }

    pub(crate) fn spawn_with_message(self, message: Option<String>) -> JoinHandle<()> {
        let Self { client, event_id } = self;
        tokio::spawn(async move {
            match client.ack_event(&event_id, message.clone()).await {
                Ok(()) => {
                    if let Some(msg) = message {
                        tracing::info!(%event_id, message = %msg, "event acknowledged from toast");
                    } else {
                        tracing::info!(%event_id, "event acknowledged from toast");
                    }
                }
                Err(err) => {
                    tracing::warn!(%event_id, error = %err, "failed to acknowledge event from toast");
                }
            }
        })
    }
}

#[derive(Clone, Copy, Debug)]
enum ToastUrgency {
    Low,
    Normal,
    Critical,
}

#[derive(Clone, Copy, Debug)]
enum ToastTimeout {
    Default,
    Never,
    Milliseconds(u32),
}

fn send_notification(
    notify: &NotifySettings,
    client: &ZbxClient,
    item: &NotificationItem,
) -> Result<()> {
    let severity = item.problem.severity;
    let urgency = match severity {
        Severity::Disaster | Severity::High => ToastUrgency::Critical,
        Severity::Average | Severity::Warning => ToastUrgency::Normal,
        Severity::Info => ToastUrgency::Low,
    };

    let timeout_ms = notify.timeout.and_then(|dur| u128_to_u32(dur.as_millis()));
    let timeout = compute_timeout(notify.sticky, timeout_ms, notify.default_timeout);

    let host_label = item
        .host
        .as_ref()
        .map_or("<unknown>", |h| h.display_name.as_str());

    let summary = format!("{severity:?} – {host_label}");
    let body = format!(
        "Event #{} {}\n{}",
        item.problem.event_id,
        if item.problem.acknowledged {
            "[ACK]"
        } else {
            "[UNACK]"
        },
        item.problem.name
    );

    let open_url = item.open_url.clone();

    #[cfg(not(target_os = "linux"))]
    let _ = client;

    #[cfg(target_os = "linux")]
    let ack_action =
        (!item.problem.acknowledged).then(|| AckAction::new(client, &item.problem.event_id));
    #[cfg(not(target_os = "linux"))]
    let ack_action = None;

    let params = ToastParams {
        summary: &summary,
        body: &body,
        urgency,
        timeout,
        appname: &notify.appname,
        icon: notify.icon.as_deref(),
        open_url: open_url.as_deref(),
        open_label: &notify.open_label,
    };

    backends::send_toast(&params, ack_action.as_ref()).map_err(AlertError::from)?;
    Ok(())
}

const fn compute_timeout(
    sticky: bool,
    timeout_ms: Option<u32>,
    default_timeout: bool,
) -> ToastTimeout {
    if sticky {
        ToastTimeout::Never
    } else if let Some(ms) = timeout_ms {
        ToastTimeout::Milliseconds(ms)
    } else if default_timeout {
        ToastTimeout::Default
    } else {
        ToastTimeout::Milliseconds(5_000)
    }
}

#[cfg(target_os = "windows")]
pub fn send_test_toast(
    summary: &str,
    body: &str,
    appname: &str,
    icon: Option<&Path>,
    open_label: &str,
) -> Result<()> {
    let params = ToastParams {
        summary,
        body,
        urgency: ToastUrgency::Normal,
        timeout: ToastTimeout::Milliseconds(5_000),
        appname,
        icon,
        open_url: None,
        open_label,
    };

    backends::send_toast(&params, None).map_err(AlertError::from)
}

fn u128_to_u32(value: u128) -> Option<u32> {
    u32::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::{ToastTimeout, compute_timeout};

    #[test]
    fn timeout_prefers_sticky() {
        let timeout = compute_timeout(true, Some(1000), true);
        assert!(matches!(timeout, ToastTimeout::Never));
    }
}
