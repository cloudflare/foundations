//! Metrics-related functionality.
//!
//! Foundations provides simple and ergonomic interface to [Prometheus] metrics:
//! - Use [`metrics`] macro to define regular metrics.
//! - Use [`report_info`] function to register service information metrics (metrics, whose value is
//!   persistent during the service lifetime, e.g. software version).
//! - Use [`collect`] method to obtain metrics report programmatically.
//! - Use [telemetry server] to expose a metrics endpoint.
//!
//! [Prometheus]: https://prometheus.io/

use super::settings::MetricsSettings;
use crate::Result;
use prometheus::{Encoder, TextEncoder};
use serde::Serialize;
use std::any::TypeId;

mod gauge;

pub(super) mod init;

#[doc(hidden)]
pub mod internal;

use internal::{ErasedInfoMetric, Registries};

pub use gauge::{GaugeGuard, RangeGauge};
pub use prometheus_client::metrics::exemplar::{CounterWithExemplar, HistogramWithExemplars};
pub use prometheus_client::metrics::family::MetricConstructor;
pub use prometheus_client::metrics::gauge::Gauge;
pub use prometheus_client::metrics::histogram::Histogram;
pub use prometools::histogram::{HistogramTimer, TimeHistogram};
pub use prometools::nonstandard::NonstandardUnsuffixedCounter as Counter;
pub use prometools::serde::Family;

/// Collects all metrics in [Prometheus text format].
///
/// [Prometheus text format]: https://prometheus.io/docs/instrumenting/exposition_formats/#text-based-format
pub fn collect(settings: &MetricsSettings) -> Result<String> {
    let mut buffer = Vec::with_capacity(128);

    Registries::collect(&mut buffer, settings.report_optional)?;
    TextEncoder::new().encode(&prometheus::gather(), &mut buffer)?;

    buffer.extend_from_slice(b"# EOF\n");

    Ok(String::from_utf8(buffer)?)
}

/// A macro that allows to define Prometheus metrics.
///
/// The macro is a proc macro attribute that should be put on a module containing
/// bodyless functions. Each bodyless function corresponds to a single metric, whose
/// name becomes `<global prefix>_<module name>_<bodyless function name>`and function's
/// Rust doc comment is reported as metric description to Prometheus.
///
/// # Labels
/// Arguments of the bodyless functions become labels for that metric.
///
/// The metric types must implement [`prometheus_client::metrics::MetricType`], they
/// are reexported from this module for convenience:
///
/// * [`Counter`]
/// * [`CounterWithExemplar`]
/// * [`Gauge`]
/// * [`Histogram`]
/// * [`HistogramWithExemplars`]
/// * [`TimeHistogram`]
///
/// The metrics associated with the functions are automatically registered in a global
/// registry, and they can be collected with the [`collect`] function.
///
/// # Metric attributes
///
/// Example below shows how to use all the attributes listed here.
///
/// ## `#[ctor]`
///
/// `#[ctor]` attribute allows specifying how the metric should be built (e.g. [`HistogramBuilder`]).
/// Constructor should implement the [`MetricConstructor<MetricType>`] trait.
///
/// ## `#[optional]`
///
/// Metrics marked with `#[optional]` are collected in a separate registry and reported only if
/// `collect_optional` argument of [`collect`] is set to `true`, or, in case the [telemetry server]
/// is used, if [`MetricsSettings::report_optional`] is set to `true`.
///
///
/// Can be used for heavy-weight metrics (e.g. with high cardinality) that don't need to be reported
/// on a regular basis.
///
/// # Example
///
/// ```
/// # // As rustdoc puts doc tests in `fn main()`, the implicit `use super::*;` inserted
/// # // in the metric mod doesn't see `SomeLabel`, so we wrap the entire test in a module.
/// # mod rustdoc_workaround {
/// use foundations::telemetry::metrics::{metrics, Counter, Gauge, HistogramBuilder, TimeHistogram};
/// use serde_with::DisplayFromStr;
/// use std::net::IpAddr;
/// use std::io;
/// use std::sync::Arc;
///
/// mod labels {
///     use serde::Serialize;
///
///     #[derive(Clone, Eq, Hash, PartialEq, Serialize)]
///     #[serde(rename_all = "lowercase")]
///     pub enum IpVersion {
///         V4,
///         V6,
///     }
///
///     #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
///     #[serde(rename_all = "lowercase")]
///     pub enum L4Protocol {
///         Tcp,
///         Udp,
///         Quic,
///         Unknown,
///     }
///
///     #[derive(Clone, Eq, Hash, PartialEq, Serialize)]
///     #[serde(rename_all = "lowercase")]
///     pub enum ProxiedProtocol {
///         Ip,
///         Tcp,
///         Udp,
///         Quic,
///         Unknown,
///     }
///
///     impl From<L4Protocol> for ProxiedProtocol {
///         fn from(l4: L4Protocol) -> Self {
///             match l4 {
///                 L4Protocol::Tcp => Self::Tcp,
///                 L4Protocol::Udp => Self::Udp,
///                 L4Protocol::Quic => Self::Quic,
///                 L4Protocol::Unknown => Self::Unknown,
///             }
///         }
///     }
/// }
///
/// // The generated module contains an implicit `use super::*;` statement.
/// #[metrics]
/// pub mod my_app_metrics {
///     /// Number of active client connections
///     pub fn client_connections_active(
///         // Labels with an anonymous reference type will get cloned.
///         endpoint: &Arc<String>,
///         protocol: labels::L4Protocol,
///         ip_version: labels::IpVersion,
///         ingress_ip: IpAddr,
///     ) -> Gauge;
///
///     /// Histogram of task schedule delays
///     #[ctor = HistogramBuilder {
///         // 100 us to 1 second
///         buckets: &[1E-4, 2E-4, 3E-4, 4E-4, 5E-4, 6E-4, 7E-4, 8E-4, 9E-4, 1E-3, 1E-2, 2E-2, 4E-2, 8E-2, 1E-1, 1.0],
///     }]
///     pub fn tokio_runtime_task_schedule_delay_histogram(
///         task: &Arc<str>,
///     ) -> TimeHistogram;
///
///     /// Number of client connections
///     pub fn client_connections_total(
///         endpoint: &Arc<String>,
///         // Labels with type `impl Into<T>` will invoke `std::convert::Into<T>`.
///         protocol: impl Into<labels::ProxiedProtocol>,
///         ingress_ip: IpAddr,
///     ) -> Counter;
///
///     /// Tunnel transmit error count
///     pub fn tunnel_transmit_errors_total(
///         endpoint: &Arc<String>,
///         protocol: labels::L4Protocol,
///         ingress_ip: IpAddr,
///         // `serde_as` attribute is allowed without decorating the metric with `serde_with::serde_as`.
///         #[serde_as(as = "DisplayFromStr")]
///         kind: io::ErrorKind,
///         raw_os_error: i32,
///     ) -> Counter;
///
///     /// Number of stalled futures
///     #[optional]
///     pub fn debug_stalled_future_count(
///         // Labels with a `'static` lifetime are used as is, without cloning.
///         name: &'static str,
///     ) -> Counter;
///
///     /// Number of Proxy-Status serialization errors
///     // Metrics with no labels are also obviously supported.
///     pub fn proxy_status_serialization_error_count() -> Counter;
/// }
///
/// fn usage() {
///     let endpoint = Arc::new("http-over-tcp".to_owned());
///     let l4_protocol = labels::L4Protocol::Tcp;
///     let ingress_ip = "127.0.0.1".parse::<IpAddr>().unwrap();
///
///     my_app_metrics::client_connections_total(
///         &endpoint,
///         l4_protocol,
///         ingress_ip,
///     ).inc();
///
///     let client_connections_active = my_app_metrics::client_connections_active(
///         &endpoint,
///         l4_protocol,
///         labels::IpVersion::V4,
///         ingress_ip,
///     );
///
///     client_connections_active.inc();
///
///     my_app_metrics::proxy_status_serialization_error_count().inc();
///
///     client_connections_active.dec();
/// }
/// # }
/// ```
///
/// # Renamed or reexported crate
///
/// The macro will fail to compile if `foundations` crate is reexported. However, the crate path
/// can be explicitly specified for the macro to workaround that:
///
/// ```
/// # mod rustdoc_workaround {
/// mod reexport {
///     pub use foundations::*;
/// }
///
/// use self::reexport::telemetry::metrics::Counter;
///
/// #[reexport::telemetry::metrics::metrics(crate_path = "reexport")]
/// mod my_app_metrics {
///     /// Total number of tasks workers stole from each other.
///     fn tokio_runtime_total_task_steal_count() -> Counter;
/// }
/// # }
/// ```
///
/// [telemetry server]: crate::telemetry::init_with_server
/// [`MetricsSettings::report_optional`]: crate::telemetry::settings::MetricsSettings::report_optional
pub use foundations_macros::metrics;

/// A macro that allows to define a Prometheus info metric.
///
/// The metrics defined by this function should be used with [`report_info`] and they can be
/// collected with the telemetry server.
///
/// The struct name becomes the metric name in `snake_case`, and each field of the struct becomes
/// a label.
///
/// # Simple example
///
/// See [`report_info`] for a simple example.
///
/// # Renaming the metric.
///
/// ```
/// use foundations::telemetry::metrics::{info_metric, report_info};
///
/// /// Build information
/// #[info_metric(name = "build_info")]
/// struct BuildInformation {
///     version: &'static str,
/// }
///
/// report_info(BuildInformation {
///     version: "1.2.3",
/// });
/// ```
/// # Renamed or reexported crate
///
/// The macro will fail to compile if `foundations` crate is reexported. However, the crate path
/// can be explicitly specified for the macro to workaround that:
///
/// ```
/// # mod rustdoc_workaround {
/// mod reexport {
///     pub use foundations::*;
/// }
///
/// /// Build information
/// #[reexport::telemetry::metrics::info_metric(crate_path = "reexport")]
/// struct BuildInfo {
///     version: &'static str,
/// }
/// # }
/// ```
pub use foundations_macros::info_metric;

/// Describes an info metric.
///
/// Info metrics are used to expose textual information, through the label set, which should not
/// change often during process lifetime. Common examples are an application's version, revision
/// control commit, and the version of a compiler.
pub trait InfoMetric: Serialize + Send + Sync + 'static {
    /// The name of the info metric.
    const NAME: &'static str;

    /// The help message of the info metric.
    const HELP: &'static str;
}

/// Registers an info metric, i.e. a gauge metric whose value is always `1`, set at init time.
///
/// # Examples
///
/// ```
/// use foundations::telemetry::metrics::{info_metric, report_info};
///
/// /// Build information
/// #[info_metric]
/// struct BuildInfo {
///     version: &'static str,
/// }
///
/// report_info(BuildInfo {
///     version: "1.2.3",
/// });
/// ```
pub fn report_info<M>(info_metric: impl Into<Box<M>>)
where
    M: InfoMetric,
{
    Registries::get().info.write().insert(
        TypeId::of::<M>(),
        info_metric.into() as Box<dyn ErasedInfoMetric>,
    );
}

/// A builder suitable for [`Histogram`] and [`TimeHistogram`].
///
/// # Example
///
/// ```
/// # // As rustdoc puts doc tests in `fn main()`, the implicit `use super::*;` inserted
/// # // in the metric mod doesn't see `SomeLabel`, so we wrap the entire test in a module.
/// # mod rustdoc_workaround {
/// use foundations::telemetry::metrics::{metrics, HistogramBuilder, TimeHistogram};
///
/// #[metrics]
/// pub mod my_app_metrics {
///     #[ctor = HistogramBuilder {
///         // 100 us to 1 second
///         buckets: &[1E-4, 2E-4, 3E-4, 4E-4, 5E-4, 6E-4, 7E-4, 8E-4, 9E-4, 1E-3, 1E-2, 2E-2, 4E-2, 8E-2, 1E-1, 1.0],
///     }]
///     pub fn tokio_runtime_task_schedule_delay_histogram(
///         task: String,
///     ) -> TimeHistogram;
/// }
/// # }
/// ```
#[derive(Clone)]
pub struct HistogramBuilder {
    /// The buckets of the histogram to be built.
    pub buckets: &'static [f64],
}

impl MetricConstructor<Histogram> for HistogramBuilder {
    fn new_metric(&self) -> Histogram {
        Histogram::new(self.buckets.iter().cloned())
    }
}

impl<S> MetricConstructor<HistogramWithExemplars<S>> for HistogramBuilder {
    fn new_metric(&self) -> HistogramWithExemplars<S> {
        HistogramWithExemplars::new(self.buckets.iter().cloned())
    }
}

impl MetricConstructor<TimeHistogram> for HistogramBuilder {
    fn new_metric(&self) -> TimeHistogram {
        TimeHistogram::new(self.buckets.iter().cloned())
    }
}

/// Adds an [ExtraProducer] that runs whenever Prometheus metrics are scraped.
/// The producer appends metrics into a provided buffer to make them available.
///
/// The motivation for this is enabling metrics export from third party libraries that
/// do not integrate with `foundations`` directly in a forward and backward compatible way.
///
/// One can ask "why not expose a `Registry` from `prometheus_client`?" The reason is that
/// it would require compatibility between `prometheus_client` version that `foundations`
/// depend on and the version that the third party crates depend on. With a producer
/// that simply appends bytes into a buffer we avoid the need to have this match,
/// at the cost of requiring the consumers to do the encoding themselves.
///
/// # Example
///
/// In this example we have a `Cache` that would be provided from an external crate, which
/// does not expose metrics directly, but allows registering them in a provided `Registry`.
///
/// The consumer code would make a `Registry` with whatever version they want and do
/// the encoding in a text format to make a suitable [ExtraProducer].
///
/// ```
/// #[derive(Default)]
/// struct Cache {
///   calls: prometheus_client::metrics::counter::Counter,
/// }
///
/// impl Cache {
///   fn register_metrics(&self, registry: &mut prometheus_client::registry::Registry) {
///     registry.register(
///       "calls",
///       "The number of calls into cache",
///       Box::new(self.calls.clone()),
///     )
///   }
/// }
///
/// let cache = Cache::default();
///
/// let mut registry = prometheus_client::registry::Registry::default();
/// let mut sub_registry = registry.sub_registry_with_prefix("cache").sub_registry_with_label((
///     std::borrow::Cow::Borrowed("cache"),
///     std::borrow::Cow::Borrowed("things"),
/// ));
///
/// cache.register_metrics(&mut sub_registry);
///
/// foundations::telemetry::metrics::add_extra_producer(move |buffer: &mut Vec<u8>| {
///     prometheus_client::encoding::text::encode(buffer, &registry).unwrap();
/// });
/// ```
pub fn add_extra_producer<P>(p: P)
where
    P: ExtraProducer + 'static,
{
    Registries::get().add_extra_producer(Box::new(p));
}

/// Describes something that can expand prometheus metrics but appending
/// them in a text format to a provided buffer.
pub trait ExtraProducer: Send + Sync {
    /// Takes a buffer and appends prometheus metrics in text format into it.
    fn produce(&self, buffer: &mut Vec<u8>);
}

impl<F> ExtraProducer for F
where
    F: Fn(&mut Vec<u8>) + Send + Sync,
{
    fn produce(&self, buffer: &mut Vec<u8>) {
        self(buffer)
    }
}
