use bedrock::telemetry::log::warn;
use bedrock::telemetry::settings::{LoggingSettings, RateLimitingSettings};
use bedrock::telemetry::TestTelemetryContext;
use bedrock_macros::with_test_telemetry;
use std::collections::HashSet;
use std::thread::sleep;
use std::time::Duration;

#[with_test_telemetry(test)]
fn test_rate_limiter(mut ctx: TestTelemetryContext) {
    ctx.set_logging_settings(LoggingSettings {
        rate_limit: RateLimitingSettings {
            enabled: true,
            max_events_per_second: 5,
        },
        ..Default::default()
    });

    for i in 0..16 {
        warn!("Hello world{}", i);
    }

    sleep(Duration::from_secs(1));
    for i in 16..32 {
        warn!("Hello world{}", i);
    }

    let messages: HashSet<String> = ctx
        .log_records()
        .iter()
        .map(|l| l.message.clone())
        .collect();

    assert_eq!(messages.len(), 10);

    for m in [
        "Hello world0",
        "Hello world1",
        "Hello world2",
        "Hello world3",
        "Hello world4",
        "Hello world16",
        "Hello world17",
        "Hello world18",
        "Hello world19",
        "Hello world20",
    ] {
        assert!(messages.contains(m));
    }
}
