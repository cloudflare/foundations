use std::borrow::Cow;

use foundations_metrics_registry::proto::{Exemplar, LabelPair, Metric, MetricFamily, MetricType};

use crate::diagnostics::report_collect_error;

pub(crate) const NAME_REQUIREMENT: &str = "a non-empty UTF-8 string without NUL bytes";

#[derive(Clone, Copy)]
pub(crate) enum ValidationContext {
    Collection,
    TextEncoding,
    ProtobufEncoding,
}

impl ValidationContext {
    fn action(self) -> &'static str {
        match self {
            Self::Collection => "collecting metrics",
            Self::TextEncoding => "encoding OpenMetrics text",
            Self::ProtobufEncoding => "encoding Prometheus protobuf",
        }
    }
}

pub(crate) fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('\0')
}

pub(crate) fn sanitize_metric_family(
    family: &mut MetricFamily,
    context: ValidationContext,
) -> bool {
    let Some(name) = family.name.as_deref().filter(|name| is_valid_name(name)) else {
        report_invalid_family_name(context, family.name.as_deref());
        return false;
    };

    sanitize_rows(
        &mut family.metric,
        family
            .r#type
            .and_then(|value| MetricType::try_from(value).ok()),
        name,
        context,
    );
    true
}

pub(crate) fn sanitized_metric_family<'a>(
    family: &'a MetricFamily,
    context: ValidationContext,
) -> Option<Cow<'a, MetricFamily>> {
    let Some(name) = family.name.as_deref().filter(|name| is_valid_name(name)) else {
        report_invalid_family_name(context, family.name.as_deref());
        return None;
    };
    let metric_type = family
        .r#type
        .and_then(|value| MetricType::try_from(value).ok());

    if rows_are_valid(&family.metric, metric_type) {
        return Some(Cow::Borrowed(family));
    }

    let mut sanitized = family.clone();
    sanitize_rows(&mut sanitized.metric, metric_type, name, context);
    Some(Cow::Owned(sanitized))
}

fn report_invalid_family_name(context: ValidationContext, name: Option<&str>) {
    report_collect_error(format_args!(
        "non-fatal error while {}: skipped metric family with invalid name {name:?}; expected {NAME_REQUIREMENT}",
        context.action()
    ));
}

fn rows_are_valid(metrics: &[Metric], metric_type: Option<MetricType>) -> bool {
    let reserved_label = reserved_row_label(metric_type);
    metrics.iter().all(|metric| {
        find_label_issue(&metric.label, reserved_label).is_none() && exemplars_are_valid(metric)
    })
}

fn sanitize_rows(
    metrics: &mut Vec<Metric>,
    metric_type: Option<MetricType>,
    family_name: &str,
    context: ValidationContext,
) {
    let reserved_label = reserved_row_label(metric_type);
    metrics.retain_mut(|metric| {
        if let Some(issue) = find_label_issue(&metric.label, reserved_label) {
            report_row_drop(context, family_name, issue);
            return false;
        }

        sanitize_exemplars(metric, family_name, context);
        true
    });
}

fn reserved_row_label(metric_type: Option<MetricType>) -> Option<&'static str> {
    match metric_type {
        Some(MetricType::Histogram | MetricType::GaugeHistogram) => Some("le"),
        Some(MetricType::Summary) => Some("quantile"),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum LabelIssue<'a> {
    Invalid(Option<&'a str>),
    Duplicate(&'a str),
    Reserved(&'a str),
}

fn find_label_issue<'a>(
    labels: &'a [LabelPair],
    reserved_label: Option<&str>,
) -> Option<LabelIssue<'a>> {
    for (index, label) in labels.iter().enumerate() {
        let Some(name) = label.name.as_deref().filter(|name| is_valid_name(name)) else {
            return Some(LabelIssue::Invalid(label.name.as_deref()));
        };

        if labels[..index]
            .iter()
            .any(|previous| previous.name.as_deref() == Some(name))
        {
            return Some(LabelIssue::Duplicate(name));
        }
        if reserved_label == Some(name) {
            return Some(LabelIssue::Reserved(name));
        }
    }

    None
}

fn report_row_drop(context: ValidationContext, family_name: &str, issue: LabelIssue<'_>) {
    match issue {
        LabelIssue::Invalid(name) => report_collect_error(format_args!(
            "non-fatal error while {}: skipped row in metric family {family_name:?} with invalid label name {name:?}; expected {NAME_REQUIREMENT}",
            context.action()
        )),
        LabelIssue::Duplicate(name) => report_collect_error(format_args!(
            "non-fatal error while {}: skipped row in metric family {family_name:?} with duplicate label name {name:?}",
            context.action()
        )),
        LabelIssue::Reserved(name) => report_collect_error(format_args!(
            "non-fatal error while {}: skipped row in metric family {family_name:?}; label name {name:?} is reserved for this metric type",
            context.action()
        )),
    }
}

fn exemplars_are_valid(metric: &Metric) -> bool {
    metric
        .counter
        .as_ref()
        .and_then(|counter| counter.exemplar.as_ref())
        .is_none_or(exemplar_is_valid)
        && metric.histogram.as_ref().is_none_or(|histogram| {
            histogram
                .bucket
                .iter()
                .all(|bucket| bucket.exemplar.as_ref().is_none_or(exemplar_is_valid))
                && histogram.exemplars.iter().all(exemplar_is_valid)
        })
}

fn exemplar_is_valid(exemplar: &Exemplar) -> bool {
    find_label_issue(&exemplar.label, None).is_none()
}

fn sanitize_exemplars(metric: &mut Metric, family_name: &str, context: ValidationContext) {
    if let Some(counter) = &mut metric.counter {
        sanitize_exemplar_slot(&mut counter.exemplar, "counter", family_name, context);
    }

    if let Some(histogram) = &mut metric.histogram {
        for bucket in &mut histogram.bucket {
            sanitize_exemplar_slot(
                &mut bucket.exemplar,
                "classic histogram bucket",
                family_name,
                context,
            );
        }
        histogram.exemplars.retain(|exemplar| {
            if let Some(issue) = find_label_issue(&exemplar.label, None) {
                report_exemplar_drop(context, family_name, "native histogram", issue);
                false
            } else {
                true
            }
        });
    }
}

fn sanitize_exemplar_slot(
    exemplar: &mut Option<Exemplar>,
    kind: &str,
    family_name: &str,
    context: ValidationContext,
) {
    let Some(issue) = exemplar
        .as_ref()
        .and_then(|exemplar| find_label_issue(&exemplar.label, None))
    else {
        return;
    };

    report_exemplar_drop(context, family_name, kind, issue);
    *exemplar = None;
}

fn report_exemplar_drop(
    context: ValidationContext,
    family_name: &str,
    kind: &str,
    issue: LabelIssue<'_>,
) {
    match issue {
        LabelIssue::Invalid(name) => report_collect_error(format_args!(
            "non-fatal error while {}: dropped {kind} exemplar in metric family {family_name:?} with invalid label name {name:?}; expected {NAME_REQUIREMENT}",
            context.action()
        )),
        LabelIssue::Duplicate(name) => report_collect_error(format_args!(
            "non-fatal error while {}: dropped {kind} exemplar in metric family {family_name:?} with duplicate label name {name:?}",
            context.action()
        )),
        LabelIssue::Reserved(_) => unreachable!("exemplar labels have no reserved names"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_non_empty_utf8_metric_and_label_names() {
        for name in [
            "é",
            "aλ",
            "metric name",
            "metric\nname",
            "metric#name",
            "metric\"name",
            "指标.名称",
        ] {
            assert!(is_valid_name(name), "metric name {name:?}");
            assert!(is_valid_name(name), "label name {name:?}");
        }
    }

    #[test]
    fn rejects_empty_and_nul_names() {
        for name in ["", "nul\0name"] {
            assert!(!is_valid_name(name));
            assert!(!is_valid_name(name));
        }
    }
}
