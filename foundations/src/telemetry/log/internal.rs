use super::init::LogHarness;
use crate::telemetry::scope::Scope;
use slog::{Logger, OwnedKV, SendSyncRefUnwindSafeKV};
use std::ops::Deref;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

pub const MAX_LOG_GENERATION: u32 = 1000;
pub const EXCEEDED_MAX_LOG_GENERATION_ERROR: &str = "foundations::telemetry::log: maximum logger generation exceeded (are add_fields! or set_verbosity being called in a loop?)";

// The slog_logger() function exposes Arc<RwLock<Logger>> as a public
// interface, so we can't store the generation number where it would make the
// most sense to store it, which would be inside the RwLock, right next to
// the Logger.
//
// Doing it this way means we lose the enforced atomicity between updating the
// Logger and updating the generation number. In general, we try to only read
// or write the generation number while we hold the appropriate lock on the
// Logger. It is not mission-critical that we do this. The generation number
// is just a guard rail to prevent us from going wild. Even if there is some
// race condition and we're off by one or two, it won't affect its ability to
// do its job.
//
// NOTE: we intentionally use a lock without poisoning here to not
// panic the threads if they just share telemetry with failed thread.
pub(crate) type SharedLog = Arc<LogContext>;

#[derive(Debug)]
pub struct LogContext {
    // The logger itself. This is the most important part of this struct.
    pub logger: Arc<parking_lot::RwLock<Logger>>,

    // Generation number, incremented every time the logger is replaced with
    // a child of itself. Accuracy is not critical as this is only used as a
    // safety check. Nevertheless, when possible, please be holding the
    // logger's lock when you read or write the generation number.
    pub(crate) generation: AtomicU32,
}

impl LogContext {
    // Create a new LogContext based on a fresh logger. The generation number
    // is initialized to zero.
    pub(crate) fn new(logger: Logger) -> Self {
        Self {
            logger: Arc::new(parking_lot::RwLock::new(logger)),
            generation: AtomicU32::new(0),
        }
    }

    // Update this LogContext's logger object, and, atomically, increment
    // the generation number. This mutates 'self' using interior mutability,
    // even though it only takes a shared reference to self.
    pub(crate) fn update<F>(&self, f: F)
    where
        F: FnOnce(&Logger) -> Logger,
    {
        let mut logger_lock = self.logger.write();
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let logger = f(logger_lock.deref());
        *logger_lock = logger;

        if generation >= MAX_LOG_GENERATION {
            panic!("{}", EXCEEDED_MAX_LOG_GENERATION_ERROR);
        }
    }

    // Use interior mutability to replace the contents of 'self' with the
    // contents of 'other', while only holding a shared reference to 'self'
    #[allow(dead_code)]
    pub(crate) fn overwrite_from(&self, other: Self) {
        let mut logger_lock = self.logger.write();
        let other_logger_lock = other.logger.read();
        self.generation
            .store(other.generation.load(Ordering::SeqCst), Ordering::SeqCst);
        *logger_lock = other_logger_lock.clone();
    }
}

impl Clone for LogContext {
    // Create a new LogContext object that is functionally the same as this one
    // (refers to the same logger, has the same generation number).
    fn clone(&self) -> Self {
        let logger_lock = self.logger.read();
        let generation = self.generation.load(Ordering::SeqCst);
        Self {
            logger: Arc::new(parking_lot::RwLock::new(logger_lock.clone())),
            generation: AtomicU32::new(generation),
        }
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
    let parent = current_log();
    parent.update(move |logger: &Logger| logger.new(fields))
}

pub fn current_log() -> SharedLog {
    let harness = LogHarness::get();
    let log = harness.log_scope_stack.current();

    log.unwrap_or_else(|| Arc::clone(&harness.root_log))
}

pub(crate) fn fork_log() -> SharedLog {
    let parent = current_log();
    Arc::new((*parent).clone())
}
