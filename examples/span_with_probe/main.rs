//! Demo workload for the `span_with_probe!` macro and `span_fn`'s
//! `end_probe = true` option.
//!
//! Instrumented spans with distinct delay ranges run in a loop. Attach
//! with bpftrace to get duration histograms for all spans (from the repo
//! root, so the probe path resolves):
//!
//! ```text
//! bpftrace examples/span_with_probe/span_durations.bt -p <pid>
//! ```
//!
//! No telemetry context is installed: every span is unsampled and tracing is
//! effectively disabled. This is intentional — the USDT probe must fire
//! regardless of span sampling.

use foundations::telemetry::tracing::{span_fn, span_with_probe};
use std::io::Write as _;
use std::time::Duration;

/// Span with a 5-25ms delay range.
async fn short_task(iter: u64) {
    let work_fut = async move {
        // Simulate variable work so durations show up as a histogram.
        tokio::time::sleep(Duration::from_millis(5 + iter % 20)).await;
    };

    span_with_probe!("example::short_task")
        .into_context()
        .apply(work_fut)
        .await;
}

/// Span with a distinct delay range (50-70ms) so the two can be told apart
/// in a duration histogram.
async fn long_task(iter: u64) {
    let work_fut = async move {
        tokio::time::sleep(Duration::from_millis(50 + iter % 20)).await;
    };

    span_with_probe!("example::long_task")
        .into_context()
        .apply(work_fut)
        .await;
}

/// Same probing via `span_fn`'s `end_probe = true` option, with its own
/// delay range (100-120ms).
#[span_fn("example::attr_task", end_probe = true)]
async fn attr_task(iter: u64) {
    tokio::time::sleep(Duration::from_millis(100 + iter % 20)).await;
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    println!("pid {}", std::process::id());

    let mut iter = 0;
    loop {
        short_task(iter).await;
        long_task(iter).await;
        attr_task(iter).await;

        print!("\riteration {iter}");
        std::io::stdout().flush().unwrap();

        iter += 1;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
