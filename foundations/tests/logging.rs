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
