use super::init::LogHarness;
use crate::telemetry::scope::Scope;
use slog::{Logger, OwnedKV, SendSyncRefUnwindSafeKV};
use std::sync::Arc;

// NOTE: we intentionally use a lock without poisoning here to not
// panic the threads if they just share telemetry with failed thread.
pub(crate) type SharedLog = Arc<parking_lot::RwLock<Logger>>;

#[must_use]
pub(crate) struct LogScope(Scope<SharedLog>);

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

    *log_lock = log_lock.new(fields);
}

pub fn current_log() -> SharedLog {
    let harness = LogHarness::get();
    let log = harness.log_scope_stack.current();

    log.unwrap_or_else(|| Arc::clone(&harness.root_log))
}

pub(crate) fn fork_log() -> SharedLog {
    let parent = current_log();
    let log = parent.read().new(slog::o!());

    Arc::new(parking_lot::RwLock::new(log))
}
