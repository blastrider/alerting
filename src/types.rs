use std::fmt::{self, Display};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AckFilter {
    Acked,
    Unacked,
    All,
}

impl AckFilter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Acked => "ack",
            Self::Unacked => "unack",
            Self::All => "all",
        }
    }
}

impl Display for AckFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AckFilter {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "ack" | "acked" => Ok(Self::Acked),
            "unack" | "unacked" => Ok(Self::Unacked),
            "all" => Ok(Self::All),
            other => Err(format!("unknown ack filter: {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Average,
    High,
    Disaster,
}

impl Severity {
    pub fn from_zabbix(code: i64) -> Option<Self> {
        match code {
            1 => Some(Self::Info),
            2 => Some(Self::Warning),
            3 => Some(Self::Average),
            4 => Some(Self::High),
            5 => Some(Self::Disaster),
            _ => None,
        }
    }

    pub fn as_zabbix_code(self) -> i64 {
        match self {
            Self::Info => 1,
            Self::Warning => 2,
            Self::Average => 3,
            Self::High => 4,
            Self::Disaster => 5,
        }
    }
}

impl Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Info => "Info",
            Severity::Warning => "Warning",
            Severity::Average => "Average",
            Severity::High => "High",
            Severity::Disaster => "Disaster",
        })
    }
}

impl FromStr for Severity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "info" | "information" => Ok(Self::Info),
            "warn" | "warning" => Ok(Self::Warning),
            "average" => Ok(Self::Average),
            "high" => Ok(Self::High),
            "disaster" => Ok(Self::Disaster),
            other => Err(format!("unknown severity: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AckFilter, Severity};
    use std::str::FromStr;

    #[test]
    fn ack_filter_from_str_accepts_variants() {
        assert_eq!(AckFilter::from_str("ack"), Ok(AckFilter::Acked));
        assert_eq!(AckFilter::from_str("unacked"), Ok(AckFilter::Unacked));
        assert_eq!(AckFilter::from_str("ALL"), Ok(AckFilter::All));
        assert!(AckFilter::from_str("maybe").is_err());
    }

    #[test]
    fn severity_from_zabbix_parses_known_codes() {
        assert_eq!(Severity::from_zabbix(4), Some(Severity::High));
        assert_eq!(Severity::from_zabbix(1), Some(Severity::Info));
        assert!(Severity::from_zabbix(42).is_none());
    }
}
