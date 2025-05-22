use foundations::telemetry::log::internal::LoggerWithKvNestingTracking;
use foundations::telemetry::log::{add_fields, set_verbosity, warn};
use foundations::telemetry::settings::{LogVerbosity, LoggingSettings, RateLimitingSettings};
use foundations::telemetry::TestTelemetryContext;
use foundations_macros::with_test_telemetry;

#[with_test_telemetry(test)]
fn test_rate_limiter(mut ctx: TestTelemetryContext) {
    for i in 0..16 {
        warn!("{}", i);
    }

    assert_eq!(ctx.log_records().len(), 16);

    ctx.set_logging_settings(LoggingSettings {
        rate_limit: RateLimitingSettings {
            enabled: true,
            max_events_per_second: 5,
        },
        ..Default::default()
    });

    for i in 16..32 {
        warn!("{}", i);
    }

    assert!(ctx.log_records().len() < 32);
}

// Every time we call set_verbosity(), or the add_fields! macro, it adds one to the depth of the
// nested structure of Arcs inside the logger object. If the structure gets too deeply nested, it
// causes a stack overflow on drop.
//
// This test case makes sure that before it would hit a dangerous depth, it panics (with an error
// that gives you a hint as to where to go look in your code).
#[with_test_telemetry(test)]
fn test_exceed_limit_kv_nesting(_ctx: TestTelemetryContext) {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for _ in 0..((LoggerWithKvNestingTracking::MAX_NESTING / 2) + 1) {
            add_fields! { "key1" => "hello" }
            set_verbosity(LogVerbosity::Info).expect("set_verbosity");
        }
    })) {
        Ok(_) => panic!("test case exceeded the maximum log KV nesting, but there was no panic"),
        Err(err) => {
            if let Some(msg) = err.downcast_ref::<&'static str>() {
                assert_eq!(
                    *msg,
                    LoggerWithKvNestingTracking::EXCEEDED_MAX_NESTING_ERROR
                );
            } else if let Some(msg) = err.downcast_ref::<String>() {
                assert_eq!(
                    *msg,
                    LoggerWithKvNestingTracking::EXCEEDED_MAX_NESTING_ERROR
                );
            } else {
                panic!("test case exceeded the maximum log KV nesting, but the panic was not castable to the expected type");
            }
        }
    }
}

// Negative version of above test. If we're just under the limit, we shouldn't get a panic, and we
// also shouldn't stack overflow. This helps us make sure we didn't set the limit too high. For
// example, if we set the limit to be 1,000,000, then having this test here would make sure that it
// doesn't cause a stack overflow at 999,990. And if it did cause a stack overflow at 999,990, then
// this test would make sure we notice that and don't set the limit that high!
#[with_test_telemetry(test)]
fn test_not_exceed_limit_kv_nesting(_ctx: TestTelemetryContext) {
    for _ in 0..((LoggerWithKvNestingTracking::MAX_NESTING / 2) - 5) {
        add_fields! { "key1" => "hello" }
        set_verbosity(LogVerbosity::Info).expect("set_verbosity");
    }
}

#[cfg(feature = "tracing-rs-compat")]
mod tracing_rs_compat {
    use std::io;
    use std::sync::{Arc, Mutex};

    use foundations::telemetry::log::{warn, TestLogRecord};
    use foundations::telemetry::settings::LoggingSettings;
    use foundations::telemetry::TelemetryContext;
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::util::SubscriberInitExt as _;

    struct TestWriter {
        log_entries: Arc<Mutex<Vec<String>>>,
    }

    impl TestWriter {
        fn with_entries(entries: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                log_entries: entries,
            }
        }
    }

    impl io::Write for TestWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let s = String::from_utf8(buf.to_vec()).unwrap();
            self.log_entries.lock().unwrap().push(s.trim().to_string());
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            unimplemented!()
        }
    }

    #[test]
    fn test_tracing_rs_compat() {
        let entries = Arc::new(Mutex::new(Vec::new()));
        let tracing_log_entries = entries.clone();
        let _subscriber = tracing_subscriber::fmt()
            .with_max_level(LevelFilter::TRACE)
            .with_writer(move || TestWriter::with_entries(entries.clone()))
            .without_time()
            .with_level(true)
            .with_ansi(false)
            .set_default();

        let settings = LoggingSettings {
            output: foundations::telemetry::settings::LogOutput::TracingRsCompat,
            ..Default::default()
        };

        let mut ctx = TelemetryContext::test();
        ctx.set_tracing_rs_log_drain(settings);

        let _scope = ctx.scope();

        warn!("compat-layer-works");

        // Validate slog is seeing all of the records
        let slog_records = ctx.log_records();
        let expected_slog_records = TestLogRecord {
            level: slog::Level::Warning,
            message: "compat-layer-works".to_string(),
            fields: vec![],
        };

        assert_eq!(*slog_records, vec![expected_slog_records]);

        let tracing_record = tracing_log_entries
            .lock()
            .unwrap()
            .first()
            .cloned()
            .unwrap();
        assert!(tracing_record.contains("WARN slog: compat-layer-works"));
    }
}
