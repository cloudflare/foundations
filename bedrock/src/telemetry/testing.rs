use super::TelemetryContext;
use crate::utils::feature_use;
use std::ops::Deref;

feature_use!(cfg(feature = "logging"), {
    use super::log::init::LogHarness;
    use super::log::testing::{create_test_log, TestLogRecord, TestLogRecords};
    use std::sync::Arc;
    use std::sync::RwLockReadGuard;
});

#[cfg(feature = "tracing")]
use super::tracing::testing::{create_test_tracer, TestTrace, TestTraceOptions, TestTracesSink};

/// A test telemetry context.
///
/// The context can be enabled for the code block by obtaining its [scope] or [wrapping a future]
/// with it.
///
/// The context is created with the [`TelemetryContext::test`] function and exposes API to
/// obtain collected telemetry for test assertions in addition to standard API of
/// [`TelemetryContext`].
///
/// [scope]: super::TelemetryContext::scope
/// [wrapping a future]: super::TelemetryContext::apply
/// [`TelemetryContext`]: super::TelemetryContext
/// [`TelemetryContext::test`]: super::TelemetryContext::test
pub struct TestTelemetryContext {
    inner: TelemetryContext,

    #[cfg(feature = "tracing")]
    traces_sink: TestTracesSink,

    #[cfg(feature = "logging")]
    log_records: TestLogRecords,
}

impl TestTelemetryContext {
    pub(crate) fn new() -> Self {
        #[cfg(feature = "logging")]
        let (log, log_records) = {
            let settings = &LogHarness::get().settings;

            create_test_log(settings.redact_keys.clone())
        };

        #[cfg(feature = "tracing")]
        let (tracer, traces_sink) = create_test_tracer();

        TestTelemetryContext {
            inner: TelemetryContext {
                #[cfg(feature = "logging")]
                log: Arc::new(parking_lot::RwLock::new(log)),

                #[cfg(feature = "tracing")]
                span: None,

                #[cfg(feature = "tracing")]
                test_tracer: Some(tracer),
            },

            #[cfg(feature = "tracing")]
            traces_sink,

            #[cfg(feature = "logging")]
            log_records,
        }
    }

    /// Returns all the log records produced in the test context.
    #[cfg(feature = "logging")]
    pub fn log_records(&self) -> RwLockReadGuard<Vec<TestLogRecord>> {
        self.log_records.read().unwrap()
    }

    /// Returns all the traces produced in the test context.
    #[cfg(feature = "tracing")]
    pub fn traces(&self, options: TestTraceOptions) -> Vec<TestTrace> {
        self.traces_sink.traces(options)
    }
}

impl Deref for TestTelemetryContext {
    type Target = TelemetryContext;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
