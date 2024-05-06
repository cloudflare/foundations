#[cfg(feature = "settings")]
use crate::settings::settings;

/// [OpenTelemetry] output settings.
///
/// [OpenTelemetry]: https://opentelemetry.io/
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub struct OpenTelemetryGrpcOutputSettings {
    /// The URL of the endpoint that will receive the telemetry data.
    ///
    /// # Default
    /// Default value is the standard gRPC endpoints URL: `http://localhost:4317`.
    /// See: https://opentelemetry.io/docs/languages/sdk-configuration/otlp-exporter/#otel_exporter_otlp_endpoint
    #[serde(default = "OpenTelemetryGrpcOutputSettings::default_endpoint_url")]
    pub endpoint_url: String,

    /// Output request timeout in seconds.
    ///
    /// An error will be logged (if logging is enabled for the service) if timeout is reached.
    ///
    /// # Default
    ///
    /// Default value is `10` seconds.
    #[serde(default = "OpenTelemetryGrpcOutputSettings::default_request_timeout_seconds")]
    pub request_timeout_seconds: u64,
}

#[cfg(not(feature = "settings"))]
impl Default for OpenTelemetryGrpcOutputSettings {
    fn default() -> Self {
        Self {
            endpoint_url: OpenTelemetryGrpcOutputSettings::default_endpoint_url(),
            request_timeout_seconds:
                OpenTelemetryGrpcOutputSettings::default_request_timeout_seconds(),
        }
    }
}

impl OpenTelemetryGrpcOutputSettings {
    fn default_endpoint_url() -> String {
        "http://localhost:4317".into()
    }

    fn default_request_timeout_seconds() -> u64 {
        10
    }
}
