use std::collections::HashMap;

use crossbeam_utils::CachePadded;
use foundations_metrics_registry::proto::{LabelPair, Metric};
use foundations_metrics_registry::{EncodeMetric, MetricFamily, iter};
use parking_lot::{Mutex, MutexGuard};

use crate::info::info_metric_names;

/// Series that haven't changed since [`clear_all`] was last called.
static CLEARED: CachePadded<Mutex<Snapshot>> = CachePadded::new(Mutex::new(Snapshot {
    metrics: Vec::new(),
}));

/// Stops reporting every series until its value changes.
///
/// After this call, collection skips every series whose value is the same as
/// at the time of the call. Once a series' value changes, it's reported again
/// with its current value and stays reported from then on. Series and metrics
/// created after the call are reported as usual, and info metrics are always
/// reported.
///
/// This is meant for a process that is winding down, such as the old instance
/// during a graceful restart. Most of its series stop changing, and exporting
/// them again only adds load to the monitoring system.
///
/// Values aren't reset, so metric handles held elsewhere keep working. Until a
/// series changes, a copy of its value is kept to compare against. Calling this
/// again starts over from the values at that time.
///
/// # Examples
///
/// ```
/// use foundations_metrics::{
///     CollectionOptions, Counter, EncodeMetric, NamedMetric, RegistrationMetadata,
///     ServiceNameFormat, clear_all, collect, register,
/// };
///
/// let requests = Counter::<u64>::default();
/// requests.inc_by(553);
///
/// register(
///     Box::new(NamedMetric::new("requests_total", "Requests.", requests.clone()))
///         as Box<dyn EncodeMetric>,
///     RegistrationMetadata::default(),
/// );
///
/// let options = CollectionOptions {
///     include_optional: false,
///     service_name: None,
///     service_name_format: ServiceNameFormat::MetricPrefix,
/// };
///
/// clear_all();
/// assert!(collect(options).is_empty());
///
/// requests.inc();
/// assert_eq!(collect(options).len(), 1);
/// ```
pub fn clear_all() {
    let always_reported = info_metric_names();
    let snapshot = Snapshot::take(
        iter().map(|registered| encode_settled(registered.metric())),
        &always_reported,
    );

    *CLEARED.lock() = snapshot;
}

/// Locks the rows that haven't changed since [`clear_all`] was called, so a
/// collection can filter every metric under one lock.
pub(crate) fn cleared() -> MutexGuard<'static, Snapshot> {
    CLEARED.lock()
}

/// Encodes `metric` twice and returns the second result.
///
/// Encoding can reset state, like a `RangeGauge`'s min and max, so the second
/// result is what later collections see while nothing changes.
fn encode_settled(metric: &dyn EncodeMetric) -> Vec<MetricFamily> {
    metric.encode();
    metric.encode()
}

/// The rows each registered metric had when [`clear_all`] was called, by
/// registry position. Rows are forgotten once they change.
pub(crate) struct Snapshot {
    metrics: Vec<MetricSnapshot>,
}

/// One metric's unchanged rows by family name, then by labels. The labels are
/// moved out of each stored row into its key.
type MetricSnapshot = HashMap<String, HashMap<Vec<LabelPair>, Metric>>;

impl Snapshot {
    fn take(
        encoded: impl IntoIterator<Item = Vec<MetricFamily>>,
        always_reported: &[&str],
    ) -> Self {
        let metrics = encoded
            .into_iter()
            .map(|families| {
                let mut snapshot = MetricSnapshot::new();

                for family in families {
                    let Some(name) = family.name else {
                        continue;
                    };

                    if family.metric.is_empty()
                        || always_reported.iter().any(|reported| *reported == name)
                    {
                        continue;
                    }

                    let rows = snapshot.entry(name).or_default();

                    for mut row in family.metric {
                        let labels = std::mem::take(&mut row.label);
                        rows.insert(labels, row);
                    }
                }

                snapshot
            })
            .collect();

        Self { metrics }
    }

    /// Removes the rows of the metric at registry position `index` that
    /// haven't changed since [`clear_all`] was called.
    pub(crate) fn hide_unchanged(&mut self, index: usize, families: &mut Vec<MetricFamily>) {
        let Some(snapshot) = self.metrics.get_mut(index) else {
            return;
        };

        if snapshot.is_empty() {
            return;
        }

        families.retain_mut(|family| {
            let Some(name) = family.name.as_deref() else {
                return true;
            };

            let Some(rows) = snapshot.get_mut(name) else {
                return true;
            };

            let had_rows = !family.metric.is_empty();

            family.metric.retain(|row| {
                let changed = match rows.get(row.label.as_slice()) {
                    Some(old) => !same_value(row, old),
                    None => return true,
                };

                if changed {
                    rows.remove(row.label.as_slice());
                }

                changed
            });

            // Drop families emptied here, but leave already empty ones alone.
            !had_rows || !family.metric.is_empty()
        });

        snapshot.retain(|_, rows| !rows.is_empty());
    }
}

/// Whether `row` still has the value stored in `old`. Labels aren't compared,
/// because they're the snapshot key. NaN never compares equal, so a series
/// holding NaN is always reported.
fn same_value(row: &Metric, old: &Metric) -> bool {
    // Destructured so that a new field can't be left out by accident.
    let Metric {
        label: _,
        gauge,
        counter,
        summary,
        untyped,
        histogram,
        timestamp_ms,
    } = row;

    *gauge == old.gauge
        && *counter == old.counter
        && *summary == old.summary
        && *untyped == old.untyped
        && *histogram == old.histogram
        && *timestamp_ms == old.timestamp_ms
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::*;
    use crate::{Counter, Family, Gauge, Histogram, NamedMetric, RangeGauge};

    #[derive(Clone, Eq, Hash, PartialEq, Serialize)]
    struct Labels {
        kind: &'static str,
    }

    fn labels(kind: &'static str) -> Labels {
        Labels { kind }
    }

    /// Collects `metric` as if it were registered at position `index`, and
    /// formats each reported row as `name{labels} value`.
    fn reported(snapshot: &mut Snapshot, index: usize, metric: &dyn EncodeMetric) -> Vec<String> {
        let mut families = metric.encode();
        snapshot.hide_unchanged(index, &mut families);

        let mut rows: Vec<String> = families
            .iter()
            .flat_map(|family| family.metric.iter().map(move |row| format_row(family, row)))
            .collect();

        rows.sort();
        rows
    }

    fn format_row(family: &MetricFamily, row: &Metric) -> String {
        let name = family.name.as_deref().unwrap_or_default();
        let value = row
            .counter
            .as_ref()
            .and_then(|counter| counter.value)
            .or_else(|| row.gauge.as_ref().and_then(|gauge| gauge.value))
            .or_else(|| {
                row.histogram
                    .as_ref()
                    .and_then(|histogram| histogram.sample_count)
                    .map(|count| count as f64)
            })
            .expect("row has a value");

        let labels: Vec<String> = row
            .label
            .iter()
            .map(|label| {
                format!(
                    "{}=\"{}\"",
                    label.name.as_deref().unwrap_or_default(),
                    label.value.as_deref().unwrap_or_default()
                )
            })
            .collect();

        if labels.is_empty() {
            format!("{name} {value}")
        } else {
            format!("{name}{{{}}} {value}", labels.join(","))
        }
    }

    #[test]
    fn hides_series_until_they_change() {
        let requests = Family::<Labels, Counter>::default();
        requests.get_or_create(&labels("ok")).inc_by(553);
        requests.get_or_create(&labels("error")).inc_by(12);

        let metric = NamedMetric::new("requests_total", "Requests.", requests.clone());
        let mut snapshot = Snapshot::take([encode_settled(&metric)], &[]);

        assert!(reported(&mut snapshot, 0, &metric).is_empty());

        requests.get_or_create(&labels("ok")).inc();
        assert_eq!(
            reported(&mut snapshot, 0, &metric),
            ["requests_total{kind=\"ok\"} 554"]
        );

        // A series stays reported once it has changed.
        assert_eq!(
            reported(&mut snapshot, 0, &metric),
            ["requests_total{kind=\"ok\"} 554"]
        );
    }

    #[test]
    fn reports_the_current_value_of_a_gauge() {
        let connections = Family::<Labels, Gauge>::default();
        connections.get_or_create(&labels("https")).set(50);

        let metric = NamedMetric::new("connections", "Open connections.", connections.clone());
        let mut snapshot = Snapshot::take([encode_settled(&metric)], &[]);

        assert!(reported(&mut snapshot, 0, &metric).is_empty());

        connections.get_or_create(&labels("https")).dec();
        assert_eq!(
            reported(&mut snapshot, 0, &metric),
            ["connections{kind=\"https\"} 49"]
        );
    }

    #[test]
    fn saved_handles_keep_reporting() {
        let requests = Family::<Labels, Counter>::default();
        let ok = requests.get_or_create_owned(&labels("ok"));
        ok.inc_by(553);

        let metric = NamedMetric::new("requests_total", "Requests.", requests);
        let mut snapshot = Snapshot::take([encode_settled(&metric)], &[]);

        ok.inc_by(100);
        assert_eq!(
            reported(&mut snapshot, 0, &metric),
            ["requests_total{kind=\"ok\"} 653"]
        );
    }

    #[test]
    fn hides_unlabeled_metrics_and_reports_new_series() {
        let reports = Counter::<u64>::default();
        reports.inc_by(7);
        let unlabeled = NamedMetric::new("reports_total", "Reports.", reports.clone());

        let requests = Family::<Labels, Counter>::default();
        requests.get_or_create(&labels("ok")).inc();
        let labeled = NamedMetric::new("requests_total", "Requests.", requests.clone());

        let mut snapshot =
            Snapshot::take([encode_settled(&unlabeled), encode_settled(&labeled)], &[]);

        assert!(reported(&mut snapshot, 0, &unlabeled).is_empty());
        assert!(reported(&mut snapshot, 1, &labeled).is_empty());

        reports.inc();
        assert_eq!(reported(&mut snapshot, 0, &unlabeled), ["reports_total 8"]);

        // A label set created after the snapshot is reported before it changes.
        drop(requests.get_or_create(&labels("new")));
        assert_eq!(
            reported(&mut snapshot, 1, &labeled),
            ["requests_total{kind=\"new\"} 0"]
        );

        // So is a metric registered after the snapshot.
        let later = NamedMetric::new(
            "later_total",
            "Registered later.",
            Counter::<u64>::default(),
        );
        assert_eq!(reported(&mut snapshot, 2, &later), ["later_total 0"]);
    }

    #[test]
    fn always_reports_exempt_families() {
        let build_info = Gauge::<u64>::default();
        build_info.set(1);

        let metric = NamedMetric::new("build_info", "Build information.", build_info);
        let mut snapshot = Snapshot::take([encode_settled(&metric)], &["build_info"]);

        assert_eq!(reported(&mut snapshot, 0, &metric), ["build_info 1"]);
    }

    #[test]
    fn idle_range_gauge_stays_hidden() {
        let inflight = RangeGauge::default();
        inflight.inc_by(3);
        inflight.dec_by(2);

        let metric = NamedMetric::new("inflight", "In-flight requests.", inflight.clone());
        let mut snapshot = Snapshot::take([encode_settled(&metric)], &[]);

        // Its min and max differed from its value when the snapshot was taken.
        assert!(reported(&mut snapshot, 0, &metric).is_empty());

        inflight.inc();
        assert_eq!(
            reported(&mut snapshot, 0, &metric),
            ["inflight 2", "inflight_max 2"]
        );
    }

    #[test]
    fn reports_a_histogram_once_it_records_a_value() {
        let latency = Histogram::new([0.1, 1.0]);
        latency.observe(0.5);

        let metric = NamedMetric::new("latency_seconds", "Latency.", latency.clone());
        let mut snapshot = Snapshot::take([encode_settled(&metric)], &[]);

        assert!(reported(&mut snapshot, 0, &metric).is_empty());

        latency.observe(0.05);
        assert_eq!(reported(&mut snapshot, 0, &metric), ["latency_seconds 2"]);
    }

    #[test]
    fn drops_families_left_without_rows() {
        let metric = NamedMetric::new("reports_total", "Reports.", Counter::<u64>::default());
        let mut snapshot = Snapshot::take([encode_settled(&metric)], &[]);

        let mut families = metric.encode();
        snapshot.hide_unchanged(0, &mut families);

        assert!(families.is_empty());
    }
}
