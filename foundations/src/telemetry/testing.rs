use super::TelemetryContext;
use crate::{telemetry::TELEMETRY_INITIALIZED, utils::feature_use};
use std::{ops::Deref, sync::atomic::Ordering};

feature_use!(cfg(feature = "logging"), {
    use super::log::testing::{TestLogRecord, TestLogRecords, create_test_log};
    use super::settings::LogVerbosity;
    use super::settings::LoggingSettings;
    use std::sync::Arc;
    use std::sync::RwLockReadGuard;
});

feature_use!(cfg(feature = "tracing"), {
    use super::settings::TracingSettings;
    use super::tracing::testing::{
        TestTrace, TestTraceOptions, TestTracesSink, create_test_tracer,
    };
});

/// A test telemetry context.
///
/// [`with_test_telemetry`] macro can automatically create test context for `#[test]` and
/// `#[tokio::test]`.
///
/// The context can be enabled for the code block by obtaining its [scope] or [wrapping a future]
/// with it.
///
/// The context is created with the [`TelemetryContext::test`] function and exposes API to
/// obtain collected telemetry for test assertions in addition to standard API of
/// [`TelemetryContext`].
///
/// [`with_test_telemetry`]: super::with_test_telemetry
/// [scope]: super::TelemetryContext::scope
/// [wrapping a future]: super::TelemetryContext::apply
/// [`TelemetryContext`]: super::TelemetryContext
/// [`TelemetryContext::test`]: super::TelemetryContext::test
pub struct TestTelemetryContext {
    inner: TelemetryContext,

    // NOTE: we intentionally use a mutex without poisoning here to not
    // panic the threads if they just share telemetry with failed thread.
    #[cfg(feature = "tracing")]
    traces_sink: parking_lot::Mutex<TestTracesSink>,

    #[cfg(feature = "logging")]
    log_records: TestLogRecords,
}

impl TestTelemetryContext {
    pub(crate) fn new() -> Self {
        TELEMETRY_INITIALIZED.store(true, Ordering::Relaxed);

        #[cfg(feature = "logging")]
        let (log, log_records) = {
            create_test_log(&LoggingSettings {
                verbosity: LogVerbosity::Trace,
                ..Default::default()
            })
        };

        #[cfg(feature = "tracing")]
        let (tracer, traces_sink) = create_test_tracer(&Default::default());

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
            traces_sink: parking_lot::Mutex::new(traces_sink),

            #[cfg(feature = "logging")]
            log_records,
        }
    }

    /// Overrides the logging settings on the test telemetry context, creating a new test logger
    /// with the settings
    #[cfg(feature = "logging")]
    pub fn set_logging_settings(&mut self, logging_settings: LoggingSettings) {
        let (log, log_records) = { create_test_log(&logging_settings) };
        *self.inner.log.write() = log;
        self.log_records = log_records;
    }

    /// Overrides the logging settings on the test telemetry context with the tracing-rs compat
    /// layer. The `logging_settings` provided must use the [tracing compat] output format
    ///
    /// [tracing compat]: super::settings::LogOutput::TracingRsCompat
    #[cfg(all(feature = "logging", feature = "tracing-rs-compat"))]
    pub fn set_tracing_rs_log_drain(&mut self, logging_settings: LoggingSettings) {
        use crate::telemetry::log::testing::create_test_log_for_tracing_compat;
        let (log, log_records) = { create_test_log_for_tracing_compat(&logging_settings) };
        *self.inner.log.write() = log;
        self.log_records = log_records;
    }

    /// Overrides the tracing settings on the test telemetry context, creating a new test tracer
    /// with the settings
    #[cfg(feature = "tracing")]
    pub fn set_tracing_settings(&mut self, tracing_settings: TracingSettings) {
        let (tracer, traces_sink) = { create_test_tracer(&tracing_settings) };
        self.inner.test_tracer = Some(tracer);
        *self.traces_sink.lock() = traces_sink;
    }

    /// Returns all the log records produced in the test context.
    #[cfg(feature = "logging")]
    pub fn log_records(&self) -> RwLockReadGuard<'_, Vec<TestLogRecord>> {
        self.log_records.read().unwrap()
    }

    /// Returns all the traces produced in the test context.
    #[cfg(feature = "tracing")]
    pub fn traces(&self, options: TestTraceOptions) -> Vec<TestTrace> {
        self.traces_sink.lock().traces(options)
    }
}

impl Deref for TestTelemetryContext {
    type Target = TelemetryContext;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
