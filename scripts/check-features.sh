#!/usr/bin/env bash
set -euo pipefail

cargo hack check --feature-powerset --no-dev-deps --depth 1
