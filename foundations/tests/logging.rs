use foundations::telemetry::log::internal::{
    EXCEEDED_MAX_LOG_GENERATION_ERROR, MAX_LOG_GENERATION,
};
use foundations::telemetry::log::{add_fields, warn};
use foundations::telemetry::settings::{LoggingSettings, RateLimitingSettings};
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

// Every time we call the add_fields! macro, it adds one to the depth of the
// nested structure of Arcs inside the logger object. If the structure gets
// too deeply nested, it causes a stack overflow on drop.
//
// This test case makes sure that before it would hit a dangerous depth, it
// panics (with an error that gives you a hint as to where to go look in your
// code).
#[with_test_telemetry(test)]
fn test_exceed_limit_num_generations(_ctx: TestTelemetryContext) {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for _ in 1..(MAX_LOG_GENERATION + 1) {
            add_fields! { "key1" => "hello" }
        }
    })) {
        Ok(_) => panic!("test case exceeded the maximum log generation, but there was no panic"),
        Err(err) => {
            if let Some(msg) = err.downcast_ref::<&'static str>() {
                assert_eq!(*msg, EXCEEDED_MAX_LOG_GENERATION_ERROR);
            } else if let Some(msg) = err.downcast_ref::<String>() {
                assert_eq!(*msg, EXCEEDED_MAX_LOG_GENERATION_ERROR);
            } else {
                panic!("test case exceeded the maximum log generation, but the panic was not castable to the expected type");
            }
        }
    }
}

// Negative version of above test. If we're just under the limit, we shouldn't
// get a panic, and we also shouldn't stack overflow. This helps us make sure
// we didn't set the limit too high. For example, if we set the limit to be
// 1,000,000, then having this test here would make sure that it doesn't
// cause a stack overflow at 999,990. And if it did cause a stack overflow at
// 999,990, then this test would make sure we notice that and don't set the
// limit that high!
#[with_test_telemetry(test)]
fn test_not_exceed_limit_num_generations(_ctx: TestTelemetryContext) {
    for _ in 1..(MAX_LOG_GENERATION - 10) {
        add_fields! { "key1" => "hello" }
    }
}
