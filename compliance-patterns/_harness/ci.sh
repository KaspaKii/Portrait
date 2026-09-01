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

# The generated-project compile checks in crates/kcp-cli/tests/ shell out to a
# nested `cargo`, which needs a populated cargo cache (network on a cold one).
# They skip unless opted in, so an offline clone degrades to a skip rather than
# a hard failure; the gate opts in.
export KCP_GATE_SCAFFOLD_BUILD=1

echo "=== kaspa-compliance-patterns CI ==="
echo "branch: $(git branch --show-current) @ $(git rev-parse --short HEAD)"
echo ""

echo "[1/7] cargo fmt --check"
cargo fmt --check

echo ""
echo "[2/7] cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo ""
echo "[3/7] cargo clippy --workspace --all-targets --all-features -- -D warnings"
# --all-features is the ONLY way the real-engine tests (behind `wrpc`) reach the
# gate; without it they compile nowhere and the engine claims go unchecked.
cargo clippy --workspace --all-targets --all-features -- -D warnings

echo ""
echo "[4/7] cargo test --workspace"
cargo test --workspace

echo ""
echo "[5/7] cargo test --workspace --all-features"
cargo test --workspace --all-features

if [ "$RELEASE" -eq 1 ]; then
  echo ""
  echo "[5b] cargo build --release --workspace"
  cargo build --release --workspace
fi

if [ "$FAST" -eq 0 ]; then
  echo ""
  echo "[6/7] examples/ — cargo test every standalone project"
  # The examples/ projects are standalone workspaces: they are NOT in
  # [workspace] members, so `cargo test --workspace` never compiles them and a
  # breaking API change in a crate can ship a broken example. This gate builds
  # and tests ALL of them (iterating every directory with a Cargo.toml), not
  # just hello-vault. ~4m30s warm; use --fast to skip.
  for example_manifest in examples/*/Cargo.toml; do
    example_dir="$(dirname "$example_manifest")"
    echo "  → $example_dir"
    ( cd "$example_dir" && cargo test --quiet )
  done
  # hello-vault is also the documented 10-minute on-ramp: run it, so the gate
  # proves the output the README promises.
  echo "  → examples/hello-vault (cargo run)"
  ( cd examples/hello-vault && cargo run --quiet )

  echo ""
  echo "[7/7] book/ — mdbook build"
  if command -v mdbook >/dev/null 2>&1; then
    ( cd book && mdbook build )
  else
    echo "  (mdbook not on PATH — install with: cargo install mdbook)"
    echo "  skipping book build; not a hard fail in --fast mode"
  fi
else
  echo ""
  echo "[6-7/7] skipped (--fast)"
fi

echo ""
echo "=== CI GREEN ==="
