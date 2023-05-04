//! Service telemetry.

#[cfg(feature = "logging")]
mod context_stack;

/// Logging-related functionality.
#[cfg(feature = "logging")]
pub mod log;

use slog::Logger;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

#[cfg(feature = "logging")]
use self::log::internal::{LogScope, SharedLog};

#[cfg(all(feature = "logging", feature = "testing"))]
mod logging_testing_imports {
    pub(super) use super::log::init::LogHarness;
    pub(super) use super::log::testing::{TestLogRecord, TestLogRecords};
    pub(super) use std::sync::RwLockReadGuard;
}

#[cfg(all(feature = "logging", feature = "testing"))]
use self::logging_testing_imports::*;

/// Wrapper for a future that provides it with [`TelemetryContext`].
pub struct WithTelemetryContext<'f, T> {
    // NOTE: we intentionally erase type here as we can get close to the type
    // length limit, adding telemetry wrappers on top causes compiler to fail in some
    // cases.
    inner: Pin<Box<dyn Future<Output = T> + Send + 'f>>,
    ctx: TelemetryContext,
}

impl<'f, T> Future for WithTelemetryContext<'f, T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let _telemetry_scope = self.ctx.scope();

        self.inner.as_mut().poll(cx)
    }
}

/// TODO document when <https://jira.cfdata.org/browse/ROCK-9> is implemented,
/// so we can provide comprehensive code examples.
#[must_use]
pub struct TelemetryScope {
    #[cfg(feature = "logging")]
    _log_scope: LogScope,
}

/// TODO document when <https://jira.cfdata.org/browse/ROCK-9> is implemented,
/// so we can provide comprehensive code examples.
#[cfg(feature = "testing")]
#[must_use]
pub struct TestTelemetryScope {
    _inner: TelemetryScope,
    #[cfg(feature = "logging")]
    log_records: TestLogRecords,
}

#[cfg(feature = "testing")]
impl TestTelemetryScope {
    /// TODO document when <https://jira.cfdata.org/browse/ROCK-9> is implemented,
    /// so we can provide comprehensive code examples.
    #[cfg(feature = "logging")]
    pub fn log_records(&self) -> RwLockReadGuard<Vec<TestLogRecord>> {
        self.log_records.read().unwrap()
    }
}

/// Telemetry context captures current log and tracing span and allows them to be used in scopes
/// with indirect control flow (e.g. in closures passed to 3rd-party libraries) or applied to a
/// future.
#[derive(Debug, Clone)]
pub struct TelemetryContext {
    #[cfg(feature = "logging")]
    log: SharedLog,
}

impl TelemetryContext {
    /// TODO document when <https://jira.cfdata.org/browse/ROCK-9> is implemented,
    /// so we can provide comprehensive code examples.
    pub fn current() -> Self {
        Self {
            #[cfg(feature = "logging")]
            log: log::internal::current_log(),
        }
    }

    /// TODO document when <https://jira.cfdata.org/browse/ROCK-9> is implemented,
    /// so we can provide comprehensive code examples.
    pub fn scope(&self) -> TelemetryScope {
        TelemetryScope {
            #[cfg(feature = "logging")]
            _log_scope: LogScope::new(Arc::clone(&self.log)),
        }
    }

    /// TODO document when <https://jira.cfdata.org/browse/ROCK-9> is implemented,
    /// so we can provide comprehensive code examples.
    #[cfg(feature = "testing")]
    pub fn test() -> TestTelemetryScope {
        let settings = &LogHarness::get().settings;
        let (log, log_records) = log::testing::create_test_log(settings.redact_keys.clone());

        let log_scope = LogScope::new(Arc::new(parking_lot::RwLock::new(log)));
        let telemetry_scope = TelemetryScope {
            _log_scope: log_scope,
        };

        TestTelemetryScope {
            _inner: telemetry_scope,
            log_records,
        }
    }

    /// TODO document when <https://jira.cfdata.org/browse/ROCK-9> is implemented,
    /// so we can provide comprehensive code examples.
    #[cfg(feature = "logging")]
    pub fn with_forked_log(&self) -> Self {
        Self {
            log: log::internal::fork(),
        }
    }

    /// TODO document when <https://jira.cfdata.org/browse/ROCK-9> is implemented,
    /// so we can provide comprehensive code examples.
    #[cfg(feature = "logging")]
    pub fn slog_logger(&self) -> parking_lot::RwLockReadGuard<Logger> {
        self.log.read()
    }

    /// TODO document when <https://jira.cfdata.org/browse/ROCK-9> is implemented,
    /// so we can provide comprehensive code examples.
    pub fn apply<'f, F>(self, fut: F) -> WithTelemetryContext<'f, F::Output>
    where
        F: Future + Send + 'f,
    {
        WithTelemetryContext {
            inner: Box::pin(fut),
            ctx: self,
        }
    }
}
