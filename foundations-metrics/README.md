# foundations-metrics

The evolving layer of the `foundations` metrics stack: the concrete metric types
and the logic that encodes them into the Prometheus protobuf data model. It sits
on top of `foundations-metrics-registry`, which owns the process-global registry
and the stable wire format.

## Migrating from `foundations::telemetry::metrics`

`foundations` re-exports this crate's items through
`foundations::telemetry::metrics` when its default `foundations-metrics-backend`
feature is enabled. The substitution is shape-compatible, so existing code keeps
compiling without changes — which makes the feature a transition aid rather than
the destination. The facade is slated for removal in the next major release.

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
bypasses validation and has no protobuf representation — so their output is
silently absent from a protobuf scrape. Implementing `EncodeMetric` returns
structured metric families instead, which both encoders can serve. Prefer
`EncodeMetricValue` paired with `NamedMetric` and `Family` unless a metric needs
to control its own naming or emit several families.

Enabling the feature also stops draining the `prometheus` crate's global
registry, which the legacy collector exported alongside its own. Anything
registered there, including that crate's process collector, needs to be
re-registered through `register`.

For further guidance, each deprecated item names its own replacement in the
[`foundations` API docs](https://docs.rs/foundations/), and the traits above are
documented with examples in the [API docs for this
crate](https://docs.rs/foundations-metrics/).

## Documentation

https://docs.rs/foundations-metrics/

## License

BSD-3 licensed. See the [LICENSE](../LICENSE) file for details.
