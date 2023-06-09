fn main() {
    #[cfg(all(
        feature = "seccomp",
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        use bindgen::{Builder, CargoCallbacks};
        use std::env;
        use std::path::PathBuf;

        println!("cargo:rerun-if-changed=build.rs");

        Builder::default()
            .header("/usr/include/seccomp.h")
            .allowlist_function("seccomp_rule_add")
            .derive_default(true)
            .parse_callbacks(Box::new(CargoCallbacks))
            .generate()
            .unwrap()
            .write_to_file(PathBuf::from(env::var("OUT_DIR").unwrap()).join("seccomp_bindings.rs"))
            .unwrap();
    }
}
