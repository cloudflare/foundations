mod field_dedup;
mod field_filtering;
mod field_redact;

pub(crate) mod init;

#[cfg(any(test, feature = "testing"))]
pub(crate) mod testing;

pub mod settings;

#[doc(hidden)]
pub mod internal;

use self::init::{build_log, LogHarness};
use self::internal::current_log;
use self::settings::LogVerbosity;
use crate::Result;
use slog::{Level, Logger, OwnedKV};

#[cfg(any(test, feature = "testing"))]
pub use self::testing::TestLogRecord;

/// Sets current log's verbosity, overriding the settings used in [bedrock::telemetry::init].
pub fn set_verbosity(level: Level) -> Result<()> {
    let mut settings = LogHarness::get().settings.clone();

    settings.verbosity = LogVerbosity(level);

    let kv = OwnedKV(current_log().read().list().clone());

    // NOTE: it's ok to pass an empty string as a version here, log key
    // for it will be copied over from the current log.
    let drain = build_log(&settings, "".into())?;

    *current_log().write() = Logger::root(drain, kv);

    Ok(())
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
/// use bedrock::telemetry::TelemetryContext;
/// use bedrock::telemetry::log::{self, TestLogRecord};
/// use bedrock::telemetry::log::settings::Level;
///
/// // Test scope is used for demonstration purposes to show the resulting log records.
/// let scope = TelemetryContext::test();
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
/// assert_eq!(*scope.log_records(), &[
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
/// [`LoggingSettings::redact_keys`]: crate::telemetry::log::settings::LoggingSettings::redact_keys
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
/// use bedrock::telemetry::TelemetryContext;
/// use bedrock::telemetry::log::{self, TestLogRecord};
/// use bedrock::telemetry::log::settings::Level;
///
/// // Test scope is used for demonstration purposes to show the resulting log records.
/// let scope = TelemetryContext::test();
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
/// assert_eq!(*scope.log_records(), &[
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
/// [`LoggingSettings::redact_keys`]: crate::telemetry::log::settings::LoggingSettings::redact_keys
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
/// use bedrock::telemetry::TelemetryContext;
/// use bedrock::telemetry::log::{self, TestLogRecord};
/// use bedrock::telemetry::log::settings::Level;
///
/// // Test scope is used for demonstration purposes to show the resulting log records.
/// let scope = TelemetryContext::test();
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
/// assert_eq!(*scope.log_records(), &[
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
/// [`LoggingSettings::redact_keys`]: crate::telemetry::log::settings::LoggingSettings::redact_keys
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
/// use bedrock::telemetry::TelemetryContext;
/// use bedrock::telemetry::log::{self, TestLogRecord};
/// use bedrock::telemetry::log::settings::Level;
///
/// // Test scope is used for demonstration purposes to show the resulting log records.
/// let scope = TelemetryContext::test();
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
/// assert_eq!(*scope.log_records(), &[
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
/// [`LoggingSettings::redact_keys`]: crate::telemetry::log::settings::LoggingSettings::redact_keys
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
/// use bedrock::telemetry::TelemetryContext;
/// use bedrock::telemetry::log::{self, TestLogRecord};
/// use bedrock::telemetry::log::settings::Level;
///
/// // Test scope is used for demonstration purposes to show the resulting log records.
/// let scope = TelemetryContext::test();
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
/// assert_eq!(*scope.log_records(), &[
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
/// [`LoggingSettings::redact_keys`]: crate::telemetry::log::settings::LoggingSettings::redact_keys
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
/// use bedrock::telemetry::TelemetryContext;
/// use bedrock::telemetry::log::{self, TestLogRecord};
/// use bedrock::telemetry::log::settings::Level;
///
/// // Test scope is used for demonstration purposes to show the resulting log records.
/// let scope = TelemetryContext::test();
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
/// assert_eq!(*scope.log_records(), &[
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
/// [`LoggingSettings::redact_keys`]: crate::telemetry::log::settings::LoggingSettings::redact_keys
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
