use crate::telemetry::settings::RateLimitingSettings;
use crate::utils::feature_use;
use std::net::Ipv4Addr;

feature_use!(cfg(feature = "settings"), {
    use crate::settings::net::SocketAddr;
    use crate::settings::settings;
});

#[cfg(feature = "telemetry-otlp-grpc")]
use crate::telemetry::settings::OpenTelemetryGrpcOutputSettings;

#[cfg(not(feature = "settings"))]
use std::net::SocketAddr;

/// Distributed tracing settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub struct TracingSettings {
    /// Enables tracing.
    #[serde(default = "TracingSettings::default_enabled")]
    pub enabled: bool,

    /// The output for the collected traces.
    pub output: TracesOutput,

    /// The strategy used to sample traces.
    pub sampling_strategy: SamplingStrategy,
    /// Enable liveness tracking of all generated spans. Even if the spans are
    /// unsampled. This can be useful for debugging potential hangs cause by
    /// some objects remaining in memory.  The default value is false, meaning
    /// only sampled spans are tracked.
    ///
    /// To get a json dump of the currently active spans, query the telemetry
    /// server's route at `/debug/traces`.
    pub liveness_tracking: LivenessTrackingSettings,
}

#[cfg(not(feature = "settings"))]
impl Default for TracingSettings {
    fn default() -> Self {
        Self {
            enabled: TracingSettings::default_enabled(),
            output: Default::default(),
            sampling_strategy: Default::default(),
            liveness_tracking: Default::default(),
        }
    }
}

impl TracingSettings {
    fn default_enabled() -> bool {
        true
    }
}

/// The output for the collected traces.
#[cfg_attr(
    feature = "settings",
    settings(crate_path = "crate", impl_default = false)
)]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub enum TracesOutput {
    /// Sends traces to the collector in the [Jaeger Thrift (UDP)] format.
    ///
    /// [Jaeger Thrift (UDP)]: https://www.jaegertracing.io/docs/1.55/apis/#thrift-over-udp-stable
    JaegerThriftUdp(JaegerThriftUdpOutputSettings),

    /// Sends traces to the collector in the [Open Telemetry] format over [gRPC].
    ///
    /// [Jaeger Thrift (UDP)]: https://opentelemetry.io/
    /// [gRPC]: https://grpc.io/
    #[cfg(feature = "telemetry-otlp-grpc")]
    OpenTelemetryGrpc(OpenTelemetryGrpcOutputSettings),
}

impl Default for TracesOutput {
    fn default() -> Self {
        Self::JaegerThriftUdp(Default::default())
    }
}

/// [Jaeger Thrift (UDP)] traces output settings.
///
/// [Jaeger Thrift (UDP)]: https://www.jaegertracing.io/docs/1.55/apis/#thrift-over-udp-stable
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub struct JaegerThriftUdpOutputSettings {
    /// The address of the Jaeger Thrift (UDP) agent.
    ///
    /// The default value is the default Jaeger UDP server address.
    /// See: <https://www.jaegertracing.io/docs/1.31/getting-started/#all-in-one>
    #[serde(default = "JaegerThriftUdpOutputSettings::default_server_addr")]
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

/// Settings used when active sampling is enabled.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub struct ActiveSamplingSettings {
    /// Sampling ratio.
    ///
    /// This can be any fractional value between `0.0` and `1.0`.
    /// Where `1.0` means "sample everything", and `0.0` means "don't sample anything".
    #[serde(default = "ActiveSamplingSettings::default_sampling_ratio")]
    pub sampling_ratio: f64,

    /// Settings for rate limiting emission of traces
    pub rate_limit: RateLimitingSettings,
}

impl ActiveSamplingSettings {
    fn default_sampling_ratio() -> f64 {
        1.0
    }
}

#[cfg(not(feature = "settings"))]
impl Default for ActiveSamplingSettings {
    fn default() -> Self {
        Self {
            sampling_ratio: ActiveSamplingSettings::default_sampling_ratio(),
            rate_limit: Default::default(),
        }
    }
}

/// The sampling strategy used for tracing purposes.
#[cfg_attr(
    feature = "settings",
    settings(crate_path = "crate", impl_default = false)
)]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub enum SamplingStrategy {
    /// This only samples traces which have one or more references.
    ///
    /// Passive sampling is meant to be used when we want to defer the sampling logic to the parent
    /// service. The traces will only be sent if the parent service sent trace stitching data.
    /// Backed by [cf_rustracing::sampler::PassiveSampler]
    Passive,

    /// Active sampling.
    ///
    /// This will sample a percentage of traces, specified in [sampling ratio], and also supports
    /// rate limiting traces - see [RateLimitingSettings].
    ///
    /// [sampling ratio]: crate::telemetry::settings::ActiveSamplingSettings::sampling_ratio
    Active(ActiveSamplingSettings),
}

impl Default for SamplingStrategy {
    fn default() -> Self {
        Self::Active(Default::default())
    }
}

/// Controls liveness tracking of all generated spans. When liveness tracking is
/// enabled, a sample of spans are kept in-memory until they are finished.
/// Querying the telemetry server at `/debug/traces` will dump all tracked spans
/// as a Chrome JSON trace log. See `chrome://tracing/`.
///
/// When tracking is enabled, by default only sampled spans are tracked.
#[cfg_attr(
    feature = "settings",
    settings(crate_path = "crate", impl_default = false)
)]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub struct LivenessTrackingSettings {
    /// Enables liveness tracking.
    pub enabled: bool,
    /// Enable liveness tracking of all generated spans. Even if the spans are
    /// unsampled. This can be useful for debugging potential hangs caused by
    /// some objects remaining in memory. The default value is `false`, meaning
    /// _only sampled_ spans are tracked.
    pub track_all_spans: bool,
}

// we want liveness tracking to be enabled for tests and clippy only sees
// `enabled: false`.
#[allow(clippy::derivable_impls)]
impl Default for LivenessTrackingSettings {
    fn default() -> Self {
        Self {
            enabled: cfg!(test),
            track_all_spans: false,
        }
    }
}
