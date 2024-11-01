mod event_output;
mod live_reference_set;

use cf_rustracing_jaeger::span::Span;
use live_reference_set::{LiveReferenceHandle, LiveReferenceSet};
use std::sync::Arc;
use std::time::SystemTime;

type SharedSpanInner = Arc<parking_lot::RwLock<Span>>;
pub(crate) type SharedSpanHandle = Arc<LiveReferenceHandle<SharedSpanInner>>;

pub(crate) struct ActiveRoots {
    roots: Arc<LiveReferenceSet<SharedSpanInner>>,
    start: SystemTime,
}

impl Default for ActiveRoots {
    fn default() -> Self {
        Self {
            roots: Default::default(),
            start: SystemTime::now(),
        }
    }
}

impl ActiveRoots {
    pub(crate) fn get_active_traces(&self) -> String {
        event_output::spans_to_trace_events(self.start, &self.roots.get_live_references())
    }

    pub(crate) fn track(&self, value: SharedSpanInner) -> SharedSpanHandle {
        self.roots.track(value)
    }
}
