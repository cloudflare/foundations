use foundations_metrics_registry::EncodeMetric;
use foundations_metrics_registry::MetricFamily;

use crate::value::EncodeMetricValue;

/// A metric paired with the name and help text it is exported under.
///
/// Storage types (`Counter`, `Gauge`, ...) hold only their value and encode
/// themselves with relative names (a suffix such as `""`, `_min`, or `_max`).
/// `NamedMetric` supplies the registered base name and help text, bridging the
/// internal `EncodeMetricValue` storage trait to the public
/// [`EncodeMetric`](foundations_metrics_registry::EncodeMetric) registry trait.
pub struct NamedMetric<M> {
    name: &'static str,
    help: &'static str,
    metric: M,
}

impl<M> NamedMetric<M> {
    /// Wraps `metric` with the `name` and `help` it is exported under.
    pub fn new(name: &'static str, help: &'static str, metric: M) -> Self {
        Self { name, help, metric }
    }
}

impl<M> EncodeMetric for NamedMetric<M>
where
    M: EncodeMetricValue,
{
    fn encode(&self) -> Vec<MetricFamily> {
        let mut families = self.metric.encode_metric_value();

        for family in &mut families {
            let suffix = family.name.take().unwrap_or_default();
            family.name = Some(format!("{}{}", self.name, suffix));

            if family.help.is_none() {
                family.help = Some(self.help.to_owned());
            }
        }

        families
    }
}

#[cfg(test)]
mod tests {
    use foundations_metrics_registry::proto::MetricType;

    use super::*;
    use crate::Counter;

    #[test]
    fn rewrites_relative_name_and_fills_help() {
        let counter = Counter::<u64>::default();
        let named = NamedMetric::new(
            "http_requests_total",
            "Number of requests.",
            counter.clone(),
        );

        counter.inc_by(5);

        let families = named.encode();
        assert_eq!(families.len(), 1);

        let family = &families[0];
        assert_eq!(family.name.as_deref(), Some("http_requests_total"));
        assert_eq!(family.help.as_deref(), Some("Number of requests."));
        assert_eq!(family.r#type, Some(MetricType::Counter as i32));
        assert_eq!(
            family.metric[0].counter.as_ref().and_then(|c| c.value),
            Some(5.0)
        );
    }
}
