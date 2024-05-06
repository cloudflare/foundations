use super::TelemetryContext;
use crate::utils::feature_use;
use std::ops::Deref;

feature_use!(cfg(feature = "logging"), {
    use super::log::testing::{create_test_log, TestLogRecord, TestLogRecords};
    use super::settings::LogVerbosity;
    use super::settings::LoggingSettings;
    use slog::Level;
    use std::sync::Arc;
    use std::sync::RwLockReadGuard;
});

feature_use!(cfg(feature = "tracing"), {
    use super::settings::TracingSettings;
    use super::tracing::testing::{
        create_test_tracer, TestTrace, TestTraceOptions, TestTracesSink,
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

    #[cfg(feature = "tracing")]
    traces_sink: TestTracesSink,

    #[cfg(feature = "logging")]
    log_records: TestLogRecords,
}

impl TestTelemetryContext {
    pub(crate) fn new() -> Self {
        #[cfg(feature = "logging")]
        let (log, log_records) = {
            create_test_log(&LoggingSettings {
                verbosity: LogVerbosity(Level::Trace),
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
            traces_sink,

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

    /// Overrides the tracing settings on the test telemetry context, creating a new test tracer
    /// with the settings
    #[cfg(feature = "tracing")]
    pub fn set_tracing_settings(&mut self, tracing_settings: TracingSettings) {
        let (tracer, traces_sink) = { create_test_tracer(&tracing_settings) };
        self.inner.test_tracer = Some(tracer);
        self.traces_sink = traces_sink;
    }

    /// Returns all the log records produced in the test context.
    #[cfg(feature = "logging")]
    pub fn log_records(&self) -> RwLockReadGuard<Vec<TestLogRecord>> {
        self.log_records.read().unwrap()
    }

    /// Returns all the traces produced in the test context.
    #[cfg(feature = "tracing")]
    pub fn traces(&mut self, options: TestTraceOptions) -> Vec<TestTrace> {
        self.traces_sink.traces(options)
    }
}

impl Deref for TestTelemetryContext {
    type Target = TelemetryContext;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
