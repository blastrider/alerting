use crate::zbx::ZbxClient;
use anyhow::{Context, Result, anyhow};
use notify_rust::{Notification, Timeout as LibTimeout, Urgency as LibUrgency};
use std::path::Path;
use std::process::{Command, Stdio};
use tokio::runtime::Handle; // conservé pour compat (champ présent dans AckControls)

use super::{ToastTimeout, ToastUrgency};

/// Contrôles Ack/Unack à insérer dans la notification (Linux uniquement).
#[derive(Clone)]
pub struct AckControls {
    #[allow(dead_code)]
    pub handle: Handle, // plus utilisé, mais conservé pour compat
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

pub fn send_toast(
    summary: &str,
    body: &str,
    urgency: ToastUrgency,
    timeout: ToastTimeout,
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
        .urgency(map_urgency(urgency))
        .timeout(map_timeout(timeout));

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

    if let Some(ac) = ack_controls.as_ref() {
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
        eprintln!("(ui) action={action}");
        match action {
            "open" => {
                if let Some(ref url) = url_opt {
                    open_url(url);
                }
            }
            "ack" => {
                if let Some(ac) = ac_opt.clone() {
                    let msg = if ac.ask_message {
                        prompt_message().ok().flatten()
                    } else {
                        None
                    };
                    let client = ac.client.clone();
                    let eid = ac.eventid.clone();

                    eprintln!("(ui) ack clicked eid={eid}");
                    let has_msg = msg.as_deref().map(|s| !s.is_empty()).unwrap_or(false);

                    // IMPORTANT: appel BLOQUANT (pas de Tokio ici)
                    if let Err(e) = client.ack_event_blocking(&eid, msg) {
                        eprintln!("(ack failed blocking) eid={eid} : {e:#}");
                        // Fallback : ACK sans message si le commentaire est refusé
                        if has_msg {
                            if let Err(e2) = client.ack_event_blocking(&eid, None) {
                                eprintln!(
                                    "(ack fallback no-msg failed blocking) eid={eid} : {e2:#}"
                                );
                            }
                        }
                    } else {
                        eprintln!("[ui] ack OK eid={eid}");
                    }
                }
            }
            "unack" => {
                if let Some(ac) = ac_opt.clone() {
                    let msg = if ac.ask_message {
                        prompt_message().ok().flatten()
                    } else {
                        None
                    };
                    let client = ac.client.clone();
                    let eid = ac.eventid.clone();

                    // IMPORTANT: appel BLOQUANT (pas de Tokio ici)
                    if let Err(e) = client.unack_event_blocking(&eid, msg) {
                        eprintln!("(unack failed blocking) eid={eid} : {e:#}");
                    } else {
                        eprintln!("[ui] unack OK eid={eid}");
                    }
                }
            }
            "ignore" | "__closed" | "__timeout" => { /* no-op */ }
            _ => { /* no-op */ }
        }
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

fn open_url(url: &str) {
    const SYSTEMD_RUN_BIN: &str = "systemd-run";
    // Launch via systemd-run in a transient user service to avoid inheriting
    // alerting.service hardening (Firefox crash lorsqu'on hérite des flags).
    let systemd_run_status = Command::new(SYSTEMD_RUN_BIN)
        .arg("--user")
        .arg("--collect")
        .arg("--quiet")
        .arg("--property=Description=Alerting open action")
        .arg("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match systemd_run_status {
        Ok(status) if status.success() => return,
        Ok(status) => {
            eprintln!("(ui) systemd-run failed with status {status} for open_url, falling back");
        }
        Err(err) => {
            eprintln!("(ui) systemd-run unavailable for open_url, falling back: {err:?}");
        }
    }

    if let Err(err) = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        eprintln!("(ui) failed to open url with xdg-open: {err:?}");
    }
}

/// Ouvre un prompt texte (`zenity --entry`) et retourne Some(message) si saisi.
fn prompt_message() -> Result<Option<String>> {
    // Nécessite zenity (Mint/Cinnamon l'a souvent).
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
