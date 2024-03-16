use crate::ServiceInfo;
use opentelemetry_proto::tonic as otlp;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) fn convert_service_info(service_info: &ServiceInfo) -> otlp::common::v1::InstrumentationScope {
    otlp::common::v1::InstrumentationScope {
        name: service_info.name.to_string(),
        version: service_info.version.to_string(),
        ..Default::default()
    }
}

pub(super) fn convert_time(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .as_ref()
        .map(Duration::as_nanos)
        .unwrap_or_default() as u64
}