use std::env;
use std::path::PathBuf;

fn main() {
    ensure_seccomp_sources_fetched();

    #[cfg(feature = "security")]
    security::build();

    percpu::build();
}

fn env_path(key: &'static str) -> PathBuf {
    let Some(v) = env::var_os(key) else {
        panic!("env variable `{key}` should exist in build script");
    };
    v.into()
}

fn ensure_seccomp_sources_fetched() {
    let src_dir = env_path("CARGO_MANIFEST_DIR").join("src/security/libseccomp/src");

    if !src_dir.exists() {
        panic!(
            "Can't find libseccomp sources. Run `git submodule update --init --recursive`. \
            This is required even if `security` feature is disabled or the OS is not Linux, \
            to ensure that sources are always included on publishing."
        );
    }
}

mod percpu {
    use super::*;
    use bindgen::RustEdition;

    pub(super) fn build() {
        const PERCPU_TARGETS: &[&str] = &["aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"];

        let target = env::var("TARGET").unwrap();
        if PERCPU_TARGETS.contains(&&*target) {
            linux_build();
        }
    }

    fn linux_build() {
        let out_path = env_path("OUT_DIR").join("percpu_sys.rs");
        let header = env_path("CARGO_MANIFEST_DIR").join("src/telemetry/metrics/percpu/sys.h");

        let bindings = match bindgen::builder()
            .header(header.to_string_lossy())
            .allowlist_function("get_nprocs_conf")
            .allowlist_item("(__)?rseq_.+")
            .allowlist_item("RSEQ_.+")
            .prepend_enum_name(false)
            .rust_edition(RustEdition::Edition2021)
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            .generate()
        {
            Ok(b) => b,
            Err(e) => {
                println!("cargo:warning=failed to generate percpu bindings: {e}");
                return;
            }
        };

        bindings
            .write_to_file(&out_path)
            .expect("failed to write percpu bindings");
        println!("cargo:rustc-cfg=has_percpu_sys");
    }
}

#[cfg(feature = "security")]
mod security {
    use super::*;
    use bindgen::Builder;
    use std::fs;
    use std::path::Path;

    const SRC_FILES: &[&str] = &[
        "api.c",
        "system.c",
        "gen_pfc.c",
        "gen_bpf.c",
        "hash.c",
        "db.c",
        "arch.c",
        "helper.c",
        "arch-parisc.c",
        "arch-parisc64.c",
        "arch-parisc-syscalls.c",
        "arch-x86.c",
        "arch-x86-syscalls.c",
        "arch-x86_64.c",
        "arch-x86_64-syscalls.c",
        "arch-x32.c",
        "arch-x32-syscalls.c",
        "arch-arm.c",
        "arch-arm-syscalls.c",
        "arch-aarch64.c",
        "arch-aarch64-syscalls.c",
        "arch-mips.c",
        "arch-mips-syscalls.c",
        "arch-mips64.c",
        "arch-mips64-syscalls.c",
        "arch-mips64n32.c",
        "arch-mips64n32-syscalls.c",
        "arch-ppc.c",
        "arch-ppc-syscalls.c",
        "arch-ppc64.c",
        "arch-ppc64-syscalls.c",
        "arch-s390.c",
        "arch-s390-syscalls.c",
        "arch-s390x.c",
        "arch-s390x-syscalls.c",
        "arch-riscv64.c",
        "arch-riscv64-syscalls.c",
    ];

    pub(super) fn build() {
        println!("cargo:rerun-if-changed=build.rs");
        println!("cargo:rerun-if-changed=src/security/libseccomp");

        let target = std::env::var("TARGET").unwrap();

        // `#[cfg(target_*)]` gates in build scripts are always about the host machine, as
        // the resulting program will always run on the host machine building the crate,
        // so we can't use those gates here and must check at runtime the `TARGET` environment
        // variable. This is unfortunate as it means we need to depend on bindgen even
        // when targetting macOS. See https://github.com/rust-lang/cargo/issues/4932.
        if target.contains("linux")
            && !target.contains("android")
            && (target.contains("x86_64") || target.contains("aarch64"))
        {
            linux_build();
        }
    }

    fn linux_build() {
        println!("cargo:rustc-link-lib=static=seccomp");

        let out_dir = env_path("OUT_DIR");
        let crate_root = env_path("CARGO_MANIFEST_DIR");
        let libseccomp_repo = crate_root.join("src/security/libseccomp");
        let include_dir = libseccomp_repo.join("include");
        let src_dir = libseccomp_repo.join("src");
        let header_file = render_header(out_dir.as_path(), include_dir.as_path());
        let mut compiler = cc::Build::new();

        fs::write(out_dir.join("configure.h"), b"").unwrap();

        for src_file in SRC_FILES {
            compiler.file(src_dir.join(src_file));
        }

        if have_linux_seccomp_h(out_dir.as_path()) {
            compiler.define("HAVE_LINUX_SECCOMP_H", Some("1"));
        }

        compiler
            .warnings(false)
            .include(include_dir)
            .include(&out_dir)
            .compile("seccomp");

        gen_security_sys_rs(
            crate_root.as_path(),
            out_dir.as_path(),
            header_file.as_path(),
        );
    }

    fn render_header(out_dir: &Path, include_dir: &Path) -> PathBuf {
        let template = include_dir.join("seccomp.h.in");
        let rendered = fs::read_to_string(&template)
            .unwrap()
            .replace(
                "@VERSION_MAJOR@",
                &env::var("CARGO_PKG_VERSION_MAJOR").unwrap(),
            )
            .replace(
                "@VERSION_MINOR@",
                &env::var("CARGO_PKG_VERSION_MINOR").unwrap(),
            )
            .replace(
                "@VERSION_MICRO@",
                &env::var("CARGO_PKG_VERSION_PATCH").unwrap(),
            );

        let header_file = out_dir.join("seccomp.h");

        fs::write(&header_file, rendered).unwrap();

        // We don't emit cargo:rerun-if-changed for the generated header to avoid unconditionally
        // recompiling foundations, but we should emit it for the template its generated from.
        println!("cargo:rerun-if-changed={}", template.display());

        header_file
    }

    fn gen_security_sys_rs(crate_root: &Path, out_dir: &Path, header_file: &Path) {
        // NOTE: we don't care about syscalls and this header needs to be in path for bindgen
        // to work, so let's just remove it.
        let edited_header = fs::read_to_string(header_file)
            .unwrap()
            .replace("#include <seccomp-syscalls.h>", "");

        fs::write(header_file, edited_header).unwrap();

        Builder::default()
            .header(header_file.display().to_string())
            .header(
                crate_root
                    .join("src/security/include/sys-deps.h")
                    .display()
                    .to_string(),
            )
            .allowlist_function("seccomp_rule_add_exact_array")
            .allowlist_function("seccomp_init")
            .allowlist_function("seccomp_load")
            .allowlist_function("SCMP_ACT_ERRNO")
            .allowlist_function("prctl")
            .allowlist_type("scmp_arg_cmp")
            .allowlist_var("SCMP_ACT_LOG")
            .allowlist_var("SCMP_ACT_KILL_PROCESS")
            .allowlist_var("SCMP_ACT_ALLOW")
            .allowlist_var("PR_SET_TSC")
            .allowlist_var("PR_TSC_SIGSEGV")
            .allowlist_var("PR_GET_SECCOMP")
            .allowlist_var("PR_SET_NAME")
            .derive_default(true)
            .parse_callbacks(Box::new(CargoCallbacksIgnoreGenHeaders::new(
                out_dir.to_owned(),
            )))
            .generate()
            .unwrap()
            .write_to_file(out_dir.join("security_sys.rs"))
            .unwrap();
    }

    fn have_linux_seccomp_h(out_dir: &Path) -> bool {
        let src = out_dir.join("check_have_linux_seccomp_h.c");

        fs::write(&src, "#include <linux/seccomp.h>").unwrap();

        cc::Build::new()
            .cargo_metadata(false)
            .warnings(false)
            .file(&src)
            .try_compile("check_have_linux_seccomp_h")
            .is_ok()
    }

    /// Customized version of bindgen::CargoCallbacks that ignores files under out_dir.
    ///
    /// We cannot emit cargo:rerun-if-changed for generated files because it leads to
    /// unconditional recompilation. This assumes all files under out_dir are generated.
    #[derive(Debug)]
    struct CargoCallbacksIgnoreGenHeaders {
        out_dir: PathBuf,
    }

    impl CargoCallbacksIgnoreGenHeaders {
        fn new(out_dir: PathBuf) -> Self {
            Self { out_dir }
        }

        fn is_generated_file(&self, filename: &str) -> bool {
            Path::new(filename).starts_with(&self.out_dir)
        }
    }

    impl bindgen::callbacks::ParseCallbacks for CargoCallbacksIgnoreGenHeaders {
        fn header_file(&self, filename: &str) {
            self.include_file(filename);
        }

        fn include_file(&self, filename: &str) {
            if !self.is_generated_file(filename) {
                println!("cargo:rerun-if-changed={filename}");
            }
        }

        fn read_env_var(&self, key: &str) {
            println!("cargo:rerun-if-env-changed={key}");
        }
    }
}
