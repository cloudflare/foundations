use crate::telemetry::settings::rate_limit::RateLimitingSettings;
use crate::utils::feature_use;

use std::path::PathBuf;

feature_use!(cfg(feature = "settings"), {
    use crate::settings::settings;
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

/// Log verbosity levels which match 1:1 with [`slog::Level`].
#[cfg_attr(
    feature = "settings",
    settings(crate_path = "crate", impl_default = false)
)]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug))]
#[derive(Copy, Default)]
pub enum LogVerbosity {
    /// See [`slog::Level::Critical`].
    Critical,
    /// See [`slog::Level::Error`].
    Error,
    /// See [`slog::Level::Warning`].
    Warning,
    /// See [`slog::Level::Info`].
    #[default]
    Info,
    /// See [`slog::Level::Debug`].
    Debug,
    /// See [`slog::Level::Trace`].
    Trace,
}

impl From<slog::Level> for LogVerbosity {
    fn from(level: slog::Level) -> Self {
        match level {
            Level::Critical => Self::Critical,
            Level::Warning => Self::Warning,
            Level::Error => Self::Error,
            Level::Info => Self::Info,
            Level::Debug => Self::Debug,
            Level::Trace => Self::Trace,
        }
    }
}

impl From<LogVerbosity> for slog::Level {
    fn from(level: LogVerbosity) -> Self {
        match level {
            LogVerbosity::Critical => Self::Critical,
            LogVerbosity::Warning => Self::Warning,
            LogVerbosity::Error => Self::Error,
            LogVerbosity::Info => Self::Info,
            LogVerbosity::Debug => Self::Debug,
            LogVerbosity::Trace => Self::Trace,
        }
    }
}

/// Log volume metrics settings
///
/// If enabled, a counter metric will be exposed as <app_name>_foundations_log_record_count
/// with a tag "level" indicating the log level.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, Default))]
pub struct LogVolumeMetricSettings {
    /// Whether to enable log volume metrics
    pub enabled: bool,
}
