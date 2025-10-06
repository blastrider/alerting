use alerting::error::NotifyError;

use super::{AckAction, ToastTimeout, ToastUrgency};

#[cfg(target_os = "linux")]
pub(crate) fn send_toast(
    summary: &str,
    body: &str,
    urgency: ToastUrgency,
    timeout: ToastTimeout,
    appname: &str,
    icon: Option<&std::path::Path>,
    open_url: Option<&str>,
    open_label: &str,
    ack_action: Option<AckAction>,
) -> std::result::Result<(), NotifyError> {
    linux::send_toast(
        summary, body, urgency, timeout, appname, icon, open_url, open_label, ack_action,
    )
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn send_toast(
    summary: &str,
    body: &str,
    urgency: ToastUrgency,
    timeout: ToastTimeout,
    appname: &str,
    icon: Option<&std::path::Path>,
    open_url: Option<&str>,
    open_label: &str,
    ack_action: Option<AckAction>,
) -> std::result::Result<(), NotifyError> {
    #[cfg(target_os = "windows")]
    {
        return windows::send_toast(
            summary, body, urgency, timeout, appname, icon, open_url, open_label, ack_action,
        );
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = (
            summary, body, urgency, timeout, appname, icon, open_url, open_label, ack_action,
        );
        Err(NotifyError::Backend)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use alerting::error::NotifyError;
    use notify_rust::{Notification, Timeout as LibTimeout, Urgency as LibUrgency};
    use std::path::Path;
    use std::process::{Command, Stdio};
    use tracing::trace;

    use super::super::{AckAction, ToastTimeout, ToastUrgency};

    pub fn send_toast(
        summary: &str,
        body: &str,
        urgency: ToastUrgency,
        timeout: ToastTimeout,
        appname: &str,
        icon: Option<&Path>,
        open_url: Option<&str>,
        open_label: &str,
        ack_action: Option<AckAction>,
    ) -> std::result::Result<(), NotifyError> {
        let mut builder = Notification::new();
        builder
            .summary(summary)
            .body(body)
            .appname(appname)
            .urgency(map_urgency(urgency))
            .timeout(map_timeout(timeout));

        if let Some(icon_path) = icon {
            builder.icon(&icon_path.to_string_lossy());
        }

        const ACK_KEY: &str = "ack";
        const OPEN_KEY: &str = "open";
        const DISMISS_KEY: &str = "dismiss";
        const ACK_LABEL: &str = "Acquitter";

        if ack_action.is_some() {
            builder.action(ACK_KEY, ACK_LABEL);
        }

        if open_url.is_some() {
            builder.action(OPEN_KEY, open_label);
        }

        builder.action(DISMISS_KEY, "Ignorer");

        let handle = builder.show().map_err(|_| NotifyError::Backend)?;
        let open = open_url.map(|url| url.to_string());
        let ack = ack_action.clone();

        handle.wait_for_action(move |action| match action {
            OPEN_KEY => {
                if let Some(url) = open.as_deref() {
                    let _ = Command::new("xdg-open")
                        .arg(url)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn();
                }
            }
            ACK_KEY => {
                if let Some(ack_action) = ack.clone() {
                    trace!("ack action triggered from toast");
                    let message = prompt_ack_message();
                    ack_action.spawn_with_message(message);
                }
            }
            _ => {}
        });
        Ok(())
    }

    fn map_urgency(urgency: ToastUrgency) -> LibUrgency {
        match urgency {
            ToastUrgency::Low => LibUrgency::Low,
            ToastUrgency::Normal => LibUrgency::Normal,
            ToastUrgency::Critical => LibUrgency::Critical,
        }
    }

    fn map_timeout(timeout: ToastTimeout) -> LibTimeout {
        match timeout {
            ToastTimeout::Default => LibTimeout::Default,
            ToastTimeout::Never => LibTimeout::Never,
            ToastTimeout::Milliseconds(ms) => LibTimeout::Milliseconds(ms),
        }
    }

    fn prompt_ack_message() -> Option<String> {
        let output = Command::new("zenity")
            .arg("--entry")
            .arg("--title")
            .arg("Acquitter l'evenement")
            .arg("--text")
            .arg("Message d'acquittement (laisser vide pour aucun)")
            .output();

        let output = match output {
            Ok(out) => out,
            Err(err) => {
                trace!(error = %err, "failed to launch zenity for ack message");
                return None;
            }
        };

        if !output.status.success() {
            return None;
        }

        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() { None } else { Some(text) }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use alerting::error::NotifyError;
    use std::path::Path;
    use windows::UI::Notifications::{NotificationSetting, ToastNotificationManager};
    use windows::core::HSTRING;
    use winrt_notification::{Duration as WinDuration, LoopableSound, Scenario, Sound, Toast};

    use super::super::{AckAction, ToastTimeout, ToastUrgency};

    pub fn send_toast(
        summary: &str,
        body: &str,
        urgency: ToastUrgency,
        timeout: ToastTimeout,
        appname: &str,
        _icon: Option<&Path>,
        _open_url: Option<&str>,
        _open_label: &str,
        _ack_action: Option<AckAction>,
    ) -> std::result::Result<(), NotifyError> {
        let app_id = if appname.trim().is_empty() {
            Toast::POWERSHELL_APP_ID
        } else {
            appname
        };
        let timeout_kind = match timeout {
            ToastTimeout::Never => "never",
            ToastTimeout::Default => "default",
            ToastTimeout::Milliseconds(_) => "custom",
        };
        tracing::debug!(
            summary,
            app_id,
            timeout = timeout_kind,
            urgency = ?urgency,
            "sending windows toast"
        );

        match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id)) {
            Ok(notifier) => {
                if let Ok(setting) = notifier.Setting() {
                    tracing::debug!(
                        setting = ?setting,
                        "windows toast notification setting"
                    );
                    if setting != NotificationSetting::Enabled {
                        tracing::warn!(?setting, "toast notifications are disabled for this app");
                    }
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to query toast manager");
            }
        }

        let toast = Toast::new(app_id)
            .title(summary)
            .text1(body)
            .duration(match timeout {
                ToastTimeout::Never => WinDuration::Long,
                _ => WinDuration::Short,
            })
            .scenario(match urgency {
                ToastUrgency::Critical => Scenario::Alarm,
                ToastUrgency::Normal => Scenario::Reminder,
                ToastUrgency::Low => Scenario::IncomingCall,
            })
            .sound(match urgency {
                ToastUrgency::Critical => Some(Sound::Loop(LoopableSound::Alarm)),
                ToastUrgency::Normal => Some(Sound::Default),
                ToastUrgency::Low => Some(Sound::Reminder),
            });

        if let Err(err) = toast.show() {
            tracing::warn!(error = %err, "windows toast failed");
            return Err(NotifyError::Backend);
        }
        tracing::debug!("windows toast displayed");
        Ok(())
    }
}
