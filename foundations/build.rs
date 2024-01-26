use std::env;
use std::path::PathBuf;

fn main() {
    ensure_seccomp_sources_fetched();

    #[cfg(feature = "security")]
    security::build()
}

fn ensure_seccomp_sources_fetched() {
    let src_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("src/security/libseccomp/src");

    if !src_dir.exists() {
        panic!(
            "Can't find libssecomp sources. Run `git submodule update --init --recursive`. \
            This is required even if `security` feature is disabled or the OS is not Linux, \
            to ensure that sources are always included on publishing."
        );
    }
}

#[cfg(feature = "security")]
mod security {
    use super::*;
    use bindgen::{Builder, CargoCallbacks};
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

        let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
        let crate_root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let libseccomp_repo = crate_root.join("src/security/libseccomp");
        let include_dir = libseccomp_repo.join("include");
        let src_dir = libseccomp_repo.join("src");

        if !src_dir.exists() {
            panic!("Can't find libssecomp sources. Run `git submodule update --init --recursive`");
        }

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
        let rendered = fs::read_to_string(include_dir.join("seccomp.h.in"))
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
            .derive_default(true)
            .parse_callbacks(Box::new(CargoCallbacks))
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
}
