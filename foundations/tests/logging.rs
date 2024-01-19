use foundations::telemetry::log::warn;
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
