use crate::telemetry::log::init::{LogHarness, apply_filters_to_drain};
use crate::telemetry::log::internal::LoggerWithKvNestingTracking;
use crate::telemetry::settings::LoggingSettings;
use parking_lot::RwLock as ParkingRwLock;
use slog::{Discard, Drain, KV, Key, Level, Logger, Never, OwnedKVList, Record, Serializer};
use std::fmt::Arguments;
use std::sync::{Arc, RwLock};

pub(crate) type TestLogRecords = Arc<RwLock<Vec<TestLogRecord>>>;

/// Log record produced in the [test telemetry context].
///
/// [test telemetry context]: crate::telemetry::TelemetryContext::test
#[derive(Debug, PartialEq, Eq)]
pub struct TestLogRecord {
    /// Verbosity level of the log record.
    pub level: Level,

    /// Log message.
    pub message: String,

    /// Log record fields.
    pub fields: Vec<(String, String)>,
}

#[derive(Default)]
struct TestFieldSerializer {
    fields: Vec<(String, String)>,
}

impl Serializer for TestFieldSerializer {
    fn emit_arguments(&mut self, key: Key, val: &Arguments) -> slog::Result {
        self.fields.push((key.to_string(), val.to_string()));

        Ok(())
    }
}

struct TestLogDrain<D> {
    records: TestLogRecords,
    /// Optional drain to forward logs to after recording in the test log
    forward: Option<D>,
}

impl<D> Drain for TestLogDrain<D>
where
    D: Drain<Ok = (), Err = Never>,
{
    type Ok = ();
    type Err = Never;

    fn log(&self, record: &Record, kv: &OwnedKVList) -> Result<Self::Ok, Self::Err> {
        let mut serializer = TestFieldSerializer::default();

        kv.serialize(record, &mut serializer).unwrap();
        record.kv().serialize(record, &mut serializer).unwrap();

        self.records.write().unwrap().push(TestLogRecord {
            level: record.level(),
            message: format!("{}", record.msg()),
            fields: serializer.fields,
        });

        if let Some(forward_drain) = &self.forward {
            let _ = forward_drain.log(record, kv);
        }

        Ok(())
    }
}

pub(crate) fn create_test_log(
    settings: &LoggingSettings,
) -> (LoggerWithKvNestingTracking, TestLogRecords) {
    let log_records = Arc::new(RwLock::new(vec![]));

    let drain: TestLogDrain<Discard> = TestLogDrain {
        records: Arc::clone(&log_records),
        forward: None,
    };

    let drain = Arc::new(apply_filters_to_drain(drain, settings));
    let log = LoggerWithKvNestingTracking::new(Logger::root(Arc::clone(&drain), slog::o!()));

    let _ = LogHarness::override_for_testing(LogHarness {
        root_log: Arc::new(ParkingRwLock::new(log.clone())),
        root_drain: drain,
        settings: settings.clone(),
        log_scope_stack: Default::default(),
    });

    (log, log_records)
}

/// Create a test log with a tracing-rs compat drain installed in addition to the test drain
#[cfg(feature = "tracing-rs-compat")]
pub(crate) fn create_test_log_for_tracing_compat(
    settings: &LoggingSettings,
) -> (LoggerWithKvNestingTracking, TestLogRecords) {
    use tracing_slog::TracingSlogDrain;

    assert!(matches!(
        settings.output,
        crate::telemetry::settings::LogOutput::TracingRsCompat
    ));

    let base_drain = TracingSlogDrain {};

    let log_records = Arc::new(RwLock::new(vec![]));

    let drain = TestLogDrain {
        records: Arc::clone(&log_records),
        forward: Some(base_drain.fuse()),
    };

    let drain = Arc::new(apply_filters_to_drain(drain, settings));
    let log = LoggerWithKvNestingTracking::new(Logger::root(Arc::clone(&drain), slog::o!()));

    let _ = LogHarness::override_for_testing(LogHarness {
        root_log: Arc::new(ParkingRwLock::new(log.clone())),
        root_drain: drain,
        settings: settings.clone(),
        log_scope_stack: Default::default(),
    });

    (log, log_records)
}
