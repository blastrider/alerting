use crate::zbx::ZbxClient;
use anyhow::{Context, Result, anyhow};
use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use tokio::runtime::Handle;
use windows::Win32::Foundation::BOOL;
use windows::Win32::System::Com::IPersistFile;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::core::{GUID, Interface, PCWSTR, PROPVARIANT};
use winrt_notification::{
    Duration as WinDuration, IconCrop, LoopableSound, Scenario, Sound, Toast,
};

use super::{ToastTimeout, ToastUrgency};

/// Structure conservée pour compat, les toasts Windows ne supportent pas les actions.
#[derive(Clone)]
pub struct AckControls {
    #[allow(dead_code)]
    pub handle: Handle,
    #[allow(dead_code)]
    pub client: ZbxClient,
    #[allow(dead_code)]
    pub eventid: String,
    #[allow(dead_code)]
    pub ask_message: bool,
    #[allow(dead_code)]
    pub allow_unack: bool,
    #[allow(dead_code)]
    pub ack_label: Option<String>,
    #[allow(dead_code)]
    pub unack_label: Option<String>,
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
    if ack_controls.is_some() {
        eprintln!("(ui) Ack/Unack non supporté sur Windows, bouton ignoré");
    }
    if action_open.is_some() {
        eprintln!("(ui) Bouton 'Ouvrir' non interactif sur Windows (non supporté)");
    }

    let toast_app_id = if appname.trim().is_empty() {
        Toast::POWERSHELL_APP_ID
    } else {
        appname
    };

    if let Err(err) = ensure_app_registration(toast_app_id) {
        eprintln!("(ui) impossible d’enregistrer l’AppUserModelID '{toast_app_id}': {err:#}");
    }

    let (duration, scenario) = map_timeout(timeout);
    let sound = map_sound(urgency, matches!(timeout, ToastTimeout::Never));

    let mut toast = Toast::new(toast_app_id)
        .title(summary)
        .duration(duration)
        .sound(sound);

    if let Some(scenario) = scenario {
        toast = toast.scenario(scenario);
    }

    let body_lines: Vec<&str> = body
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect();

    if let Some(line) = body_lines.get(0) {
        toast = toast.text1(line);
    } else if !summary.is_empty() {
        toast = toast.text1(summary);
    }

    if let Some(line) = body_lines.get(1) {
        let mut text = (*line).to_string();
        if body_lines.len() > 2 {
            let tail = body_lines[2..].join(" – ");
            if !tail.is_empty() {
                if !text.is_empty() {
                    text.push_str(" – ");
                }
                text.push_str(&tail);
            }
        }
        toast = toast.text2(&text);
    } else if body_lines.len() <= 1 && !body_lines.is_empty() {
        // single line body already used as text1; keep text2 empty
    } else if !summary.is_empty() {
        toast = toast.text2(summary);
    }

    if let Some(icon_path) = icon {
        toast = toast.icon(icon_path, IconCrop::Square, summary);
    }

    toast
        .show()
        .map_err(|err| anyhow!("échec toast WinRT: {err}"))?;

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

const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};
