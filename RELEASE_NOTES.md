
4.3.0
- 2024-12-13 Expose active traces without telemetry server
- 2024-12-09 Expose active traces through telemetry server (#87)

4.2.0
- 2024-11-18 Release 4.2.0 (#97)
- 2024-11-18 Support configuring behaviour when excessive log nesting is reached (#96)

4.1.1
- 2024-11-14 Release 4.1.1
- 2024-11-14 Upgrade minimum required tokio version to deal with deprecated unstable metric
- 2024-11-14 Rename tokio unstable metric that was deprecated (#93)
- 2024-11-13 ZTC-1819: If we're going to panic due to excessive nesting, release the lock first (#91)

4.1.0
- 2024-11-04 Release 4.1.0 (#86)
- 2024-10-31 Allow opting out of settings deny_unknown_fields for foundations internal structs (#85)
- 2024-09-21 Provide metrics::add_extra_producer() to enable external metrics
- 2024-10-21 Remove `routerify` dependency

4.0.0
- 2024-10-14 Release 4.0.0
- 2024-10-12 Adds guide for migrating to 4.0.0
- 2024-09-25 Update semgrep.yml
- 2024-09-22 Create semgrep.yml
- 2024-10-04 Improve seccomp API by removing SeccompMode enum
- 2024-10-03 OXY-1404: Revert most of #60 and expose get_current_thread_seccomp_mode (#74)
- 2024-10-03 Let GitHub run CI actions for version branches (#73)
- 2024-10-03 Fix libssecomp typo (#70)
- 2024-10-03 Add readme for examples (#69)
- 2024-10-01 Fix accidental 4.0 breaking changes and run example in CI (#68)
- 2024-09-12 Release 4.0.0-rc.1
- 2024-09-12 Update versions for 4.0.0
- 2024-09-13 OXY-1404: Avoid crashes resulting from double seccomp initialization (#60)
- 2024-08-20 chore: add passive sampling option to tracing
- 2024-07-18 Remove old deserialize
- 2024-02-03 telemetry: change LogVerbosity to an enum, use it throughout public API
- 2024-08-12 Buffer JSON slog messages
- 2024-07-24 Lock around the sender
- 2024-07-22 Add blank line
- 2024-07-22 Clippy
- 2024-07-22 More code review improvements
- 2024-07-22 Remove profiler sandboxing functionality
- 2024-07-19 Code review improvements, fix builds
- 2024-07-18 Remove now-unnecessary profiling lock
- 2024-07-18 Cleanup and add docs about initialization order
- 2024-07-17 Remove recv timeout
- 2024-07-17 ZTC-1648: Avoid heap profiling crash by starting profiling thread before seccomp is configured
- 2024-08-12 Fix seccomp violation in MemoryProfiler introduce by Rust 1.80.0 std
- 2024-08-06 Make clippy happy
- 2024-08-05 Expose TraceId, as it now can be used in serializable state ctor
- 2024-07-24 Rename depricated tokio metric
- 2024-07-22 Bump syn and darling (closes #50)
- 2024-07-19 Telemetry API improvements
- 2024-07-17 ZTC-1545: Error during settings deserialization if YAML contains unused keys (#49)
- 2024-07-17 Don't halt gRPC reporting on errors (closes #44)
- 2024-07-09 ZTC-1478: Rework logger nesting tracking
- 2024-06-25 Fix CI by temporarily allowing lint violated by darling crate (#51)
- 2024-06-13 ZTC-1478: track the number of times a logger has been replaced with a child logger (#47)
- 2024-05-12 Implement gRPC export for traces
- 2024-05-06 Re-introduce interior mutability for TestTelemetryContext::traces()
- 2024-05-06 Shape new tracing output API.
- 2024-03-29 Implement OTLP output settings. Make serde aware of defaults for settings
- 2024-03-16 Implement conversion of span data to OTLP format
- 2024-03-14 Use term "output" instead of "exporter" for tracing to be consistent with logging settings
- 2024-03-11 Introduce TelemetryDriver
- 2024-03-11 Make tracer exporters configurable
- 2024-03-26 Improve metrics bind error message

3.3.0
- 2024-03-21 Release 3.3.0
- 2024-03-21 Fix new lints
- 2024-03-19 Construct metrics registry with default() when name_in_metrics is empty
- 2024-03-10 Fix paths in gen-syscall-enum tool
- 2024-01-17 OXY-1299: implement GaugeGuard
- 2024-01-17 OXY-1299: Implement RangeGauge metric type

3.2.2
- 2024-02-15 Release 3.2.2
- 2024-02-14 Fix dependencies issue then example is started from the /example dir (closes #23)
- 2024-02-15 Fix potential deadlock in `link_new_trace_with_current`
- 2024-02-12 ci: add docsrs and minver checks (#20)
- 2024-02-09 docs: fix typo in service_info macro doc comments (#19)

3.2.1
- 2024-02-05 Release 3.2.1

3.2.0
- 2024-02-05 Release 3.2.0
- 2024-02-05 telemetry: add tokio runtime metrics (#12)
- 2024-02-02 telemetry: make logger verbosity public
- 2024-02-01 ci: cleanup actions config
- 2024-01-31 Enable feature `all` on socket2, should address #5
- 2024-02-01 Disable `default-features` for dependency `prometheus`
- 2024-01-31 ci: add macos to features check ci

3.1.1
- 2024-01-26 Release 3.1.1
- 2024-01-26 Add check for missing seccomp sources to ensure that they are always published

3.1.0
- 2024-01-26 Release 3.1.0
- 2024-01-26 Merge pull request #2 from cloudflare/android-ci
- 2024-01-26 Introduce feature sets for clients
- 2024-01-25 Merge pull request #1 from zegevlier/fix-windows
- 2024-01-25 Only use `socket.set_reuse_port` on supported operating systems
- 2024-01-23 Make crates.io happy about keywords

3.0.1
- 2024-01-23 Release 3.0.1
- 2024-01-23 Add more metadata to Cargo.toml
- 2024-01-23 Add readme to Cargo.toml
- 2024-01-23 Fix license type in Cargo.toml
- 2024-01-23 Add license field to Cargo.toml metadata

3.0.0
- 2024-01-23 Release 3.0.0
- 2024-01-23 Fix macos target in CI
- 2024-01-23 Add jemalloc flag to CI
- 2024-01-23 Fix CI target
- 2024-01-23 Update year in license
- 2024-01-22 Revive feature combination check in CI
- 2024-01-21 Remove Settings bound from settings::Map key
- 2024-01-21 Remove Send + Sync bound for settings::Map keys
- 2024-01-21 Fix doc comment typo
- 2024-01-21 Capitilise the lib name in doc comments
- 2024-01-20 Some minor renames in docs
- 2024-01-20 Add examples to README
- 2024-01-20 Add support for settings collections, add more basic impls
- 2024-01-19 Add docs paragraph for setting standard types substitutes
- 2024-01-19 Update README
- 2024-01-19 Rename the library.
- 2024-01-19 Update readme
- 2024-01-19 Fix banner
- 2024-01-19 Update README
- 2024-01-19 Add license
- 2024-01-19 Add Github CI
- 2024-01-19 Clean up
- 2024-01-19 Enable documentation of Settings within Vec and Option
- 2024-01-18 Release 2.2.0
- 2024-01-18 Release 2.2.0
- 2023-12-28 EGRESS-939: Adds log volume metric counter feature
- 2023-12-28 EGRESS-939: Update oer members in CODEOWNERS file
- 2024-01-02 OXY-1298: Disable MacOS build due to errors
- 2023-12-21 Add PID to root logger
- 2023-12-12 Drop zero histogram bucket
- 2023-11-30 TUN-8005: Document how to use jemalloc from bedrock
- 2023-10-13 Release 2.1.0
- 2023-10-13 Introduce Cli parsing from provided args.
- 2023-10-09 Release 2.0.7
- 2023-10-02 ZTC-1201: Fixes issue where log::set_verbosity broke the connection between the test logger and the log records
- 2023-09-29 ZTC-886: Rate limit trace creation if configured and removes prior additions to with_test_telemetry macro
- 2023-09-29 Release 2.0.6
- 2023-09-29 OXY-1224: raise the log verbosity for the test logger
- 2023-09-25 [OXY-1241] chore: stop specifying features in the workspace toml
- 2023-09-25 Small doc improvement for Telemetry server
- 2023-09-25 Release 2.0.5
- 2023-09-25 ZTC-1189: Adjust `with_graceful_shutdown` to be sync and improve its docs
- 2023-09-22 Set up default flavor for cfsetup
- 2023-09-22 Release 2.0.4
- 2023-09-22 ZTC-1189: Allow telemetry server to be gracefully shut down
- 2023-09-21 ZTC-1187: Reuses a single root AsyncDrain object to avoid garbled log output
- 2023-09-21 ZTC-885: Adds new options to the with_test_telemetry macro, allowing us to specify rate limit and redact_keys
- 2023-09-20 ZTC-885: Rate limits logging events
- 2023-09-21 chore: minify dep tree a little
- 2023-09-19 Use cargo-nextest
- 2023-09-19 Remove some useless cfsetup dependencies
- 2023-09-19 Use debian-bullseye-rustlang Docker image
- 2023-09-13 Release 2.0.3
- 2023-09-13 Fix cross builds with feature security
- 2023-09-12 Release 2.0.2
- 2023-09-12 Release 2.0.1
- 2023-09-11 ROCK-18: Don't panic if metrics system is not initialized
- 2023-09-07 Release 2.0.0
- 2023-09-07 Update example Cargo.toml
- 2023-09-07 Make Map generic in its keys
- 2023-09-07 Add example
- 2023-09-06 ROCK-18: Don't panic on initializing metrics registries twice
- 2023-09-05 Implement Cli
- 2023-09-05 Introduce settings Map structure whose items are documentable via Settings trait
- 2023-09-04 Add ability to add custom routes to telemetry server.
- 2023-09-04 ROCK-18: Update docs and tweak naming for metrics service name
- 2023-09-01 ROCK-18: Support a custom metrics service identifier value and format
- 2023-09-04 Disable bindgen default feature `which-rustfmt`
- 2023-09-01 ROCK-20 Implement memory profiler telemetry server endpoint
- 2023-09-01 Some tweaks to metrics API and docs
- 2023-08-29 Release 1.2.0
- 2023-06-16 ROCK-4: Implement bedrock::telemetry::metrics
- 2023-08-29 Remove unnecessary cast
- 2023-06-19 Fix unused_variables lint
- 2023-08-23 ZTC-885: Updates heap profiling code slightly to be usable by oxy
- 2023-08-02 Document SpanScope and move it out of internal module
- 2023-08-01 Do not drop heap profile temp file before reading completion
- 2023-08-01 ROCK-5 Implement jemalloc-related functionality
- 2023-07-26 Release 1.1.0
- 2023-07-24 ROCK-16: Add 'jaeger_reporter_bind_addr' to TracingSettings
- 2023-07-04 Adds cargo-release and git-cliff config
- 2023-07-03 Version 1.0.3
- 2023-06-28 Use workspace metadata for crates
- 2023-06-27 Version 1.0.2
- 2023-06-27 Removes cyclical dependency
- 2023-06-27 Release 1.0.1
- 2023-06-27 Updates cfsetup.yaml to support registry
- 2023-06-27 Specifies registry in workspace toml
- 2023-06-23 ETI-942: Fixes indentation of cfsetup publish builddeps
- 2023-06-23 Release 1.0.0
- 2023-06-23 ETI-942: Publish crate to internal registry
- 2023-06-20 Rename `seccomp` module to `security`.
- 2023-06-15 Add a few more common allow lists used by Cloudflare apps
- 2023-06-14 ROCK-3 Implement seccomp filter initialization
- 2023-06-09 ROCK-3  Seccomp: implement allow lists
- 2023-06-13 chore: remove shih-chiang from CODEOWNERS
- 2023-06-07 Fix syscall doc links
- 2023-06-06 Remove lohith from code-owners
- 2023-06-06 ROCK-3 Generate syscalls enums
- 2023-06-05 Implement with_test_telemetry macro
- 2023-06-03 Test API changes.
- 2023-06-02 ROCK-13 Finish telemetry documentation
- 2023-05-31 Set ipv6 reporter addr if agent is set to ipv6
- 2023-05-25 Add links whenever we create a new trace and we have one ongoing
- 2023-05-29 Add doctests for with_forked_trace and start_trace
- 2023-05-30 Settings: add PartialEq impl between std::net types and wrappers
- 2023-05-25 ZTC-881 Allow overriding sampling ratio for started trace
- 2023-05-24 Expose SerializableTraceState
- 2023-05-20 ROCK-9, ROCK-13 Part 1: Add the rest of the tracing API and document telemetry
- 2023-05-12 Move settings macro into the settings module
- 2023-05-12 ROCK-9, ROCK-10 Implement tracing internals and testing
- 2023-05-11 GATE-4093: change bedrock package version to use the standard indexed field
- 2023-05-04 ROCK-2 Implement logging
- 2023-05-02 Get rid of owned keys feature in slog to not introduce breaking changes
- 2023-05-02 ROCK-11 Add toggle to disable Debug impl in Settings macro
- 2023-04-27 Add feature combination testing
- 2023-04-26 ROCK-8 Implement test log and log drains
- 2023-04-18 Remove println leftover in test
- 2023-04-17 Implement settings
- 2023-04-12 Set up repo
- 2023-04-12 Initial commit


