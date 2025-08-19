use chrono::{Local, TimeZone};

/// Formatte un epoch (secondes) en heure locale lisible.
pub fn fmt_epoch_local(sec: i64) -> String {
    Local
        .timestamp_opt(sec, 0)
        .single()
        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| format!("(horodatage invalide: {sec})"))
}
