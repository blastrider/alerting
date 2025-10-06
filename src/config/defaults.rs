use std::time::Duration;

pub(super) const fn default_limit() -> u32 {
    20
}

pub(super) const fn default_concurrency() -> usize {
    4
}

pub(super) fn default_ack_filter() -> String {
    "unacked".to_string()
}

pub(super) fn default_notify_appname() -> String {
    "Alerting".to_string()
}

pub(super) fn default_open_label() -> String {
    "Open".to_string()
}

pub(super) const fn default_max_notif() -> usize {
    5
}

pub(super) const fn default_queue_bound() -> usize {
    64
}

pub(super) const fn default_dedup_cache_size() -> usize {
    256
}

pub(super) const fn default_rate_limit_max() -> usize {
    3
}

pub(super) const fn default_rate_limit_window() -> Duration {
    Duration::from_secs(5)
}

pub(super) const fn default_poll_interval() -> Duration {
    Duration::from_secs(30)
}
