use std::time::Duration;

use humantime::{format_duration, parse_duration};
use serde::Deserialize;
use serde_with::{DeserializeAs, SerializeAs};

pub(super) struct HumantimeDuration;

impl<'de> DeserializeAs<'de, Duration> for HumantimeDuration {
    fn deserialize_as<D>(deserializer: D) -> std::result::Result<Duration, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        parse_duration(&raw).map_err(serde::de::Error::custom)
    }
}

impl SerializeAs<Duration> for HumantimeDuration {
    fn serialize_as<S>(value: &Duration, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format_duration(*value).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::HumantimeDuration;
    use serde::Deserialize;
    use serde_with::serde_as;
    use std::time::Duration;

    #[test]
    fn humantime_duration_parses_strings() {
        #[serde_as]
        #[derive(Deserialize)]
        struct Sample {
            #[serde_as(as = "Option<HumantimeDuration>")]
            duration: Option<Duration>,
        }

        let sample: Sample = match serde_json::from_str(r#"{"duration":"5s"}"#) {
            Ok(value) => value,
            Err(err) => panic!("failed to parse sample json: {err}"),
        };
        assert_eq!(sample.duration, Some(Duration::from_secs(5)));
    }
}
