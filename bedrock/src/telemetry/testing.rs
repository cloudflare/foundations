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

/// [`TODO ROCK-13`]
#[cfg(feature = "testing")]
#[must_use]
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

    /// [`TODO ROCK-13`]
    #[cfg(feature = "logging")]
    pub fn log_records(&self) -> RwLockReadGuard<Vec<TestLogRecord>> {
        self.log_records.read().unwrap()
    }

    /// [`TODO ROCK-13`]
    #[cfg(feature = "tracing")]
    pub fn traces(&self, options: TestTraceOptions) -> Vec<TestTrace> {
        self.traces_sink.traces(options)
    }
}
