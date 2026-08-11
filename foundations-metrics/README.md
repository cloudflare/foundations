# foundations-metrics

The evolving layer of the `foundations` metrics stack: the concrete metric types
and the logic that encodes them into the Prometheus protobuf data model. It sits
on top of `foundations-metrics-registry`, which owns the process-global registry
and the stable wire format.

## Migrating from `foundations::telemetry::metrics`

`foundations` re-exports this crate's items through
`foundations::telemetry::metrics` when its default `foundations-metrics-backend`
feature is enabled. Typical counter, gauge, histogram, and family call sites keep
compiling, which makes the feature a transition aid rather than the destination.
Exemplar users must move to the generic `WithExemplar` wrapper, and the deprecated
items below require explicit migration. The facade is slated for removal in the
next major release.

To finish the move, depend on this crate directly and import from it:

```toml
[dependencies]
foundations-metrics = "0.1"
```

```rust
// Before
use foundations::telemetry::metrics::{Counter, Family, Gauge, InfoMetric, report_info};

// After
use foundations_metrics::{Counter, Family, Gauge, InfoMetric, report_info};
```

Two things stay with `foundations` and are not part of this crate:

- `#[metrics]` and `#[info_metric]`, which expand to paths that `foundations`
  resolves. Keep invoking them through `foundations::telemetry::metrics`.
- Telemetry setup, including the service name that prefixes or labels collected
  metrics. This crate takes that name through
  [`CollectionOptions`](https://docs.rs/foundations-metrics/latest/foundations_metrics/struct.CollectionOptions.html)
  at collection time instead of discovering it itself.

Mixing the two paths works only with `foundations-metrics-backend` enabled;
without it, `foundations::telemetry::metrics` names its own legacy types and
those are distinct from this crate's.

### Deprecated items

Most imports move by renaming the path. These have no drop-in equivalent:

| Deprecated | Replacement |
| --- | --- |
| `foundations::telemetry::metrics::add_extra_producer` | [`register`](https://docs.rs/foundations-metrics/latest/foundations_metrics/fn.register.html) |
| `foundations::telemetry::metrics::ExtraProducer` | [`EncodeMetric`](https://docs.rs/foundations-metrics/latest/foundations_metrics/trait.EncodeMetric.html) or [`EncodeMetricValue`](https://docs.rs/foundations-metrics/latest/foundations_metrics/trait.EncodeMetricValue.html) |

Extra producers append pre-encoded Prometheus *text* to the scrape buffer, which
bypasses validation and has no protobuf representation. Registering one therefore
makes protobuf unavailable and causes scrapes to fall back to text. Implementing
`EncodeMetric` returns structured metric families instead, which both encoders can
serve. Prefer `EncodeMetricValue` paired with `NamedMetric` and `Family` unless a
metric needs to control its own naming or emit several families.

Enabling the feature also stops draining the `prometheus` crate's global
registry, which the legacy collector exported alongside its own. Metrics kept
only there are no longer scraped. Note that `register` does not accept that
crate's collectors — [`IntoMetrics`](https://docs.rs/foundations-metrics-registry/latest/foundations_metrics_registry/trait.IntoMetrics.html)
is sealed over `EncodeMetric` — so they have to be reimplemented against the
traits above rather than handed over as they are.

On Linux, `foundations` preserves the process collector metrics that the legacy
backend exposed:
`process_cpu_seconds_total`, `process_resident_memory_bytes`,
`process_virtual_memory_bytes`, `process_open_fds`, `process_max_fds`,
`process_start_time_seconds`, and `process_threads`. These are registered in the
new structured registry without a service prefix or label, so existing queries,
alerts, and dashboards continue to work. This compatibility applies only to the
built-in process collector; other metrics registered solely through the
`prometheus` crate still require migration.

For further guidance, each deprecated item names its own replacement in the
[`foundations` API docs](https://docs.rs/foundations/), and the traits above are
documented with examples in the [API docs for this
crate](https://docs.rs/foundations-metrics/).

## Documentation

https://docs.rs/foundations-metrics/

## License

BSD-3 licensed. See the [LICENSE](https://github.com/cloudflare/foundations/blob/main/LICENSE)
file for details.
