mod event_output;
mod live_reference_set;

use crate::telemetry::settings::LivenessTrackingSettings;
use crate::telemetry::tracing::internal::SharedSpanHandle;
use cf_rustracing_jaeger::Span;
use live_reference_set::LiveReferenceSet;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::SystemTime;

pub(crate) use live_reference_set::LiveReferenceHandle;

pub(crate) struct ActiveRoots {
    roots: LiveReferenceSet<Arc<RwLock<Span>>>,
    start: SystemTime,
    settings: LivenessTrackingSettings,
}

impl Default for ActiveRoots {
    fn default() -> Self {
        Self {
            roots: Default::default(),
            start: SystemTime::now(),
            settings: Default::default(),
        }
    }
}

impl ActiveRoots {
    pub(crate) fn new(settings: LivenessTrackingSettings) -> Self {
        Self {
            settings,
            ..Default::default()
        }
    }

    pub(crate) fn get_active_traces(&self) -> String {
        event_output::spans_to_trace_events(self.start, &self.roots.get_live_references())
    }

    pub(crate) fn track(&self, span: Span) -> SharedSpanHandle {
        let is_sampled = span.is_sampled();

        if self.settings.enabled && (self.settings.track_all_spans || is_sampled) {
            SharedSpanHandle::Tracked(self.roots.track(Arc::new(RwLock::new(span))))
        } else if is_sampled {
            SharedSpanHandle::Untracked(Arc::new(RwLock::new(span)))
        } else {
            SharedSpanHandle::Inactive
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::telemetry::tracing::{self, StartTraceOptions, TracingHarness};
    use crate::telemetry::TelemetryContext;

    #[test]
    #[ignore = "RUST-131: test is flakey, need to figure out a way to make it deterministic"]
    fn unsampled_spans_are_not_captured() {
        let ctx = TelemetryContext::test();
        {
            let _scope = ctx.scope();
            let _root = tracing::start_trace(
                "root",
                StartTraceOptions {
                    stitch_with_trace: None,
                    override_sampling_ratio: Some(0.0), // never sample
                },
            );

            {
                let _span1 = tracing::span("span1");
            }
            let _span2 = tracing::span("span2");
            let _span2_1 = tracing::span("span2_1");

            let harness = TracingHarness::get();
            let live_spans: Vec<_> = harness.active_roots.roots.get_live_references();

            assert!(live_spans.is_empty());
        }
    }

    #[test]
    fn sampled_spans_are_captured() {
        let ctx = TelemetryContext::test();
        {
            let _scope = ctx.scope();
            let _root = tracing::start_trace(
                "root",
                StartTraceOptions {
                    stitch_with_trace: None,
                    override_sampling_ratio: Some(1.0), // always sample
                },
            );

            {
                let _span1 = tracing::span("span1");
            }
            let _span2 = tracing::span("span2");
            let _span2_1 = tracing::span("span2_1");

            let harness = TracingHarness::get();
            let live_spans: Vec<_> = harness.active_roots.roots.get_live_references();

            assert_eq!(live_spans.len(), 3); // span1 was dropped so it's not "live" anymore.
        }
    }
}
