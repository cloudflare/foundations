use foundations::telemetry::metrics::{
    Counter, Gauge, NativeHistogram, NativeHistogramBuilder, metrics,
};
use std::sync::Arc;

#[metrics]
pub(crate) mod http_server {
    /// Number of active client connections.
    pub fn active_connections(endpoint_name: &Arc<String>) -> Gauge;

    /// Number of failed client connections.
    pub fn failed_connections_total(endpoint_name: &Arc<String>) -> Counter;

    /// Number of HTTP requests.
    pub fn requests_total(endpoint_name: &Arc<String>) -> Counter;

    /// Number of failed requests.
    pub fn requests_failed_total(endpoint_name: &Arc<String>, status_code: u16) -> Counter;
    /// Time spent handling a request, in seconds.
    #[ctor = NativeHistogramBuilder {
        classic_buckets: Some(&[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
        ..NativeHistogramBuilder::new(1.1).with_max_buckets(160)
    }]
    pub fn request_latency_seconds(endpoint_name: &Arc<String>) -> NativeHistogram;
}
