//! Logging-related functionality.

mod field_dedup;
mod field_filtering;
mod field_redact;
mod rate_limit;

pub(crate) mod init;

#[cfg(any(test, feature = "testing"))]
pub(crate) mod testing;

#[doc(hidden)]
pub mod internal;

#[cfg(feature = "metrics")]
pub mod log_volume;

use self::init::LogHarness;
use self::internal::current_log;
use crate::telemetry::log::init::build_log_with_drain;
use crate::telemetry::settings::LogVerbosity;
use crate::Result;
use log::Level as std_level;
use slog::{Level, Logger, OwnedKV};
use slog_scope::{set_global_logger, GlobalLoggerGuard};
use std::sync::Arc;

static mut GLOBAL_LOGGER_GUARD: Option<GlobalLoggerGuard> = None;
static GLOBAL_LOGGER_CAPTURE: parking_lot::Once = parking_lot::Once::new();

#[cfg(any(test, feature = "testing"))]
pub use self::testing::TestLogRecord;

/// Sets current log's verbosity, overriding the settings used in [`init`].
///
/// [`init`]: crate::telemetry::init
pub fn set_verbosity(level: Level) -> Result<()> {
    let harness = LogHarness::get();

    let mut settings = harness.settings.clone();
    settings.verbosity = LogVerbosity(level);

    let kv = OwnedKV(current_log().read().list().clone());
    let logger = build_log_with_drain(&settings, kv, Arc::clone(&harness.root_drain));
    *current_log().write() = logger;

    Ok(())
}

/// Gets the current log's verbosity.
pub fn verbosity() -> LogVerbosity {
    let harness = LogHarness::get();
    harness.settings.verbosity
}

/// Returns current log as a raw [slog] crate's `Logger` used by Foundations internally.
///
/// Can be used to propagate the logging context to libraries that don't use Foundations'
/// telemetry.
///
/// [slog]: https://crates.io/crates/slog
pub fn slog_logger() -> Arc<parking_lot::RwLock<Logger>> {
    current_log()
}

/// Capture logs from the [`log`](https://docs.rs/log/latest/log/) crate by forwarding them into
/// the current [`slog::Drain`]. Due to the intricacies and subtleties of this method, **you should
/// be very careful when using it**. Note also that this method can only be called once.
///
/// # Note
///
/// After calling this method, all `log` logs will be forwarded to the [`slog::Drain`] in use for
/// the rest of the program's lifetime.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::log::capture_global_log_logs;
/// use log::warn as log_warn;
///
/// let cx = TelemetryContext::test();
/// capture_global_log_logs();
/// for i in 0..16 {
///     log_warn!("{}", i);
/// }
///
/// assert_eq!(cx.log_records().len(), 16);
/// ```
pub fn capture_global_log_logs() {
    unsafe {
        // SAFETY: mutating a `static mut` is generally unsafe, but since we're guarding it behind
        // a call_once, we should be fine.
        GLOBAL_LOGGER_CAPTURE.call_once(|| {
            let curr_logger = Arc::clone(&slog_logger()).read().clone();
            let scope_guard = set_global_logger(curr_logger);

            // Convert slog::Level from Foundations settings to log::Level
            let normalized_level = match verbosity().0 {
                Level::Critical | Level::Error => std_level::Error,
                Level::Warning => std_level::Warn,
                Level::Info => std_level::Info,
                Level::Debug => std_level::Debug,
                Level::Trace => std_level::Trace,
            };

            slog_stdlog::init_with_level(normalized_level).unwrap();

            // Storing the scope guard in a static global guard means that logs will be forwarded
            // to the slog::Drain for the entirety of the program's lifetime. This prevents users
            // from accidentally calling the method twice and triggering log::SetLoggerErrors, or
            // attempting to log messages after dropping the guard and triggering
            // slog_scope::NoLoggerSet.
            GLOBAL_LOGGER_GUARD = Some(scope_guard);
        });
    }
}

// NOTE: `#[doc(hidden)]` + `#[doc(inline)]` for `pub use` trick is used to prevent these macros
// to show up in the crate's top level docs.

/// Adds fields to all the log records, making them context fields.
///
/// Calling the method with same field name multiple times updates the key value.
///
/// Certain added fields may not be present in the resulting logs if
/// [`LoggingSettings::redact_keys`] is used.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::log::{self, TestLogRecord};
/// use foundations::telemetry::settings::Level;
///
/// // Test context is used for demonstration purposes to show the resulting log records.
/// let ctx = TelemetryContext::test();
/// let _scope = ctx.scope();
///
/// log::warn!("Hello with one field"; "foo" => "bar");
///
/// log::add_fields!("ctx_field1" => 42, "ctx_field2" => "baz");
///
/// log::warn!("With context fields"; "foo" => "bar");
///
/// // Update the context field value
/// log::add_fields!("ctx_field1" => 43);
///
/// log::warn!("One more with context fields");
///
/// assert_eq!(*ctx.log_records(), &[
///     TestLogRecord {
///         level: Level::Warning,
///         message: "Hello with one field".into(),
///         fields: vec![("foo".into(), "bar".into())]
///     },
///     TestLogRecord {
///         level: Level::Warning,
///         message: "With context fields".into(),
///         fields: vec![
///             ("ctx_field2".into(), "baz".into()),
///             ("ctx_field1".into(), "42".into()),
///             ("foo".into(), "bar".into())
///         ]
///     },
///     TestLogRecord {
///         level: Level::Warning,
///         message: "One more with context fields".into(),
///         fields: vec![
///             ("ctx_field1".into(), "43".into()),
///             ("ctx_field2".into(), "baz".into()),
///         ]
///     }
/// ]);
/// ```
///
/// [`LoggingSettings::redact_keys`]: crate::telemetry::settings::LoggingSettings::redact_keys
#[macro_export]
#[doc(hidden)]
macro_rules! __add_fields {
    ( $($args:tt)* ) => {
        $crate::telemetry::log::internal::add_log_fields(
            $crate::reexports_for_macros::slog::o!($($args)*)
        );
    };
}

/// Log error level record.
///
/// If duplicate fields are specified for the record then the last one takes precedence and
/// overwrites the value of the previous one.
///
/// Certain added fields may not be present in the resulting logs if
/// [`LoggingSettings::redact_keys`] is used.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::log::{self, TestLogRecord};
/// use foundations::telemetry::settings::Level;
///
/// // Test context is used for demonstration purposes to show the resulting log records.
/// let ctx = TelemetryContext::test();
/// let _scope = ctx.scope();
///
/// // Simple log message
/// log::error!("Hello world!");
///
/// // Macro also accepts format arguments
/// log::error!("The values are: {}, {}", 42, true);
///
/// // Fields key-value pairs can be added to log record, by separating the format message
/// // and fields by `;`.
/// log::error!("Answer: {}", 42; "foo" => "bar", "baz" => 1337);
///
/// assert_eq!(*ctx.log_records(), &[
///     TestLogRecord {
///         level: Level::Error,
///         message: "Hello world!".into(),
///         fields: vec![]
///     },
///     TestLogRecord {
///         level: Level::Error,
///         message: "The values are: 42, true".into(),
///         fields: vec![]
///     },
///     TestLogRecord {
///         level: Level::Error,
///         message: "Answer: 42".into(),
///         fields: vec![
///             ("baz".into(), "1337".into()),
///             ("foo".into(), "bar".into())
///         ]
///     }
/// ]);
/// ```
///
/// [`LoggingSettings::redact_keys`]: crate::telemetry::settings::LoggingSettings::redact_keys
#[macro_export]
#[doc(hidden)]
macro_rules! __error {
    ( $($args:tt)+ ) => {
        $crate::reexports_for_macros::slog::error!(
            $crate::telemetry::log::internal::current_log().read(),
            $($args)+
        );
    };
}

/// Log warning level record.
///
/// If duplicate fields are specified for the record then the last one takes precedence and
/// overwrites the value of the previous one.
///
/// Certain added fields may not be present in the resulting logs if
/// [`LoggingSettings::redact_keys`] is used.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::log::{self, TestLogRecord};
/// use foundations::telemetry::settings::Level;
///
/// // Test context is used for demonstration purposes to show the resulting log records.
/// let ctx = TelemetryContext::test();
/// let _scope = ctx.scope();
///
/// // Simple log message
/// log::warn!("Hello world!");
///
/// // Macro also accepts format arguments
/// log::warn!("The values are: {}, {}", 42, true);
///
/// // Fields key-value pairs can be added to log record, by separating the format message
/// // and fields by `;`.
/// log::warn!("Answer: {}", 42; "foo" => "bar", "baz" => 1337);
///
/// assert_eq!(*ctx.log_records(), &[
///     TestLogRecord {
///         level: Level::Warning,
///         message: "Hello world!".into(),
///         fields: vec![]
///     },
///     TestLogRecord {
///         level: Level::Warning,
///         message: "The values are: 42, true".into(),
///         fields: vec![]
///     },
///     TestLogRecord {
///         level: Level::Warning,
///         message: "Answer: 42".into(),
///         fields: vec![
///             ("baz".into(), "1337".into()),
///             ("foo".into(), "bar".into())
///         ]
///     }
/// ]);
/// ```
///
/// [`LoggingSettings::redact_keys`]: crate::telemetry::settings::LoggingSettings::redact_keys
#[doc(hidden)]
#[macro_export]
macro_rules! __warn {
    ( $($args:tt)+ ) => {
        $crate::reexports_for_macros::slog::warn!(
            $crate::telemetry::log::internal::current_log().read(),
            $($args)+
        );
    };
}

/// Log debug level record.
///
/// If duplicate fields are specified for the record then the last one takes precedence and
/// overwrites the value of the previous one.
///
/// Certain added fields may not be present in the resulting logs if
/// [`LoggingSettings::redact_keys`] is used.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::log::{self, TestLogRecord};
/// use foundations::telemetry::settings::Level;
///
/// // Test context is used for demonstration purposes to show the resulting log records.
/// let ctx = TelemetryContext::test();
/// let _scope = ctx.scope();
///
/// // Simple log message
/// log::debug!("Hello world!");
///
/// // Macro also accepts format arguments
/// log::debug!("The values are: {}, {}", 42, true);
///
/// // Fields key-value pairs can be added to log record, by separating the format message
/// // and fields by `;`.
/// log::debug!("Answer: {}", 42; "foo" => "bar", "baz" => 1337);
///
/// assert_eq!(*ctx.log_records(), &[
///     TestLogRecord {
///         level: Level::Debug,
///         message: "Hello world!".into(),
///         fields: vec![]
///     },
///     TestLogRecord {
///         level: Level::Debug,
///         message: "The values are: 42, true".into(),
///         fields: vec![]
///     },
///     TestLogRecord {
///         level: Level::Debug,
///         message: "Answer: 42".into(),
///         fields: vec![
///             ("baz".into(), "1337".into()),
///             ("foo".into(), "bar".into())
///         ]
///     }
/// ]);
/// ```
///
/// [`LoggingSettings::redact_keys`]: crate::telemetry::settings::LoggingSettings::redact_keys
#[macro_export]
#[doc(hidden)]
macro_rules! __debug {
    ( $($args:tt)+ ) => {
        $crate::reexports_for_macros::slog::debug!(
            $crate::telemetry::log::internal::current_log().read(),
            $($args)+
        );
    };
}

/// Log info level record.
///
/// If duplicate fields are specified for the record then the last one takes precedence and
/// overwrites the value of the previous one.
///
/// Certain added fields may not be present in the resulting logs if
/// [`LoggingSettings::redact_keys`] is used.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::log::{self, TestLogRecord};
/// use foundations::telemetry::settings::Level;
///
/// // Test context is used for demonstration purposes to show the resulting log records.
/// let ctx = TelemetryContext::test();
/// let _scope = ctx.scope();
///
/// // Simple log message
/// log::info!("Hello world!");
///
/// // Macro also accepts format arguments
/// log::info!("The values are: {}, {}", 42, true);
///
/// // Fields key-value pairs can be added to log record, by separating the format message
/// // and fields by `;`.
/// log::info!("Answer: {}", 42; "foo" => "bar", "baz" => 1337);
///
/// assert_eq!(*ctx.log_records(), &[
///     TestLogRecord {
///         level: Level::Info,
///         message: "Hello world!".into(),
///         fields: vec![]
///     },
///     TestLogRecord {
///         level: Level::Info,
///         message: "The values are: 42, true".into(),
///         fields: vec![]
///     },
///     TestLogRecord {
///         level: Level::Info,
///         message: "Answer: 42".into(),
///         fields: vec![
///             ("baz".into(), "1337".into()),
///             ("foo".into(), "bar".into())
///         ]
///     }
/// ]);
/// ```
///
/// [`LoggingSettings::redact_keys`]: crate::telemetry::settings::LoggingSettings::redact_keys
#[macro_export]
#[doc(hidden)]
macro_rules! __info {
    ( $($args:tt)+ ) => {
        $crate::reexports_for_macros::slog::info!(
            $crate::telemetry::log::internal::current_log().read(),
            $($args)+
        );
    };
}

/// Log trace level record.
///
/// If duplicate fields are specified for the record then the last one takes precedence and
/// overwrites the value of the previous one.
///
/// Certain added fields may not be present in the resulting logs if
/// [`LoggingSettings::redact_keys`] is used.
///
/// # Examples
/// ```
/// use foundations::telemetry::TelemetryContext;
/// use foundations::telemetry::log::{self, TestLogRecord};
/// use foundations::telemetry::settings::Level;
///
/// // Test context is used for demonstration purposes to show the resulting log records.
/// let ctx = TelemetryContext::test();
/// let _scope = ctx.scope();
///
/// // Simple log message
/// log::trace!("Hello world!");
///
/// // Macro also accepts format arguments
/// log::trace!("The values are: {}, {}", 42, true);
///
/// // Fields key-value pairs can be added to log record, by separating the format message
/// // and fields by `;`.
/// log::trace!("Answer: {}", 42; "foo" => "bar", "baz" => 1337);
///
/// assert_eq!(*ctx.log_records(), &[
///     TestLogRecord {
///         level: Level::Trace,
///         message: "Hello world!".into(),
///         fields: vec![]
///     },
///     TestLogRecord {
///         level: Level::Trace,
///         message: "The values are: 42, true".into(),
///         fields: vec![]
///     },
///     TestLogRecord {
///         level: Level::Trace,
///         message: "Answer: 42".into(),
///         fields: vec![
///             ("baz".into(), "1337".into()),
///             ("foo".into(), "bar".into())
///         ]
///     }
/// ]);
/// ```
///
/// [`LoggingSettings::redact_keys`]: crate::telemetry::settings::LoggingSettings::redact_keys
#[macro_export]
#[doc(hidden)]
macro_rules! __trace {
    ( $($args:tt)+ ) => {
        $crate::reexports_for_macros::slog::trace!(
            $crate::telemetry::log::internal::current_log().read(),
            $($args)+
        );
    };
}

#[doc(inline)]
pub use {
    __add_fields as add_fields, __debug as debug, __error as error, __info as info,
    __trace as trace, __warn as warn,
};
