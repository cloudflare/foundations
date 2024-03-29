use crate::telemetry::settings::{OpenTelemetryOutputSettings, RateLimitingSettings};
use crate::utils::feature_use;
use std::net::Ipv4Addr;

feature_use!(cfg(feature = "settings"), {
    use crate::settings::net::SocketAddr;
    use crate::settings::settings;
});

#[cfg(not(feature = "settings"))]
use std::net::SocketAddr;

/// Distributed tracing settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug))]
pub struct TracingSettings {
    /// Enables tracing.
    #[cfg_attr(
        feature = "settings",
        serde(default = "TracingSettings::default_enabled")
    )]
    pub enabled: bool,

    /// The output for the collected traces.
    pub output: TracesOutput,

    /// Sampling ratio.
    ///
    /// This can be any fractional value between `0.0` and `1.0`.
    /// Where `1.0` means "sample everything", and `0.0` means "don't sample anything".
    #[cfg_attr(
        feature = "settings",
        serde(default = "TracingSettings::default_sampling_ratio")
    )]
    pub sampling_ratio: f64,

    /// Settings for rate limiting emission of traces
    pub rate_limit: RateLimitingSettings,
}

#[cfg(not(feature = "settings"))]
impl Default for TracingSettings {
    fn default() -> Self {
        Self {
            enabled: TracingSettings::default_enabled(),
            output: Default::default(),
            sampling_ratio: TracingSettings::default_sampling_ratio(),
            rate_limit: Default::default(),
        }
    }
}

impl TracingSettings {
    fn default_enabled() -> bool {
        true
    }

    fn default_sampling_ratio() -> f64 {
        1.0
    }
}

/// The output for the collected traces.
#[cfg_attr(
    feature = "settings",
    settings(crate_path = "crate", impl_default = false)
)]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug))]
pub enum TracesOutput {
    /// Sends traces to the collector in the [Jaeger Thrift (UDP)] format.
    ///
    /// [Jaeger Thrift (UDP)]: https://www.jaegertracing.io/docs/1.55/apis/#thrift-over-udp-stable
    JaegerThriftUdp(JaegerThriftUdpOutputSettings),

    /// Sends traces to the collector in the [Open Telemetry] format.
    ///
    /// [Jaeger Thrift (UDP)]: https://opentelemetry.io/
    OpenTelemetry(OpenTelemetryOutputSettings),
}

impl Default for TracesOutput {
    fn default() -> Self {
        Self::OpenTelemetry(Default::default())
    }
}

/// [Jaeger Thrift (UDP)] traces output settings.
///
/// [Jaeger Thrift (UDP)]: https://www.jaegertracing.io/docs/1.55/apis/#thrift-over-udp-stable
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug))]
pub struct JaegerThriftUdpOutputSettings {
    /// The address of the Jaeger Thrift (UDP) agent.
    ///
    /// The default value is the default Jaeger UDP server address.
    /// See: https://www.jaegertracing.io/docs/1.31/getting-started/#all-in-one
    #[cfg_attr(
        feature = "settings",
        serde(default = "JaegerThriftUdpOutputSettings::default_server_addr")
    )]
    pub server_addr: SocketAddr,

    /// Overrides the bind address for the reporter API.
    ///
    /// By default, the reporter API is only exposed on the loopback
    /// interface. This won't work in environments where the
    /// Jaeger agent is on another host (for example, Docker).
    /// Must have the same address family as `jaeger_tracing_server_addr`.
    pub reporter_bind_addr: Option<SocketAddr>,
}

#[cfg(not(feature = "settings"))]
impl Default for JaegerThriftUdpOutputSettings {
    fn default() -> Self {
        Self {
            server_addr: JaegerThriftUdpOutputSettings::default_server_addr(),
            reporter_bind_addr: None,
        }
    }
}

impl JaegerThriftUdpOutputSettings {
    fn default_server_addr() -> SocketAddr {
        let server_addr: std::net::SocketAddr = (Ipv4Addr::LOCALHOST, 6831).into();

        #[cfg(feature = "settings")]
        let server_addr = server_addr.into();

        server_addr
    }
}
