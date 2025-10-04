use crate::zbx::ZbxClient;
use anyhow::{Context, Result, anyhow};
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};
use std::time::Duration;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::{EventRegistrationToken, IPropertyValue, TypedEventHandler};
use windows::UI::Notifications::{
    ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs, ToastFailedEventArgs,
    ToastNotification, ToastNotificationManager,
};
use windows::Win32::Foundation::BOOL;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};
use windows::Win32::UI::Shell::{IShellLinkW, SetCurrentProcessExplicitAppUserModelID, ShellLink};
use windows::core::{GUID, HSTRING, IInspectable, Interface, PCWSTR, PROPVARIANT};
use winrt_notification::{Duration as WinDuration, LoopableSound, Scenario, Sound, Toast};

use super::{ToastTimeout, ToastUrgency};

#[derive(Clone)]
pub struct AckControls {
    pub client: ZbxClient,
    pub eventid: String,
    pub ask_message: bool,
    pub ack_label: Option<String>,
}

pub fn send_toast(
    summary: &str,
    body: &str,
    urgency: ToastUrgency,
    timeout: ToastTimeout,
    appname: &str,
    icon: Option<&Path>,
    _replace_id: Option<u32>,
    action_open: Option<&str>,
    _action_open_label: &str,
    ack_controls: Option<AckControls>,
) -> Result<()> {
    debug_log(format!(
        "send_toast summary='{}' body_len={} urgency={:?} ack_controls={} appname='{}'",
        summary,
        body.len(),
        urgency,
        ack_controls.is_some(),
        appname
    ));
    let preferred_app_id = if appname.trim().is_empty() {
        Toast::POWERSHELL_APP_ID
    } else {
        appname
    };

    let toast_app_id = match ensure_app_registration(preferred_app_id) {
        Ok(_) => preferred_app_id,
        Err(err) => {
            eprintln!(
                "(ui) impossible d’enregistrer l’AppUserModelID '{preferred_app_id}': {err:#}. "
            );
            eprintln!("(ui) utilisation du fallback Toast::POWERSHELL_APP_ID");
            Toast::POWERSHELL_APP_ID
        }
    };

    debug_log(format!("set_current_process_app_id -> {}", toast_app_id));
    if let Err(err) = set_current_process_app_id(toast_app_id) {
        eprintln!(
            "(ui) impossible de définir AppUserModelID pour ce processus ({toast_app_id}): {err:#}",
        );
    }

    if let Some(ack_controls) = ack_controls {
        if action_open.is_some() {
            eprintln!(
                "(ui) Bouton 'Ouvrir' ignoré sur Windows pour les toasts interactifs — non implémenté",
            );
        }
        return send_interactive_ack_toast(
            summary,
            body,
            urgency,
            timeout,
            toast_app_id,
            icon,
            ack_controls,
        );
    }

    if action_open.is_some() {
        eprintln!("(ui) Bouton 'Ouvrir' non interactif sur Windows (non supporté)");
    }

    let (duration, scenario) = map_timeout(timeout);
    let sound = map_sound(urgency, matches!(timeout, ToastTimeout::Never));

    let toast_definition = build_simple_toast_xml(summary, body, icon, duration, scenario, sound);
    debug_log(format!("toast XML simple: {toast_definition}"));
    show_simple_toast(toast_app_id, &toast_definition)
}

fn send_interactive_ack_toast(
    summary: &str,
    body: &str,
    urgency: ToastUrgency,
    timeout: ToastTimeout,
    app_id: &str,
    icon: Option<&Path>,
    ack_controls: AckControls,
) -> Result<()> {
    let AckControls {
        client,
        eventid,
        ask_message,
        ack_label,
    } = ack_controls;

    let ack_label = ack_label.unwrap_or_else(|| "Valider".to_string());
    let placeholder = if ask_message {
        Some("Commentaire (optionnel)")
    } else {
        None
    };

    let (duration, scenario) = map_timeout(timeout);
    let sound = map_sound(urgency, matches!(timeout, ToastTimeout::Never));

    let toast_definition = build_ack_toast_xml(
        summary,
        body,
        icon,
        &ack_label,
        ask_message,
        placeholder,
        duration,
        scenario,
        sound,
    );
    debug_log(format!("toast XML interactif: {toast_definition}"));

    let _apartment = ComApartment::new()?;

    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(toast_definition))?;

    let toast = ToastNotification::CreateToastNotification(&document)?;
    let (tx, rx) = mpsc::channel::<ToastSignal>();

    let activated_sender = tx.clone();
    let activated_token: EventRegistrationToken = toast.Activated(&TypedEventHandler::new(
        move |_sender: &Option<ToastNotification>, args: &Option<IInspectable>| {
            if let Some(args) = args {
                if let Ok(args) = args.cast::<ToastActivatedEventArgs>() {
                    let arguments = args.Arguments().unwrap_or_default().to_string();
                    if arguments == "ack" {
                        let user_text = args
                            .UserInput()
                            .ok()
                            .and_then(|input| input.Lookup(&HSTRING::from("ackMessage")).ok())
                            .and_then(|value| value.cast::<IPropertyValue>().ok())
                            .and_then(|value| value.GetString().ok())
                            .map(|s| s.to_string());
                        let _ = activated_sender.send(ToastSignal::Ack { message: user_text });
                    } else {
                        let _ = activated_sender.send(ToastSignal::Activated(arguments));
                    }
                }
            }
            Ok(())
        },
    ))?;

    let dismissed_sender = tx.clone();
    let dismissed_token: EventRegistrationToken = toast.Dismissed(&TypedEventHandler::new(
        move |_sender: &Option<ToastNotification>, args: &Option<ToastDismissedEventArgs>| {
            if let Some(args) = args {
                let reason = args.Reason()?;
                let _ = dismissed_sender.send(ToastSignal::Dismissed(reason));
            }
            Ok(())
        },
    ))?;

    let failure_sender = tx.clone();
    let failed_token: EventRegistrationToken = toast.Failed(&TypedEventHandler::new(
        move |_sender: &Option<ToastNotification>, args: &Option<ToastFailedEventArgs>| {
            let message = if let Some(args) = args {
                match args.ErrorCode() {
                    Ok(code) => format!("HRESULT 0x{:08X}", code.0 as u32),
                    Err(_) => "code inconnu".to_owned(),
                }
            } else {
                "erreur inconnue".to_owned()
            };
            let _ = failure_sender.send(ToastSignal::Failed(message));
            Ok(())
        },
    ))?;

    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))?;
    notifier.Show(&toast)?;
    println!("Toast Windows affiché ({eventid}) — cliquez sur '{ack_label}' pour acquitter.");

    let outcome = rx.recv_timeout(Duration::from_secs(300));

    let _ = toast.RemoveActivated(activated_token);
    let _ = toast.RemoveDismissed(dismissed_token);
    let _ = toast.RemoveFailed(failed_token);

    match outcome {
        Ok(ToastSignal::Ack { message }) => {
            let trimmed = message
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let has_msg = trimmed.is_some();
            debug_log(format!(
                "ACK received event={} has_msg={} message={:?}",
                eventid, has_msg, trimmed
            ));

            if let Err(err) = client.ack_event_blocking(&eventid, trimmed.clone()) {
                eprintln!("(ui) ack Windows échoué eid={eventid}: {err:#}");
                if has_msg {
                    if let Err(fallback) = client.ack_event_blocking(&eventid, None) {
                        return Err(fallback.context("échec ACK (fallback sans message)"));
                    }
                    eprintln!("(ui) ACK sans message envoyé en repli eid={eventid}");
                } else {
                    return Err(err.context("échec ACK"));
                }
            } else {
                eprintln!("[ui] ACK Windows OK eid={eventid} (msg={})", has_msg);
            }
        }
        Ok(ToastSignal::Activated(arguments)) => {
            debug_log(format!("activation inattendue: {arguments}"));
            eprintln!("(ui) activation inattendue: {arguments}");
        }
        Ok(ToastSignal::Dismissed(reason)) => {
            debug_log(format!("toast dismissed reason={reason:?} event={eventid}"));
            eprintln!("(ui) toast fermé (reason={reason:?}) eid={eventid}");
        }
        Ok(ToastSignal::Failed(message)) => {
            debug_log(format!(
                "toast failed event={} message={}",
                eventid, message
            ));
            return Err(anyhow!("Toast Windows échec: {message}"));
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            debug_log(format!("toast timeout event={eventid}"));
            eprintln!("(ui) toast expiré sans interaction eid={eventid}");
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            debug_log(format!("toast channel disconnected event={eventid}"));
            eprintln!("(ui) canal de toast déconnecté eid={eventid}");
        }
    }

    Ok(())
}

fn show_simple_toast(app_id: &str, toast_definition: &str) -> Result<()> {
    let _apartment = ComApartment::new()?;

    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(toast_definition))?;

    let toast = ToastNotification::CreateToastNotification(&document)?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))?;
    debug_log(format!("show_simple_toast app_id={app_id}"));
    notifier.Show(&toast)?;
    Ok(())
}

fn map_timeout(timeout: ToastTimeout) -> (WinDuration, Option<Scenario>) {
    match timeout {
        ToastTimeout::Default => (WinDuration::Short, None),
        ToastTimeout::Never => (WinDuration::Long, Some(Scenario::Reminder)),
        ToastTimeout::Milliseconds(ms) => {
            if ms <= 8_000 {
                (WinDuration::Short, None)
            } else {
                (WinDuration::Long, None)
            }
        }
    }
}

fn map_sound(urgency: ToastUrgency, sticky: bool) -> Option<Sound> {
    match urgency {
        ToastUrgency::Low => None,
        ToastUrgency::Normal => Some(Sound::Default),
        ToastUrgency::Critical => {
            if sticky {
                Some(Sound::Loop(LoopableSound::Alarm4))
            } else {
                Some(Sound::Single(LoopableSound::Alarm4))
            }
        }
    }
}

fn ensure_app_registration(app_id: &str) -> Result<()> {
    let trimmed = app_id.trim();
    if trimmed.is_empty() || trimmed == Toast::POWERSHELL_APP_ID {
        return Ok(());
    }

    let shortcut_path = start_menu_shortcut_path(trimmed)
        .context("résolution du chemin du raccourci Start Menu")?;
    if shortcut_path.exists() {
        return Ok(());
    }

    create_shortcut(&shortcut_path, trimmed)
        .with_context(|| format!("création du raccourci {:?}", shortcut_path))
}

fn start_menu_shortcut_path(app_id: &str) -> Result<PathBuf> {
    let appdata =
        std::env::var_os("APPDATA").context("variable d’environnement APPDATA absente")?;
    let programs = PathBuf::from(appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs");

    Ok(programs.join(format!("{app_id}.lnk")))
}

fn create_shortcut(shortcut_path: &Path, app_id: &str) -> Result<()> {
    if let Some(parent) = shortcut_path.parent() {
        fs::create_dir_all(parent).context("création du dossier Start Menu")?;
    }

    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }

    let result = (|| {
        let exe_path = std::env::current_exe().context("Chemin exécutable introuvable")?;
        let exe_w = to_wide_path(&exe_path);
        let workdir_w = exe_path
            .parent()
            .map(to_wide_path)
            .unwrap_or_else(|| vec![0]);
        let shortcut_w = to_wide_path(shortcut_path);

        unsafe {
            let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            shell_link.SetPath(PCWSTR(exe_w.as_ptr()))?;
            shell_link.SetWorkingDirectory(PCWSTR(workdir_w.as_ptr()))?;

            let property_store: IPropertyStore = shell_link.cast()?;
            let pv: PROPVARIANT = PROPVARIANT::from(app_id);
            property_store.SetValue(&PKEY_APP_USER_MODEL_ID, &pv)?;
            property_store.Commit()?;

            let persist_file: IPersistFile = shell_link.cast()?;
            persist_file.Save(PCWSTR(shortcut_w.as_ptr()), BOOL(1))?;
        }

        Ok(())
    })();

    unsafe { CoUninitialize() };

    result
}

fn to_wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn build_ack_toast_xml(
    summary: &str,
    body: &str,
    icon: Option<&Path>,
    ack_label: &str,
    include_input: bool,
    placeholder: Option<&str>,
    duration: WinDuration,
    scenario: Option<Scenario>,
    sound: Option<Sound>,
) -> String {
    let visual = build_visual(summary, body, icon);
    let actions = build_ack_actions(ack_label, include_input, placeholder);
    build_toast_xml(visual, Some(actions), duration, scenario, sound)
}

fn build_simple_toast_xml(
    summary: &str,
    body: &str,
    icon: Option<&Path>,
    duration: WinDuration,
    scenario: Option<Scenario>,
    sound: Option<Sound>,
) -> String {
    let visual = build_visual(summary, body, icon);
    build_toast_xml(visual, None, duration, scenario, sound)
}

fn build_visual(summary: &str, body: &str, icon: Option<&Path>) -> String {
    let summary_trimmed = summary.trim();
    let title = if summary_trimmed.is_empty() {
        "Nouvelle alerte Zabbix".to_string()
    } else {
        escape_text(summary_trimmed)
    };

    let body_lines: Vec<String> = body
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(escape_text)
        .collect();

    let mut visual = String::from("  <visual>\n    <binding template=\"ToastGeneric\">\n");

    if let Some(icon_path) = icon {
        let icon_uri = escape_attr(&file_uri(icon_path));
        let alt = if summary_trimmed.is_empty() {
            "Notification".to_string()
        } else {
            escape_attr(summary_trimmed)
        };
        visual.push_str(&format!(
            "      <image placement=\"appLogoOverride\" src=\"{}\" alt=\"{}\" />\n",
            icon_uri, alt
        ));
    }

    visual.push_str(&format!("      <text>{}</text>\n", title));

    match body_lines.split_first() {
        Some((first, rest)) => {
            visual.push_str("      <text>");
            visual.push_str(first);
            visual.push_str("</text>\n");

            if !rest.is_empty() {
                let tail = rest.join(" – ");
                visual.push_str("      <text>");
                visual.push_str(&tail);
                visual.push_str("</text>\n");
            }
        }
        None => {
            visual.push_str("      <text>Aucune information supplémentaire</text>\n");
        }
    }

    visual.push_str("    </binding>\n  </visual>\n");
    visual
}

fn build_ack_actions(ack_label: &str, include_input: bool, placeholder: Option<&str>) -> String {
    let mut actions = String::from("  <actions>\n");
    if include_input {
        let placeholder = escape_attr(placeholder.unwrap_or("Commentaire (optionnel)"));
        actions.push_str(&format!(
            "    <input id=\"ackMessage\" type=\"text\" placeHolderContent=\"{}\" />\n",
            placeholder
        ));
        actions.push_str(&format!(
            "    <action content=\"{}\" arguments=\"ack\" activationType=\"foreground\" hint-inputId=\"ackMessage\" />\n",
            escape_attr(ack_label)
        ));
    } else {
        actions.push_str(&format!(
            "    <action content=\"{}\" arguments=\"ack\" activationType=\"foreground\" />\n",
            escape_attr(ack_label)
        ));
    }
    actions.push_str("  </actions>\n");
    actions
}

fn build_toast_xml(
    visual: String,
    actions: Option<String>,
    duration: WinDuration,
    scenario: Option<Scenario>,
    sound: Option<Sound>,
) -> String {
    let mut toast = String::from("<toast activationType=\"foreground\"");
    toast.push_str(match duration {
        WinDuration::Short => " duration=\"short\"",
        WinDuration::Long => " duration=\"long\"",
    });
    if let Some(attr) = scenario.and_then(scenario_to_attr) {
        toast.push(' ');
        toast.push_str(&attr);
    }
    toast.push_str(">\n");
    toast.push_str(&visual);
    if let Some(actions) = actions {
        toast.push_str(&actions);
    }
    if let Some(audio) = sound_fragment(sound) {
        toast.push_str(&audio);
    }
    toast.push_str("</toast>");
    toast
}

fn scenario_to_attr(scenario: Scenario) -> Option<String> {
    match scenario {
        Scenario::Default => None,
        Scenario::Alarm => Some("scenario=\"alarm\"".to_string()),
        Scenario::Reminder => Some("scenario=\"reminder\"".to_string()),
        Scenario::IncomingCall => Some("scenario=\"incomingCall\"".to_string()),
    }
}

fn sound_fragment(sound: Option<Sound>) -> Option<String> {
    match sound {
        None => Some("  <audio silent=\"true\" />\n".to_string()),
        Some(Sound::Default) => None,
        Some(Sound::Loop(loopable)) => Some(format!(
            "  <audio loop=\"true\" src=\"ms-winsoundevent:Notification.Looping.{}\" />\n",
            loopable
        )),
        Some(Sound::Single(loopable)) => Some(format!(
            "  <audio src=\"ms-winsoundevent:Notification.Looping.{}\" />\n",
            loopable
        )),
        Some(other) => Some(format!(
            "  <audio src=\"ms-winsoundevent:Notification.{}\" />\n",
            other
        )),
    }
}

fn escape_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_attr(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file:///");
    let raw = path.to_string_lossy().replace('\\', "/");
    uri.push_str(&raw);
    uri
}

struct ComApartment;

impl ComApartment {
    fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

enum ToastSignal {
    Ack { message: Option<String> },
    Activated(String),
    Dismissed(ToastDismissalReason),
    Failed(String),
}

const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};

fn set_current_process_app_id(app_id: &str) -> windows::core::Result<()> {
    let hstr = HSTRING::from(app_id);
    unsafe { SetCurrentProcessExplicitAppUserModelID(PCWSTR(hstr.as_ptr())) }
}

fn debug_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| {
        std::env::var("ALERTING_WINDOWS_DEBUG")
            .map(|v| !v.is_empty() && v != "0" && v.to_ascii_lowercase() != "false")
            .unwrap_or(false)
    })
}

fn debug_log(message: impl AsRef<str>) {
    if debug_enabled() {
        eprintln!("[windows-debug] {}", message.as_ref());
    }
}
