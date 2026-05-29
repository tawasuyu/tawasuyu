#!/usr/bin/env bash
# =============================================================================
#  scripts/build-testigo.sh — pipeline de la app userspace `testigo`
# -----------------------------------------------------------------------------
#  Misma forma que build-rimay.sh / build-pluma.sh: cargo build -> wasm-opt ->
#  consolidacion en wawa-kernel/assets/. `testigo` es la app que cierra la
#  capa 2 de tinkuy (Fase C4): ejerce las syscalls `sys_tinkuy_*` del kernel y
#  pinta step/T/CID del motor de particulas empotrado.
#
#    1. cargo build --release --target wasm32-unknown-unknown -p testigo.
#    2. wasm-opt -Os --strip-debug --strip-producers (mas las features que la
#       app necesita: bulk-memory, nontrapping-float-to-int — el render de
#       observables f64 entra en `as u32` cuando dibujamos la barra).
#    3. Reporta tamaño y consolida en wawa-kernel/assets/testigo.wasm.
#
#  Techo nominal: 12 KiB (un pelo mas que rimay/pluma porque
#  carga rasterizado f64 → barra ancho-proporcional).
# =============================================================================

set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$RAIZ/03_ukupacha/wawa/apps/testigo"
TARGET="$APP/target/wasm32-unknown-unknown/release/testigo.wasm"
ASSETS="$RAIZ/03_ukupacha/wawa/wawa-kernel/assets"
SALIDA="$ASSETS/testigo.wasm"

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
echo -e "${AZUL}[wawa/build-testigo]${RESET} wasm-opt ::  $WASM_OPT"

# --- 1. cargo build --release ----------------------------------------------
echo -e "${AZUL}[wawa/build-testigo]${RESET} cargo build --release --target wasm32-unknown-unknown -p testigo"
(cd "$APP" && cargo build --release --target wasm32-unknown-unknown --quiet)

if [ ! -f "$TARGET" ]; then
    echo -e "${ROJO}FALLO${RESET}: cargo no produjo el binario $TARGET"
    exit 1
fi
TAM_CRUDO=$(stat -c '%s' "$TARGET")
echo -e "${AZUL}[wawa/build-testigo]${RESET} crudo ::          ${TAM_CRUDO} bytes"

if [ "${1:-}" = "--debug" ]; then
    echo -e "${AZUL}[wawa/build-testigo]${RESET} --debug ::        salida en $TARGET (sin sellar)"
    exit 0
fi

# --- 2. wasm-opt -----------------------------------------------------------
mkdir -p "$ASSETS"
echo -e "${AZUL}[wawa/build-testigo]${RESET} wasm-opt -Os --strip-debug --strip-producers --enable-bulk-memory --enable-nontrapping-float-to-int"
"$WASM_OPT" \
    -Os \
    --strip-debug \
    --strip-producers \
    --strip-target-features \
    --enable-bulk-memory \
    --enable-nontrapping-float-to-int \
    "$TARGET" \
    -o "$SALIDA"

TAM_OPT=$(stat -c '%s' "$SALIDA")
KIB=$(awk -v b="$TAM_OPT" 'BEGIN { printf "%.2f", b/1024 }')
DELTA=$(( TAM_CRUDO - TAM_OPT ))
PORC=$(awk -v c="$TAM_CRUDO" -v o="$TAM_OPT" 'BEGIN { printf "%.1f", (c-o)*100.0/c }')

echo -e "${AZUL}[wawa/build-testigo]${RESET} sellado ::        ${TAM_OPT} bytes (${KIB} KiB) — purga ${DELTA} B (${PORC}%)"
echo -e "${AZUL}[wawa/build-testigo]${RESET} consolidado en :: ${SALIDA#$RAIZ/}"

# --- 3. Veredicto contra el techo nominal ----------------------------------
TECHO_NOMINAL=12288
if [ "$TAM_OPT" -lt "$TECHO_NOMINAL" ]; then
    echo -e "${VERDE}OK${RESET}    testigo.wasm < 12 KiB del techo nominal"
else
    EXCESO=$(( TAM_OPT - TECHO_NOMINAL ))
    echo -e "${AMARILLO}AVISO${RESET} testigo.wasm excede el techo nominal por ${EXCESO} B"
fi
