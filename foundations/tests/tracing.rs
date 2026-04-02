use foundations::telemetry::TestTelemetryContext;
use foundations::telemetry::settings::{
    ActiveSamplingSettings, RateLimitingSettings, SamplingStrategy, TracingSettings,
};
use foundations::telemetry::tracing;
use foundations_macros::with_test_telemetry;

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
        sampling_strategy: SamplingStrategy::Active(ActiveSamplingSettings {
            rate_limit: RateLimitingSettings {
                enabled: true,
                max_events_per_second: 5,
            },
            ..Default::default()
        }),
        ..Default::default()
    });

    let _scope = ctx.scope();
    for i in 10..20 {
        make_test_trace(i)
    }

    assert_eq!(ctx.traces(Default::default()).len(), 5);
}

#[with_test_telemetry(test)]
fn test_passive_sampler(mut ctx: TestTelemetryContext) {
    for i in 0..10 {
        make_test_trace(i)
    }

    assert_eq!(ctx.traces(Default::default()).len(), 10);

    ctx.set_tracing_settings(TracingSettings {
        sampling_strategy: SamplingStrategy::Passive,
        ..Default::default()
    });

    let _scope = ctx.scope();
    for i in 10..20 {
        make_test_trace(i)
    }

    assert_eq!(ctx.traces(Default::default()).len(), 0);
}

#[with_test_telemetry(test)]
fn test_span_accessors(mut ctx: TestTelemetryContext) {
    // Initially, no trace (and no span) is present
    assert!(!tracing::span_is_sampled());
    assert_eq!(tracing::trace_id(), None);

    {
        // Start a new trace
        let _root_scope = tracing::start_trace("my first span", Default::default());
        assert!(tracing::span_is_sampled());

        let trace_id = tracing::trace_id().expect("root scope should set trace ID");
        let trace_state =
            tracing::state_for_trace_stitching().expect("root scope should set stitching state");
        assert_eq!(trace_state.trace_id().to_string(), trace_id);

        // Enter a child span, which should have the same trace ID
        let _child_scope = tracing::span("my child span");
        assert!(tracing::span_is_sampled());
        assert_eq!(tracing::trace_id(), Some(trace_id));
    }

    // Change to the passive sampler, which won't sample new root traces
    ctx.set_tracing_settings(TracingSettings {
        sampling_strategy: SamplingStrategy::Passive,
        ..Default::default()
    });
    let _ctx_scope = ctx.scope();

    let _unsampled_scope = tracing::start_trace("my unsampled span", Default::default());
    assert!(!tracing::span_is_sampled());
    assert_eq!(tracing::trace_id(), None);
}
