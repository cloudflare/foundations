#!/usr/bin/env bash
set -euo pipefail

cargo fmt -- --check

cargo clippy --all-targets -- \
    -D warnings \
    -D unreachable_pub \
    -D clippy::await_holding_lock \
    -D clippy::clone_on_ref_ptr 

RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

cargo deny check