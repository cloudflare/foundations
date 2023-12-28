#!/usr/bin/env bash
set -euo pipefail

RUSTFLAGS="-D warnings" cargo hack check --feature-powerset --no-dev-deps --depth 3
