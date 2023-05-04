use super::field_dedup::FieldDedupFilterFactory;
use super::field_filtering::FieldFilteringDrain;
use super::field_redact::FieldRedactFilterFactory;
use slog::{Drain, Key, Level, LevelFilter, Logger, Never, OwnedKVList, Record, Serializer, KV};
use std::fmt::Arguments;
use std::sync::{Arc, RwLock};

pub(crate) type TestLogRecords = Arc<RwLock<Vec<TestLogRecord>>>;

/// Log record produced in the [test telemetry scope].
///
/// [test telemetry scope]: crate::telemetry::TelemetryContext::test
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

struct TestLogDrain {
    records: TestLogRecords,
}

impl Drain for TestLogDrain {
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

        Ok(())
    }
}

pub(crate) fn create_test_log(redacted_keys: Vec<String>) -> (Logger, TestLogRecords) {
    let log_records = Arc::new(RwLock::new(vec![]));

    let drain = TestLogDrain {
        records: Arc::clone(&log_records),
    };

    let drain = FieldFilteringDrain::new(drain, FieldRedactFilterFactory::new(redacted_keys));
    let drain = FieldFilteringDrain::new(drain, FieldDedupFilterFactory);
    let drain = LevelFilter::new(drain.fuse(), Level::Trace);

    let log = Logger::root(drain.fuse(), slog::o!());

    (log, log_records)
}
