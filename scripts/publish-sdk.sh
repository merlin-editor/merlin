#!/usr/bin/env bash
# scripts/publish-sdk.sh: Builds, verifies, and publishes all SDK crates in order.
set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Verifying and publishing merlin-plugin..."
(cd sdk/rust && cargo publish --target wasm32-wasip2 "$@")

echo "==> Waiting for crates.io index to settle..."
sleep 25

echo "==> Verifying and publishing merlin-lsp..."
(cd sdk/lsp && cargo publish --target wasm32-wasip2 "$@")

echo "==> Verifying and publishing merlin-grammar..."
(cd sdk/grammar && cargo publish --target wasm32-wasip2 "$@")

echo "==> All SDK crates published successfully!"
