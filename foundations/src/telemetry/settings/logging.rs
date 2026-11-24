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
    ///
    /// Defaults to `terminal`.
    pub output: LogOutput,

    /// The format to use for log messages.
    ///
    /// Defaults to `text`.
    pub format: LogFormat,

    /// Set the logging verbosity level.
    ///
    /// Defaults to `INFO`.
    pub verbosity: LogVerbosity,

    /// A list of field keys to redact when emitting logs.
    ///
    /// This might be useful to hide certain fields in production logs as they may
    /// contain sensitive information, but allow them in testing environment.
    ///
    /// Defaults to an empty list.
    pub redact_keys: Vec<String>,

    /// Whether to ignore I/O errors from built-in log outputs.
    ///
    /// By default, I/O errors from terminal, stderr, and file outputs are treated
    /// as fatal logging failures. Enabling this setting allows services to keep
    /// running if a log sink returns an error such as [`std::io::ErrorKind::BrokenPipe`].
    ///
    /// When [`JournalStreamBehavior::NonBlockingLogger`] or
    /// [`JournalStreamBehavior::NonBlockingStdio`] is configured, a stalled systemd journal stream
    /// isn't an I/O error: the record is dropped and counted in the `logging_records_dropped`
    /// metric, and logging carries on.
    ///
    /// Defaults to `false`.
    pub ignore_io_errors: bool,

    /// How to handle stdout and stderr when they are the systemd journal stream on Linux.
    ///
    /// Defaults to `blocking`.
    pub journal_stream_behavior: JournalStreamBehavior,

    /// Settings for rate limiting emission of log events.
    ///
    /// Rate limiting is disabled by default.
    pub rate_limit: RateLimitingSettings,

    /// Configure log volume metrics.
    ///
    /// Log volume metrics are disabled by default.
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
    /// Whether to enable log volume metrics.
    ///
    /// Defaults to `false`.
    pub enabled: bool,
}

/// How foundations handles stdout and stderr when they are the systemd journal stream.
///
/// On Linux, systemd connects a service's stdout and stderr to a journald socket and names it in
/// `JOURNAL_STREAM`. If journald stops reading that socket while keeping it open, the socket's
/// send buffer fills and blocking writes wait indefinitely.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Default, Debug))]
#[derive(Copy, PartialEq, Eq)]
pub enum JournalStreamBehavior {
    /// Leave the systemd journal stream untouched.
    ///
    /// Writes to stdout and stderr use the standard blocking writers and no extra syscalls
    /// (`fstat`, `sendto`, `poll`/`ppoll`, `fcntl`) are issued for journal detection or
    /// nonblocking I/O.
    #[default]
    Blocking,

    /// Never let the foundations logging thread hang indefinitely on a stalled journal stream.
    ///
    /// On Linux, when [`LoggingSettings::output`] targets the stream named in `JOURNAL_STREAM`,
    /// log records are buffered and sent with `MSG_DONTWAIT`. On `EAGAIN`, the logging thread
    /// waits briefly for journald to drain the socket so normal bursts and records larger than
    /// the socket buffer aren't dropped or truncated. Once the stream stays full past that wait,
    /// it is treated as stalled and subsequent records are dropped immediately and counted in
    /// `logging_records_dropped{reason="journal_full"}` until the stream accepts bytes again.
    ///
    /// Descriptor flags (`O_NONBLOCK`) are left unchanged, so `println!`, `eprintln!`, and child
    /// processes are unaffected.
    ///
    /// If seccomp sandboxing applies to the thread that initializes telemetry, the logging thread
    /// needs `sendto` (`NET_SOCKET_API` allows it; `SERVICE_BASICS` does not) and `poll` (x86_64)
    /// or `ppoll` (aarch64).
    NonBlockingLogger,

    /// Same as [`JournalStreamBehavior::NonBlockingLogger`], and also sets `O_NONBLOCK` on
    /// stdout and stderr during telemetry initialization when they are the journal stream,
    /// whatever [`LoggingSettings::output`] is.
    ///
    /// This also prevents other writers — including Rust's default panic hook writing to stderr —
    /// from hanging forever on a stalled journal. Because `O_NONBLOCK` belongs to the underlying
    /// open file description, it applies to every writer in the process and in child processes
    /// that inherit the stream: when the stream's buffer is full, `println!` and `eprintln!`
    /// panic instead of waiting, the default panic hook's message can be lost, and child
    /// processes may receive `EAGAIN`. The flag stays on the stream until the service restarts
    /// and systemd gives it a new one.
    ///
    /// In addition to the syscalls required by [`JournalStreamBehavior::NonBlockingLogger`],
    /// telemetry initialization needs `fcntl` (`RUST_BASICS` only allows `fcntl` in debug builds).
    NonBlockingStdio,
}
