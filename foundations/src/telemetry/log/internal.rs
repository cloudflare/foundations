use super::init::LogHarness;
use crate::telemetry::scope::Scope;
use slog::{Logger, OwnedKV, SendSyncRefUnwindSafeKV};
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
}

impl LoggerWithKvNestingTracking {
    pub const MAX_NESTING: u32 = 1000;
    pub const EXCEEDED_MAX_NESTING_ERROR: &'static str = "foundations: maximum logger KV nesting exceeded (are add_fields! or set_verbosity being called in a loop?)";

    // Create a new LoggerWithKvNestingTracking based on a fresh logger. The KV nesting level is
    // initialized to zero.
    pub(crate) fn new(logger: Logger) -> Self {
        Self {
            inner: logger,
            nesting_level: 0,
        }
    }

    // Increment the KV nesting level. You should call this every time you have replaced the
    // logger with a child of itself.
    pub(crate) fn inc_nesting_level(&mut self) {
        self.nesting_level += 1;

        assert!(
            self.nesting_level < Self::MAX_NESTING,
            "{}",
            Self::EXCEEDED_MAX_NESTING_ERROR
        );
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
pub(crate) struct LogScope(#[allow(dead_code)] Scope<SharedLog>);

impl LogScope {
    #[inline]
    pub(crate) fn new(log: SharedLog) -> Self {
        Self(Scope::new(&LogHarness::get().log_scope_stack, log))
    }
}

pub fn add_log_fields<T>(fields: OwnedKV<T>)
where
    T: SendSyncRefUnwindSafeKV + 'static,
{
    let log = current_log();
    let mut log_lock = log.write();

    log_lock.inner = log_lock.inner.new(fields);
    log_lock.inc_nesting_level();
}

pub fn current_log() -> SharedLog {
    let harness = LogHarness::get();
    let log = harness.log_scope_stack.current();

    log.unwrap_or_else(|| Arc::clone(&harness.root_log))
}

pub(crate) fn fork_log() -> SharedLog {
    let parent = current_log();
    let log = parent.read().clone();

    Arc::new(parking_lot::RwLock::new(log))
}
