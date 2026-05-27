#!/usr/bin/env bash
# =============================================================================
#  scripts/build-pluma.sh — FASE 50 :: pipeline de compactacion de userspace
# -----------------------------------------------------------------------------
#  La Fase 50 sella el "Gran Cierre del Circulo del Monorepo": el binario WASM
#  de Pluma queda procesado, sin metadatos sueltos, con su arbol de llamadas
#  estaticas purgado. Este script orquesta el pipeline canonico:
#
#    1. `cargo build --release --target wasm32-unknown-unknown -p pluma`
#       en el crate `03_ukupacha/wawa/apps/pluma`. Forja un binario de
#       Ring 3 con `opt-level = "z" + lto = true + codegen-units = 1`.
#
#    2. `wasm-opt -Os --strip-debug --strip-producers` sobre el artefacto.
#       Es la utilidad de optimizacion binaria del proyecto Binaryen; aqui
#       purga las custom sections de Rust (`producers`, `target_features`,
#       restos de info de debug), reorganiza los indices de funciones,
#       compacta la emision LEB128 y aplica una pasada agresiva de DCE
#       sobre el arbol de llamadas estaticas. Anadimos
#       `--enable-bulk-memory` porque rustc emite `memory.copy` /
#       `memory.fill` por defecto y wasm-opt rechazaria validarlos sin
#       la feature habilitada.
#
#    3. Reporta las metricas y, sin friccion, deposita el binario sellado
#       en `03_ukupacha/wawa/wawa-kernel/assets/pluma.wasm` —el directorio
#       que `boot` lee al sembrar el grafo de objetos del disco virgen—.
#
#  Politica del Manifiesto: el footprint objetivo nominal del bytecode de
#  Pluma es < 10 KiB estrictos. Si el optimizado lo cumple, el script
#  emite veredicto "OK"; si lo excede, advierte sin abortar — la
#  consolidacion en assets/ ocurre igual, el operador decide si la
#  proxima reduccion merece otra Fase.
#
#  Localizacion de `wasm-opt`: el script consulta, en orden, el PATH, el
#  bin de `~/.cargo`, y el utilitario que Flutter empaqueta dentro del
#  dart-sdk —el unico binario `wasm-opt` que el toolchain de Artix Linux
#  trae por defecto sin pedir `cargo install wasm-opt`—. Asi el pipeline
#  funciona en entornos minimos sin pedir dependencias adicionales.
#
#  Uso:
#     ./scripts/build-pluma.sh           # compila + optimiza + consolida
#     ./scripts/build-pluma.sh --debug   # solo compila + reporta tamano crudo
# =============================================================================

set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$RAIZ/03_ukupacha/wawa/apps/pluma"
TARGET="$APP/target/wasm32-unknown-unknown/release/pluma.wasm"
ASSETS="$RAIZ/03_ukupacha/wawa/wawa-kernel/assets"
SALIDA="$ASSETS/pluma.wasm"

if [ -t 1 ]; then
    ROJO='\033[31m'
    VERDE='\033[32m'
    AMARILLO='\033[33m'
    AZUL='\033[36m'
    RESET='\033[0m'
else
    ROJO=''; VERDE=''; AMARILLO=''; AZUL=''; RESET=''
fi

# --- 0. Localizar wasm-opt -------------------------------------------------
WASM_OPT=""
for cand in \
        "$(command -v wasm-opt 2>/dev/null || true)" \
        "$HOME/.cargo/bin/wasm-opt" \
        "/opt/flutter/bin/cache/dart-sdk/bin/utils/wasm-opt" \
        "$HOME/.cache/flutter_sdk/bin/cache/dart-sdk/bin/utils/wasm-opt"; do
    if [ -n "$cand" ] && [ -x "$cand" ]; then
        WASM_OPT="$cand"
        break
    fi
done
if [ -z "$WASM_OPT" ]; then
    echo -e "${ROJO}FALLO${RESET}: wasm-opt no encontrado en PATH, ~/.cargo/bin ni en el dart-sdk de Flutter."
    echo "       Instala Binaryen (\`pacman -S binaryen\`) o cargo install wasm-opt."
    exit 2
fi
echo -e "${AZUL}[wawa/build-pluma]${RESET} wasm-opt ::  $WASM_OPT"

# --- 1. cargo build --release ----------------------------------------------
echo -e "${AZUL}[wawa/build-pluma]${RESET} cargo build --release --target wasm32-unknown-unknown -p pluma"
(cd "$APP" && cargo build --release --target wasm32-unknown-unknown --quiet)

if [ ! -f "$TARGET" ]; then
    echo -e "${ROJO}FALLO${RESET}: cargo no produjo el binario $TARGET"
    exit 1
fi
TAM_CRUDO=$(stat -c '%s' "$TARGET")
echo -e "${AZUL}[wawa/build-pluma]${RESET} crudo ::          ${TAM_CRUDO} bytes"

# --- 2. wasm-opt -----------------------------------------------------------
mkdir -p "$ASSETS"
echo -e "${AZUL}[wawa/build-pluma]${RESET} wasm-opt -Os --strip-debug --strip-producers --enable-bulk-memory"
"$WASM_OPT" \
    -Os \
    --strip-debug \
    --strip-producers \
    --strip-target-features \
    --enable-bulk-memory \
    "$TARGET" \
    -o "$SALIDA"

TAM_OPT=$(stat -c '%s' "$SALIDA")
KIB=$(awk -v b="$TAM_OPT" 'BEGIN { printf "%.2f", b/1024 }')
DELTA=$(( TAM_CRUDO - TAM_OPT ))
PORC=$(awk -v c="$TAM_CRUDO" -v o="$TAM_OPT" 'BEGIN { printf "%.1f", (c-o)*100.0/c }')

echo -e "${AZUL}[wawa/build-pluma]${RESET} sellado ::        ${TAM_OPT} bytes (${KIB} KiB) — purga ${DELTA} B (${PORC}%)"
echo -e "${AZUL}[wawa/build-pluma]${RESET} consolidado en :: ${SALIDA#$RAIZ/}"

# --- 3. Veredicto contra el techo del manifiesto ---------------------------
TECHO_NOMINAL=10240
if [ "$TAM_OPT" -lt "$TECHO_NOMINAL" ]; then
    echo -e "${VERDE}OK${RESET}    pluma.wasm < 10 KiB estrictos del manifiesto"
else
    EXCESO=$(( TAM_OPT - TECHO_NOMINAL ))
    echo -e "${AMARILLO}AVISO${RESET} pluma.wasm excede el techo nominal de 10 KiB por ${EXCESO} B"
    echo "       La consolidacion procede; revisar el arbol de llamadas si la cifra escala."
fi
