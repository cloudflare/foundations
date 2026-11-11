use crate::telemetry::settings::rate_limit::RateLimitingSettings;
use crate::utils::feature_use;

use std::path::PathBuf;

feature_use!(cfg(feature = "logging"), {
    use slog::{Never, SendSyncRefUnwindSafeDrain};
    use std::sync::Arc;

    // NOTE: we technically don't need a feature gate here, but if we don't add
    // it then docs don't mark this re-export as available on when `logging` is
    // enabled.
    pub use slog::Level;

    /// A custom slog drain for use with [`LogOutput::Custom`].
    pub type CustomDrain = Arc<dyn SendSyncRefUnwindSafeDrain<Ok = (), Err = Never>>;
});

feature_use!(cfg(feature = "settings"), {
    use crate::settings::settings;
});

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

    /// Whether to ignore I/O errors from built-in log outputs.
    ///
    /// By default, I/O errors from terminal, stderr, and file outputs are treated
    /// as fatal logging failures. Enabling this setting allows services to keep
    /// running if a log sink returns an error such as [`std::io::ErrorKind::BrokenPipe`].
    ///
    /// A systemd journal stream with no room for a record isn't an I/O error: the record is
    /// dropped and counted in the `logging_records_dropped` metric, and logging carries on.
    pub ignore_io_errors: bool,

    /// Whether to let stdout and stderr block when they are the systemd journal stream.
    ///
    /// On Linux, logs written to the journal stream that systemd names in `JOURNAL_STREAM` never
    /// wait for journald, whatever this setting is. Records that don't fit in the stream's buffer
    /// are dropped and counted in the `logging_records_dropped` metric. Other writes to the
    /// stream can wait, though, and if journald stops reading it for good, they wait for good.
    /// That includes the default panic hook's message, so a thread that panics never finishes.
    /// By default, foundations makes them nonblocking to prevent that, whatever the log output is.
    ///
    /// Nonblocking mode belongs to the stream, so it applies to every writer in the process and
    /// in child processes that inherit the stream. When the stream's buffer is full, the default
    /// panic hook's message can be lost, and `println!` and `eprintln!` panic instead of waiting.
    /// Enabling this setting leaves the streams as they are; it doesn't make a nonblocking stream
    /// blocking again. A stream stays nonblocking until the service restarts and systemd gives it
    /// a new one, including when a process that inherits the stream enables this setting.
    ///
    /// If seccomp sandboxing applies to the thread that initializes telemetry, the logging thread
    /// needs `sendto` to write to the journal stream. `SERVICE_BASICS` doesn't allow it, and
    /// `NET_SOCKET_API` does. Initialization also needs `fcntl` unless this setting is enabled,
    /// and `RUST_BASICS` only allows `fcntl` in debug builds.
    pub allow_blocking_journal_stdio: bool,

    /// Settings for rate limiting emission of log events
    pub rate_limit: RateLimitingSettings,

    /// Configure log volume metrics.
    pub log_volume_metrics: LogVolumeMetricSettings,
}

/// Log output destination.
#[cfg_attr(
    feature = "settings",
    settings(crate_path = "crate", impl_debug = false)
)]
#[cfg_attr(not(feature = "settings"), derive(Clone, Default))]
pub enum LogOutput {
    /// Write log to terminal.
    #[default]
    Terminal,
    /// Write log to [`std::io::Stderr`].
    Stderr,
    /// Write log to file with the specified path.
    ///
    /// File will be created if it doesn't exist and overwritten otherwise.
    File(PathBuf),

    ///Install a logging drain that forwards to `tracing-rs`
    ///
    ///WARN: If this output format is used, the settings in [`LoggingSettings`] other than the
    ///verbosity will not be respected
    #[cfg(feature = "tracing-rs-compat")]
    TracingRsCompat,

    /// User-provided drain. Not serializable — set programmatically only.
    ///
    /// [`LogFormat`] is ignored for this variant; the custom drain is responsible for its
    /// own formatting. All other [`LoggingSettings`] (verbosity, field redaction, rate
    /// limiting) still apply.
    ///
    /// # Examples
    ///
    /// Combine a terminal drain with a JSON file drain:
    ///
    /// ```ignore
    /// use slog::{Drain, Duplicate};
    /// use slog_term::{FullFormat, TermDecorator};
    /// use slog_json::Json;
    /// use std::fs::File;
    /// use std::sync::Arc;
    ///
    /// let term = FullFormat::new(TermDecorator::new().build()).build().fuse();
    /// let file = File::create("/var/log/app.json").unwrap();
    /// let json = Json::new(file).build().fuse();
    /// let combined = Duplicate::new(term, json).fuse();
    ///
    /// settings.logging.output = LogOutput::Custom(Arc::new(combined));
    /// ```
    #[cfg(feature = "logging")]
    #[cfg_attr(feature = "settings", serde(skip))]
    Custom(CustomDrain),
}

impl std::fmt::Debug for LogOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Terminal => write!(f, "Terminal"),
            Self::Stderr => write!(f, "Stderr"),
            Self::File(path) => f.debug_tuple("File").field(path).finish(),
            #[cfg(feature = "tracing-rs-compat")]
            Self::TracingRsCompat => write!(f, "TracingRsCompat"),
            #[cfg(feature = "logging")]
            Self::Custom(_) => write!(f, "Custom(...)"),
        }
    }
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
    #[cfg_attr(feature = "settings", serde(rename = "CRITICAL"))]
    Critical,
    /// See [`slog::Level::Error`].
    #[cfg_attr(feature = "settings", serde(rename = "ERROR"))]
    Error,
    /// See [`slog::Level::Warning`].
    #[cfg_attr(feature = "settings", serde(rename = "WARN"))]
    Warning,
    /// See [`slog::Level::Info`].
    #[default]
    #[cfg_attr(feature = "settings", serde(rename = "INFO"))]
    Info,
    /// See [`slog::Level::Debug`].
    #[cfg_attr(feature = "settings", serde(rename = "DEBUG"))]
    Debug,
    /// See [`slog::Level::Trace`].
    #[cfg_attr(feature = "settings", serde(rename = "TRACE"))]
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
