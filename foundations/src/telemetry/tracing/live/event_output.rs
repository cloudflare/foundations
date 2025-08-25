//! Outputs telemetry spans in Chrome JSON trace format (i.e. the format used by about:tracing).

use crate::telemetry::tracing::live::LiveReferenceHandle;
use cf_rustracing_jaeger::Span;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::SystemTime;

/// Outputs a slice of shared spans as a Chrome JSON trace log.
pub(crate) fn spans_to_trace_events(
    epoch: SystemTime,
    spans: &[Arc<LiveReferenceHandle<Arc<RwLock<Span>>>>],
) -> String {
    use cf_rustracing::span::InspectableSpan;

    let mut log_builder = TraceLogBuilder::new();

    let end_timestamp = epoch
        .elapsed()
        .ok()
        .and_then(|x| u64::try_from(x.as_micros()).ok())
        .unwrap_or(u64::MAX);

    for span in spans {
        let span_ref = span.read();
        let Some(span_state) = span_ref.context().map(|c| c.state()) else {
            continue;
        };
        let trace_id = span_state.trace_id().to_string();
        let name = span_ref.operation_name();

        let start_ts = span_ref
            .start_time()
            .duration_since(epoch)
            .ok()
            .and_then(|x| u64::try_from(x.as_micros()).ok())
            .unwrap_or_default();

        let end_ts = span_ref
            .finish_time()
            .and_then(|x| x.duration_since(epoch).ok())
            .and_then(|x| u64::try_from(x.as_micros()).ok())
            .unwrap_or(end_timestamp);

        log_builder.write_event(&trace_id, name, "", TraceEventType::Begin, start_ts);
        log_builder.write_event(&trace_id, name, "", TraceEventType::End, end_ts);
    }

    log_builder.finalize(end_timestamp)
}

#[derive(Copy, Clone)]
enum TraceEventType {
    Begin,
    End,
}

fn escape(s: &str) -> String {
    s.escape_default().to_string()
}

struct TraceLogBuilder {
    out: String,
}

impl TraceLogBuilder {
    fn new() -> Self {
        TraceLogBuilder {
            out: "[".to_string(),
        }
    }

    fn write_event(
        &mut self,
        trace_id: &str,
        name: &str,
        category: &str,
        event_type: TraceEventType,
        timestamp_us: u64,
    ) {
        self.out.push_str(&format!(
            "{{\"pid\":1,\"name\":\"{}\",\"cat\":\"{}\",\"ph\":\"{}\",\"ts\":{},\"id\":\"{}\"}},",
            escape(name),
            escape(category),
            match event_type {
                TraceEventType::Begin => "B",
                TraceEventType::End => "E",
            },
            timestamp_us,
            trace_id,
        ));
    }

    fn finalize(mut self, end_timestamp: u64) -> String {
        self.out.push_str(&format!(
            "{{\"pid\":1,\"name\":\"Trace dump requested\",\"ph\":\"i\",\"ts\":{end_timestamp},\"s\":\"g\"}}",
        ));

        self.out.push(']');
        self.out
    }
}
