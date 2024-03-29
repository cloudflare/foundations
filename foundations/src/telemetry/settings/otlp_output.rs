#[cfg(feature = "settings")]
use crate::settings::settings;

/// [OpenTelemetry] output settings.
///
/// [OpenTelemetry]: https://opentelemetry.io/
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug))]
pub struct OpenTelemetryOutputSettings {
    /// The URL of the endpoint that will receive the telemetry data.
    ///
    /// # Default
    /// Default value is the standard gRPC endpoints URL: `http://localhost:4317`.
    /// See: https://opentelemetry.io/docs/languages/sdk-configuration/otlp-exporter/#otel_exporter_otlp_endpoint
    #[cfg_attr(
        feature = "settings",
        serde(default = "OpenTelemetryOutputSettings::default_endpoint_url")
    )]
    pub endpoint_url: String,

    /// Output request timeout in seconds.
    ///
    /// An error will be logged (if logging is enabled for the service) if timeout is reached.
    ///
    /// # Default
    ///
    /// Default value is `10` seconds.
    #[cfg_attr(
        feature = "settings",
        serde(default = "OpenTelemetryOutputSettings::default_request_timeout_seconds")
    )]
    pub request_timeout_seconds: u64,

    /// Protocol to be used communicating with the endpoint.
    pub protocol: OpenTelemetryOutputProtocol,
}

#[cfg(not(feature = "settings"))]
impl Default for OpenTelemetryOutputSettings {
    fn default() -> Self {
        Self {
            endpoint_url: OpenTelemetryOutputSettings::default_endpoint_url(),
            request_timeout_seconds: OpenTelemetryOutputSettings::default_request_timeout_seconds(),
            protocol: Default::default(),
        }
    }
}

impl OpenTelemetryOutputSettings {
    fn default_endpoint_url() -> String {
        "http://localhost:4317".into()
    }

    fn default_request_timeout_seconds() -> u64 {
        10
    }
}

/// OpenTelemetry output protocol.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Default, Debug))]
#[derive(Copy)]
pub enum OpenTelemetryOutputProtocol {
    /// gRPC
    #[default]
    Grpc,

    /// HTTP
    Http,
}
