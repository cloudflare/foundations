# foundations-metrics

`foundations-metrics` provides counters, gauges, histograms, metric families,
and encoders for OpenMetrics text and Prometheus protobuf formats. It uses
`foundations-metrics-registry` for the process-global registry and protobuf data
model.

## Migrating from `foundations::telemetry::metrics`

`foundations` re-exports this crate's items through
`foundations::telemetry::metrics` when its opt-in `foundations-metrics-backend`
feature is enabled. Existing counter, gauge, histogram, and family call sites
therefore continue to compile. This compatibility is intended to ease migration,
but exposes users to more breaking changes via foundations' larger API surface.
Code using exemplars must move to the generic `WithExemplar` wrapper, and the
deprecated items below require code changes.

`Counter` no longer exposes an underlying
`prometheus_client::metrics::counter::Counter`. Code that previously accessed it
through `counter.0` should call methods on `Counter` directly, such as `inc`,
`inc_by`, or `get`.

Users that do not want any of the other features of foundations are encouraged to
depend directly on `foundations-metrics` in the future. To migrate, update your
`Cargo.toml` and import paths:

```toml
[dependencies]
foundations-metrics = "0.1.0-beta.1"
```

```rust
// Before
use foundations::telemetry::metrics::{Counter, Family, Gauge, InfoMetric, report_info};

// After
use foundations_metrics::{Counter, Family, Gauge, InfoMetric, report_info};
```

Continue to use `foundations` for:

- `#[metrics]` and `#[info_metric]`, which expand to paths that `foundations`
  resolves. Keep invoking them through `foundations::telemetry::metrics`.
- Telemetry setup, including the service name that prefixes or labels collected
  metrics. This crate takes that name through
  [`CollectionOptions`](https://docs.rs/foundations-metrics/latest/foundations_metrics/struct.CollectionOptions.html)
  at collection time instead of discovering it itself.

Types imported through the two paths are compatible only when
`foundations-metrics-backend` is enabled. Without it,
`foundations::telemetry::metrics` exposes legacy metric types that are distinct
from this crate's types.

### Deprecated items

Most APIs require only an import-path change. The following APIs require code
changes:

| Deprecated | Replacement |
| --- | --- |
| `foundations::telemetry::metrics::add_extra_producer` | [`register`](https://docs.rs/foundations-metrics/latest/foundations_metrics/fn.register.html) |
| `foundations::telemetry::metrics::ExtraProducer` | [`EncodeMetric`](https://docs.rs/foundations-metrics/latest/foundations_metrics/trait.EncodeMetric.html) or [`EncodeMetricValue`](https://docs.rs/foundations-metrics/latest/foundations_metrics/trait.EncodeMetricValue.html) |

An extra producer appends pre-encoded Prometheus *text* to the scrape buffer,
bypassing validation. Because that output has no protobuf representation,
registering an extra producer disables protobuf output and causes scrapes to
fall back to text. An `EncodeMetric` implementation returns structured metric
families that both encoders can use. Prefer `EncodeMetricValue` paired with
`NamedMetric` and `Family` unless a metric needs to control its own naming or
emit several families.

When `foundations-metrics-backend` is enabled, `foundations` no longer collects
metrics from the `prometheus` crate's global registry, which the legacy backend
included in its output. Metrics registered directly with that registry are not
collected by this backend.
The [`register`](https://docs.rs/foundations-metrics/latest/foundations_metrics/fn.register.html)
function accepts boxed `EncodeMetric` implementations, not collectors from the
`prometheus` crate. Reimplement those metrics using `EncodeMetric` or
`EncodeMetricValue` before registering them.

On Linux, `foundations` preserves the process collector metrics that the legacy
backend exposed:
`process_cpu_seconds_total`, `process_resident_memory_bytes`,
`process_virtual_memory_bytes`, `process_open_fds`, `process_max_fds`,
`process_start_time_seconds`, and `process_threads`. These are registered in the
new structured registry without a service prefix or label, so existing queries,
alerts, and dashboards continue to work. This compatibility applies only to the
built-in process collector; other metrics registered solely through the
`prometheus` crate still require migration.

Each deprecated item documents its replacement in the
[`foundations` API docs](https://docs.rs/foundations/), and the traits above are
documented with examples in the [API docs for this
crate](https://docs.rs/foundations-metrics/).

## Documentation

https://docs.rs/foundations-metrics/

## License

BSD-3 licensed. See the [LICENSE](https://github.com/cloudflare/foundations/blob/main/LICENSE)
file for details.
