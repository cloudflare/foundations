#![cfg(all(target_os = "linux", feature = "memory-profiling"))]

use std::process::Command;

fn run_helper(malloc_conf: &str, enabled: bool) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_memory-profiler-test-helper"));
    command
        .arg(if enabled { "enabled" } else { "disabled" })
        .env("_RJEM_MALLOC_CONF", malloc_conf);

    let output = command
        .output()
        .expect("failed to run memory profiler test helper");

    assert!(
        output.status.success(),
        "memory profiler test helper failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout)
        .expect("helper output should be UTF-8")
        .trim()
        .to_owned()
}

#[test]
fn disabled_settings_are_applied() {
    // Disabled application settings must override an allocator configured as active at startup.
    assert_eq!(
        run_helper("prof:true,prof_active:true,lg_prof_sample:19", false),
        "profiler=false active=false sample_interval=17"
    );
}

#[test]
fn enabled_settings_activate_inactive_profiler() {
    // Enabled application settings must activate the allocator and replace its sample interval.
    assert_eq!(
        run_helper("prof:true,prof_active:false,lg_prof_sample:19", true),
        "profiler=true active=true sample_interval=17"
    );
}
