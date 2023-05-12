//! Distributed tracing settings.

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
    /// Enables tracing
    pub enabled: bool,

    /// The address of the Jaeger Thrift (UDP) agent.
    pub jaeger_tracing_server_addr: SocketAddr,

    /// Sampling ratio.
    ///
    /// This can be any fractional value between `0.0` and `1.0`.
    /// Where `1.0` means "sample everything", and `0.0` means "don't sample anything".
    pub sampling_ratio: f64,
}

impl Default for TracingSettings {
    fn default() -> Self {
        // NOTE: default Jaeger UDP agent address.
        // See: https://www.jaegertracing.io/docs/1.31/getting-started/#all-in-one
        let jaeger_tracing_server_addr: std::net::SocketAddr = (Ipv4Addr::LOCALHOST, 6831).into();

        #[cfg(feature = "settings")]
        let jaeger_tracing_server_addr = jaeger_tracing_server_addr.into();

        Self {
            enabled: false,
            jaeger_tracing_server_addr,
            sampling_ratio: 1.0,
        }
    }
}

fn _assert_traits_implemented_for_all_features() {
    fn assert<S: std::fmt::Debug + Clone + Default>() {}

    assert::<TracingSettings>();
}
