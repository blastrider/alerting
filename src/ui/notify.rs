use anyhow::{Context, Result};
use notify_rust::{Notification, Timeout, Urgency};
use std::{path::Path, process::Command};
use crate::zbx::ZbxClient;
use anyhow::anyhow;

/// Contrôles Ack/Unack à insérer dans la notification.
#[derive(Clone)]
pub struct AckControls {
    pub client: ZbxClient,
    pub eventid: String,
    /// Afficher un prompt pour message (optionnel) avant d'envoyer.
    pub ask_message: bool,
    /// Afficher aussi le bouton Unack si déjà ACK.
    pub allow_unack: bool,
    /// Libellés (fallbacks par défaut si None).
    pub ack_label: Option<String>,
    pub unack_label: Option<String>,
}

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
    ack_controls: Option<AckControls>,
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

    if ack_controls.is_some() {
        let ac = ack_controls.as_ref().unwrap();
        builder.action("ack", ac.ack_label.as_deref().unwrap_or("Ack"));
        if ac.allow_unack {
            builder.action("unack", ac.unack_label.as_deref().unwrap_or("Unack"));
        }
    }

    let handle = builder
        .show()
        .context("échec d’affichage de la notification")?;

    let url_opt = action_open.map(|s| s.to_string());
    let ac_opt = ack_controls.clone();
    handle.wait_for_action(move |action| {
        match action {
            "open" => {
                if let Some(ref url) = url_opt {
                    let _ = Command::new("xdg-open").arg(url).spawn();
                }
            }
            "ack" => {
                if let Some(ac) = ac_opt.clone() {
                    let msg = if ac.ask_message { prompt_message().ok().flatten() } else { None };
                    // On ne bloque pas le thread de notif : spawn async.
                    let client = ac.client.clone();
                    let eid = ac.eventid.clone();
                    tokio::spawn(async move {
                        if let Err(e) = client.ack_event(&eid, msg).await {
                            eprintln!("(ack failed) event {}: {}", eid, e);
                        }
                    });
                }
            }
            "unack" => {
                if let Some(ac) = ac_opt.clone() {
                    let msg = if ac.ask_message { prompt_message().ok().flatten() } else { None };
                    let client = ac.client.clone();
                    let eid = ac.eventid.clone();
                    tokio::spawn(async move {
                        if let Err(e) = client.unack_event(&eid, msg).await {
                            eprintln!("(unack failed) event {}: {}", eid, e);
                        }
                    });
                }
            }
            "ignore" | "__closed" | "__timeout" => { /* no-op */ }
            _ => { /* no-op */ }
        }
    });
    Ok(())
}

/// Ouvre un prompt texte (`zenity --entry`) et retourne Some(message) si saisi.
fn prompt_message() -> Result<Option<String>> {
    // Nécessite zenity (Mint/Cinnamon l'a souvent).  :contentReference[oaicite:5]{index=5}
    let output = Command::new("zenity")
        .arg("--entry")
        .arg("--title=Commentaire")
        .arg("--text=Motif (facultatif) :")
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if s.is_empty() { Ok(None) } else { Ok(Some(s)) }
        }
        Ok(_) => Ok(None), // annulé/fermé
        Err(e) => Err(anyhow!("zenity introuvable ou erreur: {e}")),
    }
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
