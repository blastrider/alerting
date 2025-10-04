/// Cross-platform urgency levels understood by the UI layer.
#[derive(Clone, Copy, Debug)]
pub enum ToastUrgency {
    Low,
    Normal,
    Critical,
}

/// Cross-platform timeout abstraction (some backends only support coarse control).
#[derive(Clone, Copy, Debug)]
pub enum ToastTimeout {
    Default,
    Never,
    Milliseconds(u32),
}

/// Calcule le timeout de notification selon trois flags indépendamment de la plateforme.
pub fn compute_timeout(
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
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::{AckControls, send_toast};
#[cfg(target_os = "windows")]
pub use windows::{AckControls, send_toast};
