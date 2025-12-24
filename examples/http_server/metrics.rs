use foundations::telemetry::metrics::{Counter, Gauge, metrics};
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
}
