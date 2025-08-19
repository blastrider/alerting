use anyhow::{Context, Result};
use notify_rust::{Notification, Timeout, Urgency};
use std::{path::Path, process::Command};

/// Affiche une notification système (Cinnamon/Mint).
/// Si `action_open` est `Some(url)`, ajoute un bouton "open" qui lance `xdg-open url`.
pub fn send_toast(
    summary: &str,
    body: &str,
    urgency: Urgency,
    timeout: Timeout,
    appname: &str,
    icon: Option<&Path>,
    replace_id: Option<u32>,
    action_open: Option<&str>,
    action_open_label: &str,
) -> Result<()> {
    let mut builder = Notification::new();
    builder
        .summary(summary)
        .body(body)
        .appname(appname)
        .urgency(urgency)
        .timeout(timeout);

    if let Some(icon_path) = icon {
        builder.icon(&icon_path.to_string_lossy());
    }
    if let Some(id) = replace_id {
        builder.id(id);
    }
    if action_open.is_some() {
        builder.action("open", action_open_label);
        builder.action("ignore", "Ignorer");
    }

    let handle = builder
        .show()
        .context("échec d’affichage de la notification")?;

    if let Some(url) = action_open {
        handle.wait_for_action(move |action| match action {
            "open" => {
                let _ = Command::new("xdg-open").arg(url).spawn();
            }
            "ignore" | "__closed" | "__timeout" => { /* no-op */ }
            _ => { /* no-op */ }
        });
    }
    Ok(())
}

/// Calcule le timeout notify-osd selon trois flags.
pub fn compute_timeout(sticky: bool, timeout_ms: Option<u32>, default_timeout: bool) -> Timeout {
    if sticky {
        Timeout::Never
    } else if let Some(ms) = timeout_ms {
        Timeout::Milliseconds(ms)
    } else if default_timeout {
        Timeout::Default
    } else {
        Timeout::Milliseconds(5_000)
    }
}
