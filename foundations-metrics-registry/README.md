# foundations-metrics-registry

The stable core of the `foundations` metrics stack. It holds the process-global metric
registry and the [`prometheus/client_model`] protobuf types that are the
canonical wire format for the protobuf `/metrics` endpoint.

The crate is deliberately small and dependency-light. The registry is a
process-global singleton, so when two `foundations` majors are linked into the
same binary they have to resolve to the *same* version of this crate to share one
registry instead of splitting metrics between them. Staying minimal and
slow-moving is what makes that shared version easy to hold still; the one
expected source-breaking change is a change to the protobuf data model.

Everything that can evolve more freely — metric types, encoders, label
serialisation, and service-name handling — lives in the sibling
[`foundations-metrics`](../foundations-metrics) crate. Reach for this crate
directly only to consume the registry through `iter`; most code wants `foundations-metrics` instead.

## Documentation

https://docs.rs/foundations-metrics-registry/

## License

BSD-3 licensed. See the [LICENSE](../LICENSE) file for details.

[`prometheus/client_model`]: https://github.com/prometheus/client_model
[`EncodeMetric`]: https://docs.rs/foundations-metrics-registry/latest/foundations_metrics_registry/trait.EncodeMetric.html
