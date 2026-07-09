use std::fs;
use std::path::{Path, PathBuf};

const PROTO_PATH: &str = "proto/metrics.proto";
const GENERATED_PATH: &str = "src/proto/model.rs";
const PROST_OUTPUT_FILE: &str = "io.prometheus.client.rs";

#[test]
fn generated_proto_matches_vendored_proto() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let checked_in_path = crate_dir.join(GENERATED_PATH);
    let generated = generate_proto(&crate_dir);

    let checked_in = fs::read_to_string(&checked_in_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", checked_in_path.display()));

    if generated == checked_in {
        return;
    }

    if std::env::var_os("CI").is_some() {
        panic!(
            "{GENERATED_PATH} is out of date; run `cargo test -p foundations-metrics-registry generated_proto_matches_vendored_proto` locally and commit the updated file."
        );
    }

    fs::write(&checked_in_path, generated)
        .unwrap_or_else(|e| panic!("failed to update {}: {e}", checked_in_path.display()));

    panic!("{GENERATED_PATH} was regenerated; commit the updated file and rerun this test");
}

fn generate_proto(crate_dir: &Path) -> String {
    let out_dir = tempfile::tempdir().expect("failed to create temporary output directory");
    let proto_path = crate_dir.join(PROTO_PATH);
    let include_path = crate_dir.join("proto");

    let file_descriptor_set = protox::compile([proto_path.as_path()], [include_path.as_path()])
        .expect("failed to compile protobuf descriptors");

    prost_build::Config::new()
        .out_dir(out_dir.path())
        .compile_fds(file_descriptor_set)
        .expect("failed to generate Rust protobuf model");

    fs::read_to_string(out_dir.path().join(PROST_OUTPUT_FILE))
        .expect("prost-build did not write the expected generated file")
}
