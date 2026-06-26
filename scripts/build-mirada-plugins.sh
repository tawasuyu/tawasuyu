#!/usr/bin/env bash
# =============================================================================
#  scripts/build-mirada-plugins.sh — forja los plugins WASM de ejemplo de mirada
# -----------------------------------------------------------------------------
#  Espejo de build-pluma.sh, simplificado. Para cada crate de ejemplo (que vive
#  fuera del workspace raíz, con su propio `[workspace]`):
#
#    1. cargo build --release --target wasm32-unknown-unknown
#    2. wasm-opt -Os --strip-debug --strip-producers (si está disponible)
#    3. deposita el .wasm en mirada-plugin-host/assets/, junto a su .ron
#
#  Los .wasm se COMMITEAN: los tests del host los cargan con include_bytes!,
#  hermético y sin asumir el toolchain wasm32 en cada máquina.
#
#  Uso: ./scripts/build-mirada-plugins.sh
# =============================================================================

set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MIRADA="$RAIZ/02_ruway/mirada"
ASSETS="$MIRADA/mirada-plugin-host/assets"

if [ -t 1 ]; then
    VERDE='\033[32m'; AMARILLO='\033[33m'; AZUL='\033[36m'; RESET='\033[0m'
else
    VERDE=''; AMARILLO=''; AZUL=''; RESET=''
fi

# Localizar wasm-opt (opcional): PATH, ~/.cargo/bin, dart-sdk de Flutter.
WASM_OPT=""
for cand in \
        "$(command -v wasm-opt 2>/dev/null || true)" \
        "$HOME/.cargo/bin/wasm-opt" \
        "/opt/flutter/bin/cache/dart-sdk/bin/utils/wasm-opt"; do
    if [ -n "$cand" ] && [ -x "$cand" ]; then WASM_OPT="$cand"; break; fi
done

# (nombre_crate, nombre_artefacto, nombre_salida)
forjar() {
    local crate="$1" artefacto="$2" salida="$3"
    local dir="$MIRADA/$crate"
    echo -e "${AZUL}[mirada-plugins]${RESET} cargo build --release --target wasm32-unknown-unknown ($crate)"
    (cd "$dir" && cargo build --release --target wasm32-unknown-unknown --quiet)
    local wasm="$dir/target/wasm32-unknown-unknown/release/$artefacto.wasm"
    if [ ! -f "$wasm" ]; then
        echo "FALLO: no se produjo $wasm"; exit 1
    fi
    local crudo; crudo=$(stat -c '%s' "$wasm")
    local dst="$ASSETS/$salida.wasm"
    if [ -n "$WASM_OPT" ]; then
        "$WASM_OPT" -Os --strip-debug --strip-producers --enable-bulk-memory "$wasm" -o "$dst"
        local opt; opt=$(stat -c '%s' "$dst")
        echo -e "${VERDE}[mirada-plugins]${RESET} $salida.wasm :: $crudo → $opt bytes (wasm-opt)"
    else
        cp "$wasm" "$dst"
        echo -e "${AMARILLO}[mirada-plugins]${RESET} $salida.wasm :: $crudo bytes (sin wasm-opt)"
    fi
}

mkdir -p "$ASSETS"
forjar "mirada-plugin-example-layout"  "mirada_plugin_example_layout"  "example-layout"
forjar "mirada-plugin-example-reactor" "mirada_plugin_example_reactor" "example-reactor"

echo -e "${VERDE}[mirada-plugins]${RESET} listo → $ASSETS"
