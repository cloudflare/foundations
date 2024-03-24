use crate::telemetry::settings::rate_limit::RateLimitingSettings;
use crate::utils::feature_use;
use std::net::Ipv4Addr;

feature_use!(cfg(feature = "settings"), {
    use crate::settings::net::SocketAddr;
    use crate::settings::settings;
});

#[cfg(not(feature = "settings"))]
use std::net::SocketAddr;

/// Distributed tracing settings.
#[cfg_attr(
    feature = "settings",
    settings(crate_path = "crate", impl_default = false)
)]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug))]
pub struct TracingSettings {
    /// Enables tracing.
    pub enabled: bool,

    /// The exporter for the collected traces.
    pub output: TracesOutput,

    /// Sampling ratio.
    ///
    /// This can be any fractional value between `0.0` and `1.0`.
    /// Where `1.0` means "sample everything", and `0.0` means "don't sample anything".
    pub sampling_ratio: f64,

    /// Settings for rate limiting emission of traces
    pub rate_limit: RateLimitingSettings,
}

impl Default for TracingSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            output: Default::default(),
            sampling_ratio: 1.0,
            rate_limit: Default::default(),
        }
    }
}

/// The exporter for the collected traces.
#[cfg_attr(
    feature = "settings",
    settings(crate_path = "crate", impl_default = false)
)]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug))]
pub enum TracesOutput {
    /// Sends traces to the collector in the Jaeger Thrift (UDP) format.
    JaegerThriftUdp(JaegerThriftUdpOutputSettings),
}

impl Default for TracesOutput {
    fn default() -> Self {
        Self::JaegerThriftUdp(Default::default())
    }
}

/// Jaeger Thrift (UDP) traces output settings.
#[cfg_attr(
    feature = "settings",
    settings(crate_path = "crate", impl_default = false)
)]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug))]
pub struct JaegerThriftUdpOutputSettings {
    /// The address of the Jaeger Thrift (UDP) agent.
    pub server_addr: SocketAddr,

    /// Overrides the bind address for the reporter API.
    ///
    /// By default, the reporter API is only exposed on the loopback
    /// interface. This won't work in environments where the
    /// Jaeger agent is on another host (for example, Docker).
    /// Must have the same address family as `jaeger_tracing_server_addr`.
    pub reporter_bind_addr: Option<SocketAddr>,
}

impl Default for JaegerThriftUdpOutputSettings {
    fn default() -> Self {
        // NOTE: default Jaeger UDP agent address.
        // See: https://www.jaegertracing.io/docs/1.31/getting-started/#all-in-one
        let server_addr: std::net::SocketAddr = (Ipv4Addr::LOCALHOST, 6831).into();

        #[cfg(feature = "settings")]
        let server_addr = server_addr.into();

        Self {
            server_addr,
            reporter_bind_addr: None,
        }
    }
}

fn _assert_traits_implemented_for_all_features() {
    fn assert<S: std::fmt::Debug + Clone + Default>() {}

    assert::<TracingSettings>();
}
