### *slog\_logger*

Users of foundations::telemetry::log::slog\_logger() will notice that the signature has changed.

* It used to return `Arc<RwLock<slog::Logger>>`
* It now returns `Arc<RwLock<impl Deref<Target = slog::Logger>>>  `
* This change should be invisible for most usages of `slog_logger()`. If you were acquiring the RwLock and dereferencing it to get a shared reference to an `slog::Logger`, before, you can still do that after, with no changes being required in the calling source code.

### *Errors for unused keys during setting deserialization (PR [\#49](https://github.com/cloudflare/foundations/pull/49))*

By default, Foundations will now return an error during startup if an app’s settings .yaml file contains keys that don’t have a match in the corresponding settings struct in the app’s code. This likely means that, if nothing is done, most existing apps would fail to start after updating to foundations 4.0 due to unused keys. Note also that keys used only for YAML anchors, if the field doesn’t have a matching field in the settings struct, also count as ‘unused’ and will generate these errors.

This leaves users of foundations two choices:

1. Update configs to remove unused keys. This will involve:  
   1. Remove keys fields that are fully unused  
   2. Move keys that were used purely for YAML anchors inline to their first use. For example, for the case in Apollo linked above, it would instead be something like  
2. Opt out of this feature by updating the settings struct annotation from `#[settings]` to `#[settings(deny_unknown_fields = false)]`

### ZTC-1648: Avoid heap profiling crash by eagerly starting long-lived profiling thread (PR [\#54](https://github.com/cloudflare/foundations/pull/54/files))

Removed `sandbox_profiling_syscalls` from `MemoryProfilerSettings`.

### *Tracing configuration changes*

The new version of the `TracingSettings` introduces more modular and flexible settings for distributed tracing. Here are the main changes and how to migrate:

#### **1\. Struct Redesign and Field Changes**

* **Old:**  
  * `TracingSettings` had individual fields like `jaeger_tracing_server_addr`, `jaeger_reporter_bind_addr`, `sampling_ratio`, and `rate_limit`.  
* **New:**  
  * The tracing settings are now broken down into structured components for better clarity and extensibility:  
    * `TracingSettings` has the following key fields:  
      * `enabled: bool`  
      * `output: TracesOutput`  
      * `sampling_strategy: SamplingStrategy`

**Migration**:

* Replace direct field accesses (`jaeger_tracing_server_addr`, etc.) with the appropriate nested structs and enums.

For example:  
`# Old`  
`tracing:`  
`enabled: true`  
	`jaeger_tracing_server_addr: 127.0.0.1:8080`

`# New`  
`tracing:`  
	`enabled: true`  
	`output:`  
		`jaeger_thrift_udp:`  
			`server_addr: 127.0.0.1:8080`

#### **2\. New Enum `TracesOutput`**

* **Old:**  
  * Traces were sent using the Jaeger UDP format with no enum abstraction.  
* **New:**  
  * The `TracesOutput` enum defines trace output types:  
    * `JaegerThriftUdp`  
    * Optionally `OpenTelemetryGrpc` (if the `telemetry-otlp-grpc` feature is enabled).

**Migration**:

* For Jaeger output, replace direct handling with the `TracesOutput::JaegerThriftUdp` variant.  
* If using OpenTelemetry, you'll need to handle the `OpenTelemetryGrpc` variant (if enabled).

#### **3\. Sampling Strategy as Enum**

* **Old:**  
  * `sampling_ratio` was a direct field in `TracingSettings`.  
* **New:**  
  * The `SamplingStrategy` enum now encapsulates sampling strategies:  
    * `Passive`: For passive sampling.  
    * `Active`: With `ActiveSamplingSettings` (contains `sampling_ratio` and `rate_limit`).

**Migration**:

* Replace `sampling_ratio` with `ActiveSamplingSettings` inside `SamplingStrategy::Active`.

`# Old`  
`tracing_settings:`  
`sampling_ratio: 1.0`

`# New`  
`tracing_settings:`  
	`sampling_strategy:`  
		`active:`  
			`sampling_ratio: 1.0`

#### **5\. Feature Flag Adjustments**

* The new code uses the feature flags `telemetry-otlp-grpc` and `settings` for conditional compilation.

**Migration**:

* Ensure you use the correct feature flags for your build configuration, especially if using OpenTelemetry.


