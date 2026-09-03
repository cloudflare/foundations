#![cfg(feature = "user-tracing")]

use foundations::telemetry::tracing::{RoutingMetadata, user_tracing::UserSpan};

#[derive(Debug)]
struct TestRouting;

impl RoutingMetadata for TestRouting {
    fn group_key(&self) -> String {
        "test".to_string()
    }

    fn encode(&self) -> String {
        "test".to_string()
    }
}

#[test]
fn deferred_root_remains_inactive_without_a_pipeline() {
    let root = UserSpan::deferred();

    root.activate("request", TestRouting, None);

    assert!(!root.is_sampled());
    assert!(root.w3c_traceparent().is_none());
}
