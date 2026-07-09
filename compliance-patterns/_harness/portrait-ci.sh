#!/usr/bin/env bash
# _harness/portrait-ci.sh — gate script for the Portrait compiler workspace.
#
# Companion to _harness/ci.sh (which gates the kaspa-compliance-patterns Rust
# workspace). Portrait lives in a SEPARATE repo at portrait/portrait;
# this script runs the same fmt + clippy + test gates there so the compiler stack
# stays green alongside the library. Includes the golden + differential + reject
# regression harness in crates/portrait-cli/tests/golden.rs (Phase E + §4.2):
# the differential layer invokes the real `silverc` and asserts exit 0.
#
# Usage:
#   ./_harness/portrait-ci.sh
#
# Exits non-zero (and prints FAIL) on the first gate that regresses; do not
# commit Portrait changes if this exits non-zero.

set -eu

PORTRAIT_DIR="${PORTRAIT_DIR:-$HOME/kii-portrait/portrait/portrait}"

echo "=== Portrait compiler workspace CI ==="
echo "workspace: $PORTRAIT_DIR"

if [ ! -f "$PORTRAIT_DIR/Cargo.toml" ]; then
  echo ""
  echo "=== Portrait CI FAIL: no Cargo.toml at $PORTRAIT_DIR ==="
  echo "    (set PORTRAIT_DIR to the portrait workspace root)"
  exit 1
fi

cd "$PORTRAIT_DIR"
echo ""

echo "[1/3] cargo fmt --check"
if ! cargo fmt --check; then
  echo ""
  echo "=== Portrait CI FAIL (fmt) ==="
  exit 1
fi

echo ""
echo "[2/3] cargo clippy --workspace --all-targets -- -D warnings"
if ! cargo clippy --workspace --all-targets -- -D warnings; then
  echo ""
  echo "=== Portrait CI FAIL (clippy) ==="
  exit 1
fi

echo ""
echo "[3/3] cargo test --workspace"
if ! cargo test --workspace; then
  echo ""
  echo "=== Portrait CI FAIL (test) ==="
  exit 1
fi

echo ""
echo "=== Portrait CI PASS ==="
