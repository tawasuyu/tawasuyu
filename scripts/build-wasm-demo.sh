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

build() {
  local crate="$1" out="$2"
  echo "→ compilando $crate ($TARGET, release)"
  cargo build -p "$crate" --target "$TARGET" --release
  cp "$ROOT/target/$TARGET/release/${crate//-/_}.wasm" "$OUT/$out"
  echo "✓ $OUT/$out ($(wc -c <"$OUT/$out") bytes)"
}

build llimphi-wasm-demo-counter counter.wasm
build llimphi-wasm-demo-form form.wasm
