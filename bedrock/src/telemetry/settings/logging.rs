use crate::telemetry::settings::rate_limit::RateLimitingSettings;
use crate::utils::feature_use;

use std::ops::Deref;
use std::path::PathBuf;

feature_use!(cfg(feature = "settings"), {
    use crate::settings::{settings, Settings};
    use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
    use std::str::FromStr;
});

// NOTE: we technically don't need a feature gate here, but if we don't add it then docs don't
// mark this re-export as available on when `logging` is enabled.
#[cfg(feature = "logging")]
pub use slog::Level;

/// Logging settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Default, Debug))]
pub struct LoggingSettings {
    /// Specifies log output.
    pub output: LogOutput,

    /// The format to use for log messages.
    pub format: LogFormat,

    /// Set the logging verbosity level.
    pub verbosity: LogVerbosity,

    /// A list of field keys to redact when emitting logs.
    ///
    /// This might be useful to hide certain fields in production logs as they may
    /// contain sensitive information, but allow them in testing environment.
    pub redact_keys: Vec<String>,

    /// Settings for rate limiting emission of log events
    pub rate_limit: RateLimitingSettings,

    /// Configure log volume metrics.
    pub log_volume_metrics: LogVolumeMetricSettings,
}

/// Log output destination.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, Default))]
pub enum LogOutput {
    /// Write log to terminal.
    #[default]
    Terminal,
    /// Write log to file with the specified path.
    ///
    /// File will be created if it doesn't exist and overwritten otherwise.
    File(PathBuf),
}

/// Format of the log output.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Default, Debug))]
#[derive(Copy)]
pub enum LogFormat {
    /// Plain text
    #[default]
    Text,
    /// JSON
    Json,
}

/// Verbosity level of the log.
#[derive(Clone, Debug, Copy)]
pub struct LogVerbosity(pub Level);

impl Default for LogVerbosity {
    fn default() -> Self {
        Self(Level::Info)
    }
}

impl Deref for LogVerbosity {
    type Target = Level;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Log volume metrics settings
///
/// If enabled, a counter metric will be exposed as <app_name>_bedrock_log_record_count
/// with a tag "level" indicating the log level.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, Default))]
pub struct LogVolumeMetricSettings {
    /// Whether to enable log volume metrics
    pub enabled: bool,
}

#[cfg(feature = "settings")]
mod with_settings_feature {
    use super::*;

    impl<'de> Deserialize<'de> for LogVerbosity {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            Level::from_str(&String::deserialize(deserializer)?)
                .map_err(|_| de::Error::custom("incorrect verbosity level"))
                .map(LogVerbosity)
        }
    }

    impl Serialize for LogVerbosity {
        fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            s.serialize_str(self.0.as_str())
        }
    }

    impl Settings for LogVerbosity {}
}

fn _assert_traits_implemented_for_all_features() {
    fn assert<S: std::fmt::Debug + Clone + Default>() {}

    assert::<LoggingSettings>();
}
