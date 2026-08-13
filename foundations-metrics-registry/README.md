# foundations-metrics-registry

`foundations-metrics-registry` provides the process-global metric registry and
the [`prometheus/client_model`] protobuf types used by the `foundations` metrics
stack.

Most applications should use the sibling
[`foundations-metrics`](https://crates.io/crates/foundations-metrics) crate,
which provides metric types, encoders, label serialisation, and collection. Use
this crate directly when you need to inspect registered metrics through
[`iter`].

## Documentation

https://docs.rs/foundations-metrics-registry/

## License

BSD-3 licensed. See the [LICENSE](https://github.com/cloudflare/foundations/blob/main/LICENSE)
file for details.

[`prometheus/client_model`]: https://github.com/prometheus/client_model
[`iter`]: https://docs.rs/foundations-metrics-registry/latest/foundations_metrics_registry/fn.iter.html
