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
use slog::{Logger, OwnedKV};
use std::ops::Deref;
use std::sync::Arc;

#[cfg(any(test, feature = "testing"))]
pub use self::testing::TestLogRecord;

/// Sets current log's verbosity, overriding the settings used in [`init`].
///
/// For reasons related to the current implementation of `set_verbosity()`, there is a danger of
/// stack overflow if it is called an extremely large number of times on the same logger. To
/// protect against the possibility of stack overflow, there is an internal counter which will
/// trigger a panic if a limit of (currently) 1000 calls on a single logger is reached.
///
/// To avoid this panic, only call `set_verbosity()` when there is an actual change to the
/// verbosity level.
///
/// [`init`]: crate::telemetry::init
pub fn set_verbosity(verbosity: LogVerbosity) -> Result<()> {
    let harness = LogHarness::get();

    let mut settings = harness.settings.clone();
    settings.verbosity = verbosity;

    let current_log = current_log();
    let mut current_log_lock = current_log.write();

    let kv = OwnedKV(current_log_lock.list().clone());
    current_log_lock.inner = build_log_with_drain(&settings, kv, Arc::clone(&harness.root_drain));
    if current_log_lock.has_too_much_nesting() {
        // Drop the lock guard before panicking
        drop(current_log_lock);
        crate::telemetry::log::internal::LoggerWithKvNestingTracking::panic_from_too_much_nesting();
    }

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
pub fn slog_logger() -> Arc<parking_lot::RwLock<impl Deref<Target = Logger>>> {
    current_log()
}

// NOTE: `#[doc(hidden)]` + `#[doc(inline)]` for `pub use` trick is used to prevent these macros
// to show up in the crate's top level docs.

/// Adds fields to all the log records, making them context fields.
///
/// Calling the method with same field name multiple times updates the key value. There is a small
/// cost in performance if large numbers of the same field are added, which then must be
/// deduplicated at runtime. For that reason, as well as the fact that there is a danger of stack
/// overflow if `add_fields!` is called an extremely large number of times on the same logger,
/// there is an internal counter which will trigger a panic if a limit of (currently) 1000 calls on
/// a single logger is reached.
///
/// To avoid this panic, make sure to only use `add_fields!` for fields that will remain relatively
/// static (under 1000 updates over the lifetime of any given logger).
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
