use std::fmt;

/// Enum typée pour la sévérité Zabbix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    NotClassified,
    Information,
    Warning,
    Average,
    High,
    Disaster,
    Unknown(u8),
}

impl From<u8> for Severity {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::NotClassified,
            1 => Self::Information,
            2 => Self::Warning,
            3 => Self::Average,
            4 => Self::High,
            5 => Self::Disaster,
            x => Self::Unknown(x),
        }
    }
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::NotClassified => "Not classified",
            Severity::Information => "Information",
            Severity::Warning => "Warning",
            Severity::Average => "Average",
            Severity::High => "High",
            Severity::Disaster => "Disaster",
            Severity::Unknown(_) => "Unknown",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
