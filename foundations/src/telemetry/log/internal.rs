use super::init::LogHarness;
use crate::telemetry::scope::Scope;
use slog::{
    KV, Key, Level, Logger, OwnedKV, OwnedKVList, Record, SendSyncRefUnwindSafeKV, Serializer,
};
use std::collections::HashSet;
use std::fmt::{Arguments, Debug};
use std::ops::Deref;
use std::sync::Arc;

// NOTE: we intentionally use a lock without poisoning here to not
// panic the threads if they just share telemetry with failed thread.
pub(crate) type SharedLog = Arc<parking_lot::RwLock<LoggerWithKvNestingTracking>>;

#[derive(Debug, Clone)]
pub struct LoggerWithKvNestingTracking {
    // The logger itself. This is the most important part of this struct. (We implement Deref to
    // let you go straight to this field, in contexts where you need a &Logger)
    pub(crate) inner: Logger,

    // KV nesting level. You should increment this (using the inc_nesting_level() method) every
    // time you replace the logger with a child of itself. You should likewise set this back to
    // zero if you replace the logger with a "root" logger that doesn't have any nested KVs in it.
    // (That said, accuracy is not critical, as this is only used as a safety check)
    pub(crate) nesting_level: u32,

    // Whether the logger is frozen. When frozen, any attempt to mutate the logger (via
    // add_fields! or set_verbosity) is detected and causes a panic by default, or is logged as
    // an error with a backtrace if the `panic_on_frozen_logger` feature is disabled.
    //
    // Forked loggers always start unfrozen, regardless of the parent's frozen state.
    pub(crate) frozen: bool,

    // Whether add_fields! panics when it sets a key that the logger already has. Set from
    // `LoggingSettings::panic_on_field_overwrite`, and inherited by forked loggers.
    pub(crate) panic_on_field_overwrite: bool,
}

impl LoggerWithKvNestingTracking {
    pub const MAX_NESTING: u32 = 1000;
    pub const EXCEEDED_MAX_NESTING_ERROR: &'static str = "foundations: maximum logger KV nesting exceeded (are add_fields! or set_verbosity being called in a loop?)";

    pub const FROZEN_LOGGER_ERROR: &'static str = "foundations: attempt to modify a frozen logger \
         (are you modifying the root logger from a spawned task/thread that lacks a forked context?)";

    pub const FIELD_OVERWRITE_ERROR: &'static str = "foundations: add_fields! overwrote an existing \
         log field (use `add_fields!(allow_overwrite; ...)` if this is intended)";

    /// Create a new LoggerWithKvNestingTracking based on a fresh logger. The KV nesting level is
    /// initialized to zero and the logger starts unfrozen.
    pub(crate) fn new(logger: Logger) -> Self {
        Self {
            inner: logger,
            nesting_level: 0,
            frozen: false,
            panic_on_field_overwrite: false,
        }
    }

    /// Check whether the logger is frozen before performing a mutation.
    ///
    /// If the logger is not frozen, returns `Some(lock_guard)` and the caller may proceed.
    ///
    /// If the logger is frozen and `panic_on_frozen_logger` is enabled, the lock is dropped and
    /// this function panics. Otherwise it logs a `slog::error!` with a backtrace and returns
    /// `None`, causing the caller to skip the mutation.
    pub(crate) fn check_frozen(
        current_log_lock: parking_lot::lock_api::RwLockWriteGuard<
            parking_lot::RawRwLock,
            LoggerWithKvNestingTracking,
        >,
    ) -> Option<
        parking_lot::lock_api::RwLockWriteGuard<
            parking_lot::RawRwLock,
            LoggerWithKvNestingTracking,
        >,
    > {
        if !current_log_lock.frozen {
            return Some(current_log_lock);
        }

        if cfg!(feature = "panic_on_frozen_logger") {
            drop(current_log_lock);
            panic!("{}", Self::FROZEN_LOGGER_ERROR);
        } else {
            slog::error!(current_log_lock, "{}", Self::FROZEN_LOGGER_ERROR; "backtrace" => std::backtrace::Backtrace::capture().to_string());
            None
        }
    }

    /// Increment the KV nesting level. You should call this before any time you're going to replace the
    /// logger with a child of itself.
    ///
    /// If this returns None, it will consume the logger lock, and you should not nest any further.
    /// If panic_on_too_much_logger_nesting is enabled, instead of returning None this will free the
    /// logger lock and then panic.
    pub(crate) fn check_nesting_level(
        mut current_log_lock: parking_lot::lock_api::RwLockWriteGuard<
            parking_lot::RawRwLock,
            LoggerWithKvNestingTracking,
        >,
    ) -> Option<
        parking_lot::lock_api::RwLockWriteGuard<
            parking_lot::RawRwLock,
            LoggerWithKvNestingTracking,
        >,
    > {
        current_log_lock.nesting_level = current_log_lock.nesting_level.saturating_add(1);

        match current_log_lock.nesting_level {
            0..Self::MAX_NESTING => Some(current_log_lock), // continue with operation
            Self::MAX_NESTING => {
                // Drop the lock guard before panicking
                if cfg!(feature = "panic_on_too_much_logger_nesting") {
                    drop(current_log_lock);
                    panic!("{}", Self::EXCEEDED_MAX_NESTING_ERROR);
                } else {
                    slog::error!(current_log_lock, "{}", Self::EXCEEDED_MAX_NESTING_ERROR; "backtrace"=> std::backtrace::Backtrace::capture().to_string());
                    None // avoid further nesting
                }
            }
            _ => None, // avoid further nesting
        }
    }
}

impl Deref for LoggerWithKvNestingTracking {
    type Target = Logger;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[must_use]
pub(crate) struct LogScope {
    _inner: Scope<SharedLog>,
}

impl LogScope {
    #[inline]
    pub(crate) fn new(log: SharedLog) -> Self {
        Self {
            _inner: Scope::new(&LogHarness::get().log_scope_stack, log),
        }
    }
}

pub fn add_log_fields<T>(fields: OwnedKV<T>, allow_overwrite: bool)
where
    T: SendSyncRefUnwindSafeKV + 'static,
{
    let log = current_log();
    let log_lock = log.write();

    let Some(log_lock) = LoggerWithKvNestingTracking::check_frozen(log_lock) else {
        return; // logger is frozen, mutation rejected
    };

    if log_lock.panic_on_field_overwrite
        && !allow_overwrite
        && let Some(key) = overwritten_key(log_lock.list(), &fields.0)
    {
        drop(log_lock);
        panic!(
            "{} (key: `{key}`)",
            LoggerWithKvNestingTracking::FIELD_OVERWRITE_ERROR
        );
    }

    let Some(mut log_lock) = LoggerWithKvNestingTracking::check_nesting_level(log_lock) else {
        return; // avoid changes, nesting level was beyond threshold
    };

    log_lock.inner = log_lock.inner.new(fields);
}

/// Returns the first key in `fields` that is already in `existing`.
fn overwritten_key(existing: &OwnedKVList, fields: &dyn KV) -> Option<String> {
    #[derive(Default)]
    struct KeyCollector(Vec<String>);

    impl Serializer for KeyCollector {
        fn emit_arguments(&mut self, key: Key, _val: &Arguments) -> slog::Result {
            self.0.push(key.to_string());
            Ok(())
        }
    }

    let keys = |kv: &dyn KV| {
        let mut keys = KeyCollector::default();
        // `KV::serialize` needs a record, but only the keys are collected here.
        let _ = kv.serialize(
            &slog::record!(Level::Info, "", &format_args!(""), slog::b!()),
            &mut keys,
        );
        keys.0
    };

    let existing: HashSet<String> = keys(existing).into_iter().collect();
    keys(fields).into_iter().find(|key| existing.contains(key))
}

pub fn current_log() -> SharedLog {
    let harness = LogHarness::get();
    let log = harness.log_scope_stack.current();

    log.unwrap_or_else(|| Arc::clone(&harness.root_log))
}

pub(crate) fn fork_log() -> SharedLog {
    let parent = current_log();
    let mut log = parent.read().clone();

    // Forked loggers always start unfrozen, regardless of the parent's frozen state.
    log.frozen = false;

    Arc::new(parking_lot::RwLock::new(log))
}

/// Logs a [`Record`] directly to the root drain. Unlike our regular logger, this function
/// does not filter by level and does not panic on errors (it returns them instead.)
#[allow(dead_code, reason = "only used by some features")]
pub(crate) fn log_to_drain(record: &Record) -> Result<(), Box<dyn Debug>> {
    let harness = LogHarness::get();
    let root_drain = &harness.root_drain;
    let shared_logger = current_log();

    let logger = shared_logger.read();
    root_drain.log(record, logger.list())
}

/// Freezes the current logger. Any subsequent call to [`add_log_fields`] or `set_verbosity`
/// will be rejected: by default a panic is raised; if the `panic_on_frozen_logger` feature is
/// disabled, an error is logged with a backtrace instead.
pub(crate) fn freeze() {
    current_log().write().frozen = true;
}

/// Unfreezes the current logger, re-enabling mutations.
pub(crate) fn unfreeze() {
    current_log().write().frozen = false;
}

/// Returns `true` if the current logger is frozen.
pub(crate) fn is_frozen() -> bool {
    current_log().read().frozen
}
