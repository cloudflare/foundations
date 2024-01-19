use foundations::telemetry::tracing::{self, test_trace};
use foundations::telemetry::{with_test_telemetry, TestTelemetryContext};

#[with_test_telemetry(tokio::test)]
async fn wrap_tokio_test(ctx: TestTelemetryContext) {
    {
        let _span = tracing::span("span1");
    }

    tokio::task::yield_now().await;

    {
        let _span = tracing::span("span2");
    }

    assert_eq!(
        ctx.traces(Default::default()),
        vec![
            test_trace! {
                "span1"
            },
            test_trace! {
                "span2"
            }
        ]
    );
}

#[with_test_telemetry(test)]
fn wrap_rust_test(ctx: TestTelemetryContext) {
    {
        let _span = tracing::span("root");
    }

    assert_eq!(
        ctx.traces(Default::default()),
        vec![test_trace! {
            "root"
        }]
    );
}
