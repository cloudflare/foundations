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
    /// See: <https://opentelemetry.io/docs/languages/sdk-configuration/otlp-exporter/#otel_exporter_otlp_endpoint>
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

    /// Number of concurrent tasks to spawn for output.
    ///
    /// A higher number means more spans can be collected in parallel in a
    /// multi-threaded runtime. All tasks share a gRPC channel, but can send
    /// RPCs in parallel. The default is 1 task.
    #[serde(default = "OpenTelemetryGrpcOutputSettings::default_num_tasks")]
    pub num_tasks: usize,

    /// Maximum number of entries to be batched together and sent in one request.
    ///
    /// # Default
    ///
    /// Default value is `512`.
    #[serde(default = "OpenTelemetryGrpcOutputSettings::default_max_batch_size")]
    pub max_batch_size: usize,
}

#[cfg(not(feature = "settings"))]
impl Default for OpenTelemetryGrpcOutputSettings {
    fn default() -> Self {
        Self {
            endpoint_url: OpenTelemetryGrpcOutputSettings::default_endpoint_url(),
            request_timeout_seconds:
                OpenTelemetryGrpcOutputSettings::default_request_timeout_seconds(),
            num_tasks: OpenTelemetryGrpcOutputSettings::default_num_tasks(),
            max_batch_size: OpenTelemetryGrpcOutputSettings::default_max_batch_size(),
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

    const fn default_num_tasks() -> usize {
        1
    }

    fn default_max_batch_size() -> usize {
        512
    }
}
