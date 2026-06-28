#!/usr/bin/env bash
# Compila las apps WASM Tier 3 de ejemplo a wasm32-unknown-unknown y deja el
# .wasm como fixture del host (llimphi-wasm-runner/assets/), que lo embebe con
# include_bytes! en su example y sus tests.
#
# Requiere: rustup target add wasm32-unknown-unknown
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET=wasm32-unknown-unknown
OUT="$ROOT/02_ruway/llimphi/llimphi-wasm-runner/assets"
mkdir -p "$OUT"

echo "→ compilando llimphi-wasm-demo-counter ($TARGET, release)"
cargo build -p llimphi-wasm-demo-counter --target "$TARGET" --release

cp "$ROOT/target/$TARGET/release/llimphi_wasm_demo_counter.wasm" "$OUT/counter.wasm"
echo "✓ $OUT/counter.wasm ($(wc -c <"$OUT/counter.wasm") bytes)"
