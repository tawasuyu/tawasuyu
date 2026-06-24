#!/usr/bin/env bash
# build-tawasuyu-bundle.sh — forja el bundle precompilado de la suite que
# consume el instalador gráfico `churay` (lado A: instalar sin compilar).
#
#   scripts/build-tawasuyu-bundle.sh [out_dir]
#
# Pasos:
#   1. cargo build --release de cada binario del catálogo + el instalador.
#   2. churay-bundle: copia los binarios, calcula hash BLAKE3 + tamaño y emite
#      el manifiesto (manifest.json; manifest.signed.json si CHURAY_SIGN_SEED).
#   3. empaqueta a <out>.tar.zst con SHA256SUMS.
#
# Para firmar el manifiesto, exportá una semilla ed25519 (hex de 64 chars):
#   export CHURAY_SIGN_SEED=$(head -c32 /dev/urandom | xxd -p -c64)
#
# Nota: las apps GPU (Llimphi/wgpu) se linkean dinámicamente; el bundle es
# portable entre Linux de glibc comparable. Un bundle 100% estático (musl) o
# AppImage para "cualquier Linux" sin compromiso es un paso posterior.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

OUT="${1:-dist/tawasuyu-bundle}"
mkdir -p "$OUT"

# Binarios del catálogo (fuente de verdad: churay_core::suite_catalog).
# Si alguno no compila/está, churay-bundle lo omite con aviso — no aborta.
PROGRAMS=(
  churay
  nada pluma-editor-llimphi pluma-notebook-llimphi tullpu-app-llimphi
  takiy-app-llimphi media-app media-tube cosmos-app-llimphi
  dominium-app-llimphi tinkuy-llimphi chaka-app-llimphi nakui-sheet-llimphi
  puriy raymi-app supay-app-llimphi sandokan-monitor nahual-shell-llimphi
  mirada-llimphi wawa-panel
  # componente de sistema (root):
  arje
)

echo "==> compilando ${#PROGRAMS[@]} binarios (release)…"
for p in "${PROGRAMS[@]}"; do
  echo "  · $p"
  cargo build --release --bin "$p" 2>/dev/null || echo "    (saltado: $p no compila aislado)"
done

echo "==> ensamblando bundle con churay-bundle…"
cargo run --release -q -p churay-core --bin churay-bundle -- "$OUT"

echo "==> empaquetando…"
STAMP="$(date +%Y-%m-%d)"
TAR="$OUT-$STAMP.tar.zst"
( cd "$(dirname "$OUT")" && tar --zstd -cf "$(basename "$TAR")" "$(basename "$OUT")" )
( cd "$(dirname "$OUT")" && sha256sum "$(basename "$TAR")" > "$(basename "$TAR").sha256" )

echo
echo "listo:"
echo "  bundle:  $OUT/"
echo "  tar:     $TAR"
echo
echo "probar el instalador contra este bundle, sin compilar:"
echo "  CHURAY_BUNDLE=$ROOT/$OUT cargo run --release -p churay-llimphi"
