#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all

cargo clippy --fix --allow-dirty -- \
    -D warnings \
    -D unreachable_pub \
    -D clippy::await_holding_lock \
    -D clippy::clone_on_ref_ptr

RUSTFLAGS="-D warnings" cargo hack check --feature-powerset --no-dev-deps --depth 3
