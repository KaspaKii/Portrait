#!/usr/bin/env bash
# _harness/ci.sh — the gate script every change must pass before commit.
#
# Single source of truth for "is this green?" — used by automation AND by any
# human contributor. If this exits non-zero, do not commit.
#
# Usage:
#   ./_harness/ci.sh              # run all gates
#   ./_harness/ci.sh --fast       # skip mdbook + hello-vault (rust-only)
#   ./_harness/ci.sh --release    # also run cargo build --release (slow)

set -eu
cd "$(dirname "$0")/.."

FAST=0
RELEASE=0
for arg in "$@"; do
  case "$arg" in
    --fast)    FAST=1 ;;
    --release) RELEASE=1 ;;
  esac
done

echo "=== kaspa-compliance-patterns CI ==="
echo "branch: $(git branch --show-current) @ $(git rev-parse --short HEAD)"
echo ""

echo "[1/5] cargo fmt --check"
cargo fmt --check

echo ""
echo "[2/5] cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo ""
echo "[3/5] cargo test --workspace"
cargo test --workspace

if [ "$RELEASE" -eq 1 ]; then
  echo ""
  echo "[3b] cargo build --release --workspace"
  cargo build --release --workspace
fi

if [ "$FAST" -eq 0 ]; then
  echo ""
  echo "[4/5] examples/hello-vault — cargo run"
  ( cd examples/hello-vault && cargo run --quiet )

  echo ""
  echo "[5/5] book/ — mdbook build"
  if command -v mdbook >/dev/null 2>&1; then
    ( cd book && mdbook build )
  else
    echo "  (mdbook not on PATH — install with: cargo install mdbook)"
    echo "  skipping book build; not a hard fail in --fast mode"
  fi
else
  echo ""
  echo "[4-5/5] skipped (--fast)"
fi

echo ""
echo "=== CI GREEN ==="
