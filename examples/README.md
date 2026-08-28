
# Examples


## Init


Pull submodules to grab `libseccomp` source.


```
git submodule update --init --recursive
```


## Run


```
cargo run --example http_server -- --config http_server/example_conf.yaml
```


## `span_with_probe`


Demo workload for per-span USDT probes. Runs spans instrumented with the
`span_with_probe!` macro and `span_fn`'s `end_probe = true` option in a loop;
attach with bpftrace to get duration histograms:

```
cargo run --example span_with_probe
sudo bpftrace examples/span_with_probe/span_durations.bt -p <pid>
```

Sample output:

```
Attaching to span end probes, durations in milliseconds...
Hit Ctrl-C to end and print histograms.
^C

@attr_task_ms:
[64, 128)             23 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@|

@long_task_ms:
[32, 64)              17 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@|
[64, 128)              7 |@@@@@@@@@@@@@@@@@@@@@                               |

@short_task_ms:
[4, 8)                 5 |@@@@@@@@@@@@@@@@@@@@@@@@@@                          |
[8, 16)                9 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@      |
[16, 32)              10 |@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@|
```




