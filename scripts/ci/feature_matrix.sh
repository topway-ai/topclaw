#!/usr/bin/env bash
set -euo pipefail

run_step() {
  local label="$1"
  shift

  echo "==> ${label}"
  "$@"
}

run_step "cargo check --no-default-features" cargo check --no-default-features
run_step "cargo check" cargo check
run_step "cargo check --features standard" cargo check --features standard
run_step "cargo check --features desktop" cargo check --features desktop
run_step "cargo check --features full" cargo check --features full
run_step "cargo test" cargo test
run_step "cargo test --features standard" cargo test --features standard
run_step "cargo test --features full" cargo test --features full
