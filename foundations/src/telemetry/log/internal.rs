use super::init::LogHarness;
use crate::telemetry::scope::Scope;
use slog::{Logger, OwnedKV, SendSyncRefUnwindSafeKV};
use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug)]
pub struct LoggerWithNestingTracking {
    pub(crate) inner: Logger,
    pub(crate) nesting_level: u32,
}

impl LoggerWithNestingTracking {
    pub(crate) fn inc_nesting(&mut self) {
        const MAX_LOG_NESTING: u32 = 1000;

        self.nesting_level += 1;

        if self.nesting_level >= MAX_LOG_NESTING {
            panic!(
                "foundations: maximum logger generation exceeded (are add_fields! or set_verbosity being called in a loop?)"
            );
        }
    }
}

impl Deref for LoggerWithNestingTracking {
    type Target = Logger;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

// NOTE: we intentionally use a lock without poisoning here to not
// panic the threads if they just share telemetry with failed thread.
pub(crate) type SharedLog = Arc<parking_lot::RwLock<LoggerWithNestingTracking>>;

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

    log_lock.inc_nesting();
    log_lock.inner = log_lock.inner.new(fields);
}

pub fn current_log() -> SharedLog {
    let harness = LogHarness::get();
    let log = harness.log_scope_stack.current();

    log.unwrap_or_else(|| Arc::clone(&harness.root_log))
}

pub(crate) fn fork_log() -> SharedLog {
    let parent = current_log();
    let parent_lock = parent.read();

    let log = LoggerWithNestingTracking {
        inner: parent_lock.new(slog::o!()),
        nesting_level: parent_lock.nesting_level,
    };

    Arc::new(parking_lot::RwLock::new(log))
}
