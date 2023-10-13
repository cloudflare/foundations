use bedrock::telemetry::settings::{RateLimitingSettings, TracingSettings};
use bedrock::telemetry::tracing;
use bedrock::telemetry::TestTelemetryContext;
use bedrock_macros::with_test_telemetry;

fn make_test_trace(idx: usize) {
    let _root1 = tracing::span(format!("root{idx}"));
    let _root1_child1 = tracing::span(format!("root{idx}_child1"));
    let _root1_child1 = tracing::span(format!("root{idx}_child2"));
}

#[with_test_telemetry(test)]
fn test_rate_limiter(mut ctx: TestTelemetryContext) {
    for i in 0..10 {
        make_test_trace(i)
    }

    assert_eq!(ctx.traces(Default::default()).len(), 10);

    ctx.set_tracing_settings(TracingSettings {
        rate_limit: RateLimitingSettings {
            enabled: true,
            max_events_per_second: 5,
        },
        ..Default::default()
    });

    for i in 10..20 {
        make_test_trace(i)
    }

    assert!(ctx.traces(Default::default()).len() < 20);
}
