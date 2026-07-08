//! Generates the `io.prometheus.client` protobuf model from the vendored
//! `proto/metrics.proto` into `OUT_DIR`, where `src/proto/model.rs` includes it.

use std::error::Error;

const PROTO_PATH: &str = "proto/metrics.proto";
const INCLUDE: &str = "proto";

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={PROTO_PATH}");
    println!("cargo:rerun-if-changed={INCLUDE}");

    let file_descriptor_set = protox::compile([PROTO_PATH], [INCLUDE])?;
    prost_build::Config::new().compile_fds(file_descriptor_set)?;

    Ok(())
}
