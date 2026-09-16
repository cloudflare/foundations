#[cfg(feature = "telemetry-otlp-grpc")]
use crate::ServiceInfo;
use opentelemetry_proto::tonic as otlp;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn service_name_attribute(service_name: &str) -> otlp::common::v1::KeyValue {
    otlp::common::v1::KeyValue {
        key: "service.name".to_string(),
        value: Some(otlp::common::v1::AnyValue {
            value: Some(otlp::common::v1::any_value::Value::StringValue(
                service_name.to_string(),
            )),
        }),
    }
}

#[cfg(feature = "user-tracing")]
pub(super) fn convert_service_name_to_resource(service_name: &str) -> otlp::resource::v1::Resource {
    otlp::resource::v1::Resource {
        attributes: vec![service_name_attribute(service_name)],
        dropped_attributes_count: 0,
        entity_refs: vec![],
    }
}

#[cfg(feature = "telemetry-otlp-grpc")]
pub(super) fn convert_service_info_to_resource(
    service_info: &ServiceInfo,
) -> otlp::resource::v1::Resource {
    let service_version = otlp::common::v1::KeyValue {
        key: "service.version".to_string(),
        value: Some(otlp::common::v1::AnyValue {
            value: Some(otlp::common::v1::any_value::Value::StringValue(
                service_info.version.to_string(),
            )),
        }),
    };

    otlp::resource::v1::Resource {
        attributes: vec![service_name_attribute(service_info.name), service_version],
        dropped_attributes_count: 0,
        entity_refs: vec![],
    }
}

pub(super) fn convert_time(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .as_ref()
        .map(Duration::as_nanos)
        .unwrap_or_default() as u64
}
