use super::TelemetryScope;
use crate::utils::feature_use;

feature_use!(cfg(feature = "logging"), {
    use super::log::init::LogHarness;
    use super::log::internal::LogScope;
    use super::log::testing::{create_test_log, TestLogRecord, TestLogRecords};
    use std::sync::Arc;
    use std::sync::RwLockReadGuard;
});

#[cfg(feature = "tracing")]
use super::tracing::testing::{
    create_test_tracer, TestTrace, TestTraceOptions, TestTracerScope, TestTracesSink,
};

/// A handle for the scope in which telemetry testing is enabled.
///
/// Scope ends when the handle is dropped.
///
/// The scope is created with the [`TelemetryContext::test`] function and exposes API to
/// obtain collected telemetry for test assertions.
///
/// The scope can be propagated using standard [`TelemetryContext::current`] and
/// [`TelemetryContext::apply`] methods. So, if `TestTelemetryScope` is a root scope all the
/// production code telemetry will be collected by it, allowing testing without any changes to
/// the production code.
///
/// [`TelemetryContext::test`]: super::TelemetryContext::test
/// [`TelemetryContext::current`]: super::TelemetryContext::current
/// [`TelemetryContext::apply`]: super::TelemetryContext::apply
#[cfg(feature = "testing")]
#[must_use = "Test telemetry collection stops when scope is dropped."]
pub struct TestTelemetryScope {
    _inner: TelemetryScope,

    #[cfg(feature = "tracing")]
    traces_sink: TestTracesSink,

    #[cfg(feature = "logging")]
    log_records: TestLogRecords,
}

#[cfg(feature = "testing")]
impl TestTelemetryScope {
    pub(crate) fn new() -> Self {
        #[cfg(feature = "logging")]
        let (log, log_records) = {
            let settings = &LogHarness::get().settings;

            create_test_log(settings.redact_keys.clone())
        };

        #[cfg(feature = "tracing")]
        let (tracer, traces_sink) = create_test_tracer();

        TestTelemetryScope {
            _inner: TelemetryScope {
                #[cfg(feature = "logging")]
                _log_scope: LogScope::new(Arc::new(parking_lot::RwLock::new(log))),

                #[cfg(feature = "tracing")]
                _span_scope: None,

                #[cfg(feature = "tracing")]
                _test_tracer_scope: Some(TestTracerScope::new(tracer)),
            },

            #[cfg(feature = "tracing")]
            traces_sink,

            #[cfg(feature = "logging")]
            log_records,
        }
    }

    /// Returns all the log records produced in the test scope.
    #[cfg(feature = "logging")]
    pub fn log_records(&self) -> RwLockReadGuard<Vec<TestLogRecord>> {
        self.log_records.read().unwrap()
    }

    /// Returns all the traces produced in the test scope.
    #[cfg(feature = "tracing")]
    pub fn traces(&self, options: TestTraceOptions) -> Vec<TestTrace> {
        self.traces_sink.traces(options)
    }
}
