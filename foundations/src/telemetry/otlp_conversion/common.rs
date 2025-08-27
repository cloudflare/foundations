use crate::ServiceInfo;
use opentelemetry_proto::tonic as otlp;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) fn convert_service_info_to_instrumentation_scope(
    service_info: &ServiceInfo,
) -> otlp::common::v1::InstrumentationScope {
    otlp::common::v1::InstrumentationScope {
        name: service_info.name.to_string(),
        version: service_info.version.to_string(),
        ..Default::default()
    }
}

pub(super) fn convert_service_info_to_resource(
    service_info: &ServiceInfo,
) -> otlp::resource::v1::Resource {
    let service_name_attr = otlp::common::v1::KeyValue {
        key: "service.name".to_string(),
        value: Some(otlp::common::v1::AnyValue {
            value: Some(otlp::common::v1::any_value::Value::StringValue(
                service_info.name.to_string(),
            )),
        }),
    };

    otlp::resource::v1::Resource {
        attributes: vec![service_name_attr],
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
