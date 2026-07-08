//! Raw `prost`-generated Prometheus protobuf types
//!
//! The code is generated at build time into `$OUT_DIR/io.prometheus.client.rs`
//! from the vendored `proto/metrics.proto` and included here. Prefer the
//! re-exports from the parent [`proto`](crate::proto) module over reaching into this one.

include!(concat!(env!("OUT_DIR"), "/io.prometheus.client.rs"));
