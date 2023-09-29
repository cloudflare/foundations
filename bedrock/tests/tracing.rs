use bedrock::telemetry::settings::{RateLimitingSettings, TracingSettings};
use bedrock::telemetry::tracing::{self, test_trace};
use bedrock::telemetry::TestTelemetryContext;
use bedrock_macros::with_test_telemetry;
use std::thread;
use std::time::Duration;

fn make_test_trace(idx: usize) {
    let _root1 = tracing::span(format!("root{idx}"));
    let _root1_child1 = tracing::span(format!("root{idx}_child1"));
    let _root1_child1 = tracing::span(format!("root{idx}_child2"));
}

#[with_test_telemetry(test)]
fn test_rate_limiter(mut ctx: TestTelemetryContext) {
    ctx.set_tracing_settings(TracingSettings {
        rate_limit: RateLimitingSettings {
            enabled: true,
            max_events_per_second: 5,
        },
        ..Default::default()
    });

    let _scope = ctx.scope();

    for i in 0..10 {
        make_test_trace(i)
    }

    thread::sleep(Duration::from_secs(1));

    for i in 10..20 {
        make_test_trace(i)
    }

    assert_eq!(
        ctx.traces(Default::default()),
        vec![
            test_trace! {
                "root0" => {
                    "root0_child1" => {
                        "root0_child2"
                    }
                }
            },
            test_trace! {
                "root1" => {
                    "root1_child1" => {
                        "root1_child2"
                    }
                }
            },
            test_trace! {
                "root2" => {
                    "root2_child1" => {
                        "root2_child2"
                    }
                }
            },
            test_trace! {
                "root3" => {
                    "root3_child1" => {
                        "root3_child2"
                    }
                }
            },
            test_trace! {
                "root4" => {
                    "root4_child1" => {
                        "root4_child2"
                    }
                }
            },
            test_trace! {
                "root10" => {
                    "root10_child1" => {
                        "root10_child2"
                    }
                }
            },
            test_trace! {
                "root11" => {
                    "root11_child1" => {
                        "root11_child2"
                    }
                }
            },
            test_trace! {
                "root12" => {
                    "root12_child1" => {
                        "root12_child2"
                    }
                }
            },
            test_trace! {
                "root13" => {
                    "root13_child1" => {
                        "root13_child2"
                    }
                }
            },
            test_trace! {
                "root14" => {
                    "root14_child1" => {
                        "root14_child2"
                    }
                }
            },
        ]
    );
}
