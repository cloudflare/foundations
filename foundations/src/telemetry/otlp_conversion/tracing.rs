use super::common::{convert_service_info_to_resource, convert_time};
use crate::ServiceInfo;
use cf_rustracing::log::Log;
use cf_rustracing::span::SpanReference;
use cf_rustracing::tag::{Tag, TagValue};
use cf_rustracing_jaeger::span::FinishedSpan;
use cf_rustracing_jaeger::span::SpanContextState;
use opentelemetry_proto::tonic as otlp;

fn convert_trace_id(span_state: &SpanContextState) -> Vec<u8> {
    span_state
        .trace_id()
        .high
        .to_be_bytes()
        .into_iter()
        .chain(span_state.trace_id().low.to_be_bytes())
        .collect()
}

fn convert_parent_span_id(span: &FinishedSpan) -> Vec<u8> {
    span.references()
        .iter()
        .find_map(|r| match r {
            SpanReference::ChildOf(c) => Some(c.span_id().to_be_bytes().to_vec()),
            _ => None,
        })
        .unwrap_or_default()
}

// NOTE: https://www.w3.org/TR/trace-context/#sampled-flag
fn convert_sampled_flag(span_state: &SpanContextState) -> u32 {
    if span_state.is_sampled() {
        0x01
    } else {
        0x00
    }
}

fn convert_tag(tag: &Tag) -> otlp::common::v1::KeyValue {
    otlp::common::v1::KeyValue {
        key: tag.name().to_string(),
        value: Some(otlp::common::v1::AnyValue {
            value: Some(match tag.value() {
                TagValue::Boolean(v) => otlp::common::v1::any_value::Value::BoolValue(*v),
                TagValue::Float(v) => otlp::common::v1::any_value::Value::DoubleValue(*v),
                TagValue::Integer(v) => otlp::common::v1::any_value::Value::IntValue(*v),
                TagValue::String(v) => {
                    otlp::common::v1::any_value::Value::StringValue(v.to_string())
                }
            }),
        }),
    }
}

fn convert_log_entry(log: &Log) -> otlp::trace::v1::span::Event {
    otlp::trace::v1::span::Event {
        name: "Log entry".to_string(),
        time_unix_nano: convert_time(log.time()),
        attributes: log
            .fields()
            .iter()
            .map(|field| otlp::common::v1::KeyValue {
                key: field.name().to_string(),
                value: Some(otlp::common::v1::AnyValue {
                    value: Some(otlp::common::v1::any_value::Value::StringValue(
                        field.value().to_string(),
                    )),
                }),
            })
            .collect(),
        dropped_attributes_count: Default::default(),
    }
}

fn convert_tags(
    span: &FinishedSpan,
) -> (
    otlp::trace::v1::status::StatusCode,
    Vec<otlp::common::v1::KeyValue>,
) {
    let mut status_code = otlp::trace::v1::status::StatusCode::Ok;

    let attributes = span
        .tags()
        .iter()
        .map(|tag| {
            if status_code != otlp::trace::v1::status::StatusCode::Error && tag.name() == "error" {
                status_code = otlp::trace::v1::status::StatusCode::Error;
            }

            convert_tag(tag)
        })
        .collect();

    (status_code, attributes)
}

pub(crate) fn convert_span(
    span: FinishedSpan,
    service_info: &ServiceInfo,
) -> otlp::trace::v1::ResourceSpans {
    let span_state = span.context().state();
    let (status_code, attributes) = convert_tags(&span);

    let status = (status_code == otlp::trace::v1::status::StatusCode::Error).then(|| {
        otlp::trace::v1::Status {
            code: status_code.into(),
            message: Default::default(),
        }
    });

    otlp::trace::v1::ResourceSpans {
        resource: Some(convert_service_info_to_resource(service_info)),
        schema_url: Default::default(),
        scope_spans: vec![otlp::trace::v1::ScopeSpans {
            schema_url: Default::default(),
            scope: None,
            spans: vec![otlp::trace::v1::Span {
                trace_id: convert_trace_id(span_state),
                span_id: span_state.span_id().to_be_bytes().to_vec(),
                trace_state: Default::default(),
                parent_span_id: convert_parent_span_id(&span),
                flags: convert_sampled_flag(span_state),
                name: span.operation_name().to_string(),
                kind: otlp::trace::v1::span::SpanKind::Unspecified as i32,
                start_time_unix_nano: convert_time(span.start_time()),
                end_time_unix_nano: convert_time(span.finish_time()),
                dropped_attributes_count: Default::default(),
                attributes,
                dropped_events_count: Default::default(),
                events: span.logs().iter().map(convert_log_entry).collect(),
                dropped_links_count: Default::default(),
                links: Default::default(),
                status,
            }],
        }],
    }
}
