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
use std::fmt::Display;

#[cfg(not(feature = "foundations-metrics-backend"))]
use prometheus::{Encoder, TextEncoder};
#[cfg(not(feature = "foundations-metrics-backend"))]
use serde::Serialize;
#[cfg(not(feature = "foundations-metrics-backend"))]
use std::any::TypeId;

// Aliased so it does not shadow the glob-re-exported `backend::ServiceNameFormat`.
#[cfg(feature = "foundations-metrics-backend")]
use super::settings::ServiceNameFormat as SettingsServiceNameFormat;
#[cfg(feature = "foundations-metrics-backend")]
use std::sync::OnceLock;

#[cfg(not(feature = "foundations-metrics-backend"))]
mod gauge;
#[cfg(not(feature = "foundations-metrics-backend"))]
mod rewind;

pub(super) mod init;

#[doc(hidden)]
#[cfg(not(feature = "foundations-metrics-backend"))]
pub mod internal;

#[doc(hidden)]
#[cfg(feature = "foundations-metrics-backend")]
#[path = "internal_backend.rs"]
pub mod internal;

#[cfg(not(feature = "foundations-metrics-backend"))]
use internal::{ErasedInfoMetric, Registries};

#[cfg(feature = "foundations-metrics-backend")]
mod backend {
    pub use foundations_metrics::{
        Counter, Family, Gauge, GaugeGuard, Histogram, HistogramTimer, InfoMetric,
        MetricConstructor, NativeHistogram, NativeHistogramBuilder, RangeGauge, TimeHistogram,
        WithExemplar,
    };

    // Everything needed to define, register, and label a custom metric. The
    // protobuf data model is re-exported as `proto` so that implementors do not
    // need a direct dependency on `foundations-metrics-registry`.
    pub use foundations_metrics::{
        EncodeMetric, EncodeMetricValue, IntoMetrics, LabelError, MetricFamily, NamedMetric,
        RegistrationMetadata, proto, register, to_label_pairs,
    };
}
#[cfg(not(feature = "foundations-metrics-backend"))]
mod backend {
    pub use super::gauge::{GaugeGuard, RangeGauge};
    pub use prometheus_client::metrics::exemplar::{CounterWithExemplar, HistogramWithExemplars};
    pub use prometheus_client::metrics::family::MetricConstructor;
    pub use prometheus_client::metrics::gauge::Gauge;
    pub use prometheus_client::metrics::histogram::Histogram;
    pub use prometools::histogram::{HistogramTimer, TimeHistogram};
    pub use prometools::nonstandard::NonstandardUnsuffixedCounter as Counter;
    pub use prometools::serde::Family;
}

pub use backend::*;

/// Translates telemetry settings into collection options.
///
/// The service name is only known once telemetry is initialized, so it is
/// applied at collection time rather than at registration time.
#[cfg(feature = "foundations-metrics-backend")]
fn collection_options(settings: &MetricsSettings) -> foundations_metrics::CollectionOptions<'_> {
    let service_name_format = match &settings.service_name_format {
        SettingsServiceNameFormat::MetricPrefix => {
            foundations_metrics::ServiceNameFormat::MetricPrefix
        }
        SettingsServiceNameFormat::LabelWithName(label_name) => {
            foundations_metrics::ServiceNameFormat::LabelWithName(label_name)
        }
    };

    foundations_metrics::CollectionOptions {
        include_optional: settings.report_optional,
        service_name: Some(init::service_name()),
        service_name_format,
    }
}

/// Collects all metrics in [Prometheus text format].
///
/// [Prometheus text format]: https://prometheus.io/docs/instrumenting/exposition_formats/#text-based-format
pub fn collect(settings: &MetricsSettings) -> Result<String> {
    let mut buffer: Vec<u8> = Vec::with_capacity(128);

    #[cfg(not(feature = "foundations-metrics-backend"))]
    {
        Registries::collect(&mut buffer, settings.report_optional)?;
        TextEncoder::new().encode(&prometheus::gather(), &mut buffer)?;
    }

    #[cfg(feature = "foundations-metrics-backend")]
    {
        let families = foundations_metrics::collect(collection_options(settings));

        buffer.extend_from_slice(foundations_metrics::encode_to_text(&families).as_bytes());

        // Extra producers append their own terminated output, so the terminator
        // is dropped here and re-added once everything has been produced.
        truncate_eof(&mut buffer);

        #[allow(deprecated)]
        if let Some(producers) = EXTRA_PRODUCERS.get() {
            for producer in producers.read().iter() {
                producer.produce(&mut buffer);
                truncate_eof(&mut buffer);
            }
        }
    }

    buffer.extend_from_slice(b"# EOF\n");

    let metrics_str = String::from_utf8(buffer).unwrap_or_else(|err| {
        report_nonfatal_collect_error(&format_args!("converting raw metrics to string: {err}"));
        String::from_utf8_lossy(err.as_bytes()).into_owned()
    });
    Ok(metrics_str)
}

const LEGACY_CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";

/// Wire format a scraper asked for.
#[cfg(feature = "foundations-metrics-backend")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScrapeFormat {
    /// Length-delimited Prometheus protobuf, the only format able to carry
    /// native histograms.
    Protobuf,

    /// OpenMetrics text, optionally permitted to quote UTF-8 names.
    Text { utf8_names: bool },
}

#[cfg(feature = "foundations-metrics-backend")]
impl ScrapeFormat {
    /// The format assumed when a scraper expresses no usable preference.
    const fn fallback() -> Self {
        Self::Text { utf8_names: false }
    }

    /// Content type describing what this format produces.
    ///
    /// Each string is owned by the encoder that emits it, so that what is
    /// advertised cannot drift from what is produced.
    fn content_type(self) -> &'static str {
        match self {
            Self::Protobuf => foundations_metrics::PROTOBUF_CONTENT_TYPE,
            Self::Text { utf8_names: true } => foundations_metrics::OPENMETRICS_CONTENT_TYPE,
            Self::Text { utf8_names: false } => LEGACY_CONTENT_TYPE,
        }
    }
}

/// Chooses the most preferred format that the active encoder can produce, given
/// the value of a request's `Accept` header.
///
/// `None` means the header ruled out every format this server can produce. An
/// absent header expresses no preference and yields the fallback instead.
#[cfg(feature = "foundations-metrics-backend")]
fn negotiate(accept: Option<&str>, protobuf_available: bool) -> Option<ScrapeFormat> {
    let Some(accept) = accept else {
        return Some(ScrapeFormat::fallback());
    };

    let mut best: Option<(f32, ScrapeFormat)> = None;

    for range in accept.split(',') {
        let mut parts = range.split(';').map(str::trim);
        let Some(media_type) = parts.next().filter(|media| !media.is_empty()) else {
            continue;
        };

        let mut quality = 1.0f32;
        let (mut escaping, mut proto, mut encoding) = (None, None, None);

        for parameter in parts {
            let Some((name, value)) = parameter.split_once('=') else {
                continue;
            };

            let value = value.trim().trim_matches('"');
            let name = name.trim();

            if name.eq_ignore_ascii_case("q") {
                quality = match value.parse::<f32>() {
                    Ok(parsed) if (0.0..=1.0).contains(&parsed) => parsed,
                    _ => 0.0,
                };
            } else if name.eq_ignore_ascii_case("escaping") {
                escaping = Some(value);
            } else if name.eq_ignore_ascii_case("proto") {
                proto = Some(value);
            } else if name.eq_ignore_ascii_case("encoding") {
                encoding = Some(value);
            }
        }

        // `q=0` refuses a format outright rather than ranking it last.
        if quality <= 0.0 {
            continue;
        }

        let format = if media_type.eq_ignore_ascii_case("application/vnd.google.protobuf") {
            // Only delimited streams of this message type are produced, and only
            // when protobuf can carry the whole exposition.
            if protobuf_available
                && proto.is_some_and(|proto| proto == "io.prometheus.client.MetricFamily")
                && encoding.is_some_and(|encoding| encoding.eq_ignore_ascii_case("delimited"))
            {
                ScrapeFormat::Protobuf
            } else {
                continue;
            }
        } else if media_type.eq_ignore_ascii_case("application/openmetrics-text") {
            ScrapeFormat::Text {
                utf8_names: escaping
                    .is_some_and(|escaping| escaping.eq_ignore_ascii_case("allow-utf-8")),
            }
        } else if media_type.eq_ignore_ascii_case("text/plain") || media_type == "*/*" {
            ScrapeFormat::Text { utf8_names: false }
        } else {
            continue;
        };

        // Highest quality wins; ties keep the earliest listed.
        if best.is_none_or(|(best_quality, _)| quality > best_quality) {
            best = Some((quality, format));
        }
    }

    best.map(|(_, format)| format)
}

/// Reports whether protobuf can represent everything this process exposes.
///
/// Extra producers hand over opaque text, which cannot be transcoded into the
/// protobuf data model, so serving protobuf while any are registered would
/// silently drop every series they emit.
///
/// Evaluated per scrape because producers may be registered at any point.
#[cfg(feature = "foundations-metrics-backend")]
#[allow(deprecated)]
fn protobuf_available() -> bool {
    EXTRA_PRODUCERS
        .get()
        .is_none_or(|producers| producers.read().is_empty())
}

/// Collects metrics in the format a scraper asked for through its `Accept` header.
///
/// The content type is returned alongside the body so that the two cannot
/// disagree; deriving one separately from the other is how a response ends up
/// declaring a format it did not encode.
///
/// An `Accept` header that rules out every available format is served the
/// fallback text format, after logging a warning.
///
/// The negotiated escaping is currently reported rather than enforced: the text
/// encoder quotes a name whenever that name requires it, regardless of what the
/// scraper asked for.
pub fn collect_negotiated(
    accept: Option<&str>,
    settings: &MetricsSettings,
) -> Result<(&'static str, Vec<u8>)> {
    #[cfg(feature = "foundations-metrics-backend")]
    {
        let format = negotiate(accept, protobuf_available()).unwrap_or_else(|| {
            let fallback = ScrapeFormat::fallback();

            // Only a header that was present can rule everything out, so the
            // default here stands in for a case `negotiate` never reports.
            report_unsatisfiable_accept(accept.unwrap_or_default(), fallback.content_type());

            fallback
        });

        // Protobuf is only reachable with no extra producers registered, so the
        // registry holds everything and skipping them here loses nothing.
        if format == ScrapeFormat::Protobuf {
            let families = foundations_metrics::collect(collection_options(settings));

            return Ok((
                format.content_type(),
                foundations_metrics::encode_to_protobuf(&families),
            ));
        }

        Ok((format.content_type(), collect(settings)?.into_bytes()))
    }

    #[cfg(not(feature = "foundations-metrics-backend"))]
    {
        let _ = accept;

        Ok((LEGACY_CONTENT_TYPE, collect(settings)?.into_bytes()))
    }
}

/// Warns that a scrape's `Accept` header ruled out every available format.
///
/// Not routed through [`report_nonfatal_collect_error`], because collection
/// itself succeeded and the actionable detail is the header.
#[cfg(feature = "foundations-metrics-backend")]
fn report_unsatisfiable_accept(accept: &str, served: &str) {
    #[cfg(feature = "logging")]
    crate::telemetry::log::warn!(
        "no requested metrics format can be served, responding with the fallback instead";
        "accept" => accept,
        "served" => served,
    );

    #[cfg(not(feature = "logging"))]
    eprintln!(
        "no requested metrics format can be served, responding with the fallback instead: \
         accept={accept:?} served={served:?}"
    );
}

/// Removes the trailing OpenMetrics terminator, if present.
#[cfg(feature = "foundations-metrics-backend")]
fn truncate_eof(buffer: &mut Vec<u8>) {
    const EOF_MARKER: &[u8] = b"# EOF\n";

    if buffer.ends_with(EOF_MARKER) {
        buffer.truncate(buffer.len() - EOF_MARKER.len());
    }
}

#[inline]
#[track_caller]
fn report_nonfatal_collect_error(err: &dyn Display) {
    #[cfg(feature = "logging")]
    crate::telemetry::log::warn!("non-fatal error while collecting metrics"; "error" => %err);

    #[cfg(not(feature = "logging"))]
    eprintln!("non-fatal error while collecting metrics: {err}");
}

/// A macro that allows to define Prometheus metrics.
///
/// The macro is a proc macro attribute that should be put on a module containing
/// bodyless functions. Each bodyless function corresponds to a single metric, whose
/// name becomes `<global prefix>_<module name>_<bodyless function name>`and function's
/// Rust doc comment is reported as metric description to Prometheus.
///
/// The `<global_prefix>` can be disabled by passing the `unprefixed` flag to the macro
/// invocation, like `#[metrics(unprefixed)]`. The module name is a mandatory prefix.
///
/// # Labels
/// Arguments of the bodyless functions become labels for that metric.
///
/// Supported metric types are reexported from this module for convenience:
///
/// * [`Counter`]
/// * [`Gauge`]
/// * [`Histogram`]
/// * [`TimeHistogram`]
///
/// To attach exemplars, wrap any of the above in [`WithExemplar<T, S>`], which
/// derefs to the metric it wraps.
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
/// `#[ctor]` allows specifying how the metric should be built (e.g. [`HistogramBuilder`]).
/// The constructor should implement [`MetricConstructor`] for the metric type.
///
/// ## `#[optional]`
///
/// Metrics marked with `#[optional]` are collected in a separate registry and reported only if
/// `collect_optional` argument of [`collect`] is set to `true`, or, in case the [telemetry server]
/// is used, if [`MetricsSettings::report_optional`] is set to `true`.
///
/// Can be used for heavy-weight metrics (e.g. with high cardinality) that don't need to be reported
/// on a regular basis.
///
/// ## `#[with_removal]` (unstable)
///
/// **This feature is unstable and becomes a noop without `cfg(foundations_unstable)`.**
///
/// Metrics with labels make up a shared [`Family`]. Occasionally, it can be useful to
/// remove one or all existing metrics from a family. This functionality is provided by
/// the `#[with_removal]` attribute. Single metrics (without labels) do not support this
/// argument.
///
/// If the attribute is present on a metric function, two additional functions are
/// generated in addition to the metric itself. These are called `<metric>_remove` and
/// `<metric>_clear`. The `_remove` variant takes the same arguments as the original
/// function and removes that instance from the family. It returns a boolean indicating
/// whether the labels were present before. The `_clear` variant takes no arguments
/// and removes all existing metrics from the family.
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
///
///     /// Number of HTTP requests
///     #[with_removal]
///     pub fn requests_total(endpoint: &Arc<String>) -> Counter;
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
///     my_app_metrics::requests_total(&endpoint).inc();
///
///     client_connections_active.dec();
///
/// #   #[cfg(foundations_unstable)] {
///     my_app_metrics::requests_total_remove(&endpoint);
///     // Or remove all existing instances:
///     my_app_metrics::requests_total_clear();
/// #   }
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
#[cfg(not(feature = "foundations-metrics-backend"))]
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
    #[cfg(not(feature = "foundations-metrics-backend"))]
    {
        Registries::get().info.write().insert(
            TypeId::of::<M>(),
            info_metric.into() as Box<dyn ErasedInfoMetric>,
        );
    }

    #[cfg(feature = "foundations-metrics-backend")]
    {
        foundations_metrics::report_info(info_metric);
    }
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

#[cfg(not(feature = "foundations-metrics-backend"))]
impl<S> MetricConstructor<HistogramWithExemplars<S>> for HistogramBuilder {
    fn new_metric(&self) -> HistogramWithExemplars<S> {
        HistogramWithExemplars::new(self.buckets.iter().cloned())
    }
}

// The `foundations-metrics` backend replaces the per-type exemplar wrappers
// with a single generic `WithExemplar<T, S>`, so the builder constructs that
// instead.
#[cfg(feature = "foundations-metrics-backend")]
impl<S> MetricConstructor<WithExemplar<Histogram, S>> for HistogramBuilder {
    fn new_metric(&self) -> WithExemplar<Histogram, S> {
        WithExemplar::new(MetricConstructor::<Histogram>::new_metric(self))
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
/// # #[allow(deprecated)]
/// foundations::telemetry::metrics::add_extra_producer(move |buffer: &mut Vec<u8>| {
///     prometheus_client::encoding::text::encode(buffer, &registry).unwrap();
/// });
/// ```
#[deprecated = "Text output bypasses validation and cannot be encoded as protobuf. Implement `EncodeMetric` and pass it to `register` instead, enabling the `foundations-metrics-backend` feature if it is disabled."]
// TODO: remove before next major release
#[allow(deprecated)]
pub fn add_extra_producer<P>(p: P)
where
    P: ExtraProducer + 'static,
{
    #[cfg(not(feature = "foundations-metrics-backend"))]
    Registries::get().add_extra_producer(Box::new(p));

    #[cfg(feature = "foundations-metrics-backend")]
    EXTRA_PRODUCERS
        .get_or_init(Default::default)
        .write()
        .push(Box::new(p));
}

/// Producers appended to the collected metrics, in registration order.
#[cfg(feature = "foundations-metrics-backend")]
#[allow(deprecated)]
static EXTRA_PRODUCERS: OnceLock<parking_lot::RwLock<Vec<Box<dyn ExtraProducer>>>> =
    OnceLock::new();

/// Describes something that can expand prometheus metrics but appending
/// them in a text format to a provided buffer.
#[deprecated = "Text output bypasses validation and cannot be encoded as protobuf. Implement `EncodeMetric` instead, enabling the `foundations-metrics-backend` feature if it is disabled."]
// TODO: remove before next major release
pub trait ExtraProducer: Send + Sync {
    /// Takes a buffer and appends prometheus metrics in text format into it.
    fn produce(&self, buffer: &mut Vec<u8>);
}

#[allow(deprecated)]
impl<F> ExtraProducer for F
where
    F: Fn(&mut Vec<u8>) + Send + Sync,
{
    fn produce(&self, buffer: &mut Vec<u8>) {
        self(buffer)
    }
}

#[cfg(all(test, feature = "foundations-metrics-backend"))]
mod negotiation_tests {
    use super::*;

    const TEXT: Option<ScrapeFormat> = Some(ScrapeFormat::Text { utf8_names: false });
    const TEXT_UTF8: Option<ScrapeFormat> = Some(ScrapeFormat::Text { utf8_names: true });
    const PROTOBUF: Option<ScrapeFormat> = Some(ScrapeFormat::Protobuf);

    /// What Prometheus sends unless configured to prefer protobuf.
    const PROMETHEUS_DEFAULT: &str = "application/openmetrics-text;version=1.0.0;q=0.5,\
                                      text/plain;version=0.0.4;q=0.4,*/*;q=0.1";

    const PROTOBUF_PREFERRED: &str = "application/vnd.google.protobuf;\
                                      proto=io.prometheus.client.MetricFamily;\
                                      encoding=delimited;q=0.5,\
                                      application/openmetrics-text;version=1.0.0;q=0.4";

    /// Delimited protobuf and nothing else: no text range, no `*/*`.
    const PROTOBUF_ONLY: &str = "application/vnd.google.protobuf;\
                                 proto=io.prometheus.client.MetricFamily;encoding=delimited";

    #[test]
    fn absent_header_falls_back_to_legacy_text() {
        assert_eq!(negotiate(None, true), TEXT);
    }

    #[test]
    fn prometheus_default_accept_selects_text() {
        assert_eq!(negotiate(Some(PROMETHEUS_DEFAULT), true), TEXT);
    }

    #[test]
    fn utf8_escaping_is_detected() {
        let accept =
            "application/openmetrics-text;version=1.0.0;escaping=allow-utf-8;q=0.5,*/*;q=0.1";

        assert_eq!(negotiate(Some(accept), true), TEXT_UTF8);
    }

    #[test]
    fn delimited_protobuf_wins_when_preferred() {
        assert_eq!(negotiate(Some(PROTOBUF_PREFERRED), true), PROTOBUF);
    }

    #[test]
    fn protobuf_without_delimited_encoding_is_not_offered() {
        let accept = "application/vnd.google.protobuf;\
                      proto=io.prometheus.client.MetricFamily;q=0.9,text/plain;q=0.1";

        assert_eq!(negotiate(Some(accept), true), TEXT);
    }

    #[test]
    fn zero_quality_refuses_a_format() {
        let accept = "application/openmetrics-text;escaping=allow-utf-8;q=0,text/plain;q=0.4";

        assert_eq!(negotiate(Some(accept), true), TEXT);
    }

    #[test]
    fn zero_quality_on_the_only_range_matches_nothing() {
        assert_eq!(
            negotiate(Some("application/openmetrics-text;q=0"), true),
            None
        );
    }

    #[test]
    fn malformed_quality_refuses_a_format() {
        let accept = "application/openmetrics-text;escaping=allow-utf-8;q=garbage,text/plain;q=0.4";

        assert_eq!(negotiate(Some(accept), true), TEXT);
    }

    #[test]
    fn parameters_tolerate_whitespace_case_and_quoting() {
        let accept =
            "  APPLICATION/OpenMetrics-Text ; Version=1.0.0 ; Escaping=\"Allow-UTF-8\" ; Q=0.7 ";

        assert_eq!(negotiate(Some(accept), true), TEXT_UTF8);
    }

    #[test]
    fn ties_keep_the_earliest_listed() {
        let accept = "application/openmetrics-text;escaping=allow-utf-8;q=0.5,text/plain;q=0.5";

        assert_eq!(negotiate(Some(accept), true), TEXT_UTF8);
    }

    #[test]
    fn protobuf_is_withheld_when_unavailable() {
        assert_eq!(negotiate(Some(PROTOBUF_PREFERRED), false), TEXT);
    }

    #[test]
    fn withholding_protobuf_leaves_text_negotiation_untouched() {
        let utf8 = "application/openmetrics-text;escaping=allow-utf-8;q=0.9,text/plain;q=0.1";

        assert_eq!(negotiate(Some(utf8), false), TEXT_UTF8);
        assert_eq!(negotiate(Some(PROMETHEUS_DEFAULT), false), TEXT);
        assert_eq!(negotiate(None, false), TEXT);
    }

    #[test]
    fn protobuf_only_matches_nothing_when_unavailable() {
        assert_eq!(negotiate(Some(PROTOBUF_ONLY), false), None);
    }

    #[test]
    fn protobuf_only_is_served_when_available() {
        assert_eq!(negotiate(Some(PROTOBUF_ONLY), true), PROTOBUF);
    }

    #[test]
    fn unservable_media_types_match_nothing() {
        assert_eq!(negotiate(Some("application/json,text/html"), true), None);
    }

    #[test]
    fn malformed_quality_on_the_only_range_matches_nothing() {
        let accept = "application/vnd.google.protobuf;\
                      proto=io.prometheus.client.MetricFamily;encoding=delimited;q=";

        assert_eq!(negotiate(Some(accept), true), None);
    }

    #[test]
    fn unrankable_quality_matches_nothing() {
        for weight in [
            "nan", "NaN", "+nan", "inf", "infinity", "-inf", "2", "1e3", "-0.5",
        ] {
            let accept = format!("application/openmetrics-text;q={weight}");

            assert_eq!(
                negotiate(Some(&accept), true),
                None,
                "q={weight} should invalidate the range"
            );
        }
    }

    #[test]
    fn unrankable_quality_does_not_mask_a_later_range() {
        let accept = format!("application/openmetrics-text;q=nan,{PROTOBUF_PREFERRED}");

        assert_eq!(negotiate(Some(&accept), true), PROTOBUF);
    }

    #[test]
    fn out_of_range_quality_does_not_outrank_the_maximum() {
        let accept = "application/openmetrics-text;escaping=allow-utf-8;q=5,text/plain;q=1.0";

        assert_eq!(negotiate(Some(accept), true), TEXT);
    }

    #[test]
    fn quality_bounds_are_accepted() {
        assert_eq!(
            negotiate(Some("application/openmetrics-text;q=1.000"), true),
            TEXT
        );
        assert_eq!(
            negotiate(Some("application/openmetrics-text;q=0.001"), true),
            TEXT
        );
    }
}

/// Covers the warning emitted when an `Accept` header cannot be satisfied.
#[cfg(all(test, feature = "foundations-metrics-backend", feature = "logging"))]
mod fallback_logging_tests {
    use super::*;
    use crate::telemetry::TelemetryContext;

    /// Matches nothing on offer whether or not protobuf is available, so it
    /// reaches the fallback in a test binary that has no extra producers.
    const UNSERVABLE: &str = "application/json,text/html";

    #[test]
    fn unsatisfiable_accept_is_served_as_text_with_a_warning() {
        let ctx = TelemetryContext::test();
        let _scope = ctx.scope();

        let (content_type, body) =
            collect_negotiated(Some(UNSERVABLE), &MetricsSettings::default())
                .expect("an unsatisfiable header should still produce a body");

        assert_eq!(content_type, ScrapeFormat::fallback().content_type());
        assert!(
            body.ends_with(b"# EOF\n"),
            "fallback body should be terminated text"
        );

        let records = ctx.log_records();
        let warning = records
            .iter()
            .find(|record| record.message.contains("no requested metrics format"))
            .unwrap_or_else(|| panic!("falling back should warn: {records:?}"));

        assert_eq!(warning.level, slog::Level::Warning);
        assert!(
            warning
                .fields
                .contains(&("accept".to_owned(), UNSERVABLE.to_owned())),
            "the warning should name the header that could not be satisfied: {:?}",
            warning.fields
        );
    }
}
