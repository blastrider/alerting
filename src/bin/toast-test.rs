#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
use std::sync::mpsc;
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use windows::{
    Data::Xml::Dom::XmlDocument,
    Foundation::{EventRegistrationToken, IPropertyValue, TypedEventHandler},
    UI::Notifications::{
        ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs,
        ToastFailedEventArgs, ToastNotification, ToastNotificationManager,
    },
    Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
    core::{HSTRING, Interface, Result},
};

#[cfg(windows)]
use winrt_notification::Toast;

#[cfg(windows)]
struct ComApartment;

#[cfg(windows)]
impl ComApartment {
    fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}

#[cfg(windows)]
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

#[cfg(windows)]
fn main() -> Result<()> {
    let _apartment = ComApartment::new()?;

    let toast_definition = r#"<toast activationType="foreground">
  <visual>
    <binding template="ToastGeneric">
      <text>Toast test</text>
      <text>coucou</text>
    </binding>
  </visual>
  <actions>
    <input id="userText" type="text" placeHolderContent="Saisissez votre texte ici"/>
    <action content="Valider" arguments="submit" activationType="foreground" hint-inputId="userText"/>
  </actions>
</toast>"#;

    let toast_payload = HSTRING::from(toast_definition);
    let document = XmlDocument::new()?;
    document.LoadXml(&toast_payload)?;

    let toast = ToastNotification::CreateToastNotification(&document)?;
    let (tx, rx) = mpsc::channel::<String>();

    // Relay activation events so we can dump the captured text in the console.
    let activated_sender = tx.clone();
    let activated_token: EventRegistrationToken = toast.Activated(&TypedEventHandler::new(
        move |_sender: &Option<ToastNotification>, args: &Option<windows::core::IInspectable>| {
            if let Some(args) = args {
                if let Ok(args) = args.cast::<ToastActivatedEventArgs>() {
                    let arguments = args.Arguments().unwrap_or_default().to_string();
                    let user_text = args
                        .UserInput()
                        .ok()
                        .and_then(|input| input.Lookup(&HSTRING::from("userText")).ok())
                        .and_then(|value| value.cast::<IPropertyValue>().ok())
                        .and_then(|value| value.GetString().ok())
                        .map(|s| s.to_string());

                    let message = if let Some(text) = user_text {
                        format!("Texte validé : {}", text)
                    } else if !arguments.is_empty() {
                        format!("Activation reçue : {}", arguments)
                    } else {
                        "Activation reçue sans texte".to_owned()
                    };

                    let _ = activated_sender.send(message);
                }
            }
            Ok(())
        },
    ))?;

    let dismissed_sender = tx.clone();
    let dismissed_token: EventRegistrationToken = toast.Dismissed(&TypedEventHandler::new(
        move |_sender: &Option<ToastNotification>, args: &Option<ToastDismissedEventArgs>| {
            if let Some(args) = args {
                let reason = match args.Reason()? {
                    ToastDismissalReason::ApplicationHidden => "fermé par l'application",
                    ToastDismissalReason::UserCanceled => "fermé par l'utilisateur",
                    ToastDismissalReason::TimedOut => "fermé automatiquement",
                    _ => "fermé (raison inconnue)",
                };
                let _ = dismissed_sender.send(format!("Toast {}", reason));
            }
            Ok(())
        },
    ))?;

    let failure_sender = tx.clone();
    let failed_token: EventRegistrationToken = toast.Failed(&TypedEventHandler::new(
        move |_sender: &Option<ToastNotification>, args: &Option<ToastFailedEventArgs>| {
            let message = if let Some(args) = args {
                match args.ErrorCode() {
                    Ok(code) => {
                        format!("Échec de la notification : HRESULT 0x{:08X}", code.0 as u32)
                    }
                    Err(_) => "Échec de la notification (code inconnu)".to_owned(),
                }
            } else {
                "Échec de la notification".to_owned()
            };
            let _ = failure_sender.send(message);
            Ok(())
        },
    ))?;

    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
        Toast::POWERSHELL_APP_ID,
    ))?;
    notifier.Show(&toast)?;
    println!("Toast 'toast-test' affiché. Saisissez du texte puis cliquez sur 'Valider'.");

    match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(message) => println!("{}", message),
        Err(_) => println!("Aucune interaction reçue dans les 60 secondes."),
    }

    toast.RemoveActivated(activated_token)?;
    toast.RemoveDismissed(dismissed_token)?;
    toast.RemoveFailed(failed_token)?;

    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("toast-test ne fonctionne que sur Windows.");
}
