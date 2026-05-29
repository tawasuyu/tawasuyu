#!/usr/bin/env bash
# =============================================================================
#  scripts/build-tinkuy.sh — C5 del roadmap tinkuy (capa ABI WASM)
# -----------------------------------------------------------------------------
#  Pipeline canónico de compactación del cdylib `tinkuy` (motor de partículas
#  DOD vía ABI plana `tk_sim_*`) para userspace de Wawa:
#
#    1. `cargo build --release --target wasm32-unknown-unknown` en el crate
#       `03_ukupacha/wawa/apps/tinkuy` (perfil endurecido opt-level=z + lto +
#       codegen-units=1 + strip).
#
#    2. `wasm-opt -Os --strip-debug --strip-producers --strip-target-features
#       --enable-bulk-memory` para purgar custom sections, compactar LEB128 y
#       barrer DCE estático sobre el árbol de exports `tk_*`.
#
#    3. Reporta métricas y deposita el binario en
#       `03_ukupacha/wawa/wawa-kernel/assets/tinkuy.wasm` — el directorio que
#       `boot` lee al sembrar el grafo de objetos del disco virgen.
#
#  Localización de `wasm-opt`: copiada de `build-pluma.sh` (PATH → ~/.cargo →
#  dart-sdk de Flutter) para que el pipeline funcione en entornos mínimos sin
#  dependencias adicionales.
#
#  Uso:
#     ./scripts/build-tinkuy.sh           # compila + optimiza + consolida
#     ./scripts/build-tinkuy.sh --debug   # solo compila + reporta tamaño crudo
# =============================================================================

set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$RAIZ/03_ukupacha/wawa/apps/tinkuy"
TARGET="$APP/target/wasm32-unknown-unknown/release/tinkuy.wasm"
ASSETS="$RAIZ/03_ukupacha/wawa/wawa-kernel/assets"
SALIDA="$ASSETS/tinkuy.wasm"

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
echo -e "${AZUL}[wawa/build-tinkuy]${RESET} wasm-opt ::  $WASM_OPT"

# --- 1. cargo build --release ----------------------------------------------
echo -e "${AZUL}[wawa/build-tinkuy]${RESET} cargo build --release --target wasm32-unknown-unknown"
(cd "$APP" && cargo build --release --target wasm32-unknown-unknown --quiet)

if [ ! -f "$TARGET" ]; then
    echo -e "${ROJO}FALLO${RESET}: cargo no produjo el binario $TARGET"
    exit 1
fi
TAM_CRUDO=$(stat -c '%s' "$TARGET")
echo -e "${AZUL}[wawa/build-tinkuy]${RESET} crudo ::          ${TAM_CRUDO} bytes"

if [ "${1-}" = "--debug" ]; then
    echo -e "${AZUL}[wawa/build-tinkuy]${RESET} --debug pedido; no se ejecuta wasm-opt"
    exit 0
fi

# --- 2. wasm-opt -----------------------------------------------------------
mkdir -p "$ASSETS"
echo -e "${AZUL}[wawa/build-tinkuy]${RESET} wasm-opt -Os --strip-debug --strip-producers (+ bulk-memory/sign-ext/nontrapping/mutable-globals)"
# rustc para wasm32-unknown-unknown emite por defecto: bulk-memory (memory.copy),
# nontrapping-fptoint (trunc_sat — usado por nuestros casts f32→u32 en grid),
# sign-ext (i32.extend8_s) y mutable-globals. Hay que habilitarlas explícitamente
# en wasm-opt para que el validador no rechace el binario.
"$WASM_OPT" \
    -Os \
    --strip-debug \
    --strip-producers \
    --strip-target-features \
    --enable-bulk-memory \
    --enable-sign-ext \
    --enable-nontrapping-float-to-int \
    --enable-mutable-globals \
    "$TARGET" \
    -o "$SALIDA"

TAM_OPT=$(stat -c '%s' "$SALIDA")
KIB=$(awk -v b="$TAM_OPT" 'BEGIN { printf "%.2f", b/1024 }')
DELTA=$(( TAM_CRUDO - TAM_OPT ))
PORC=$(awk -v c="$TAM_CRUDO" -v o="$TAM_OPT" 'BEGIN { printf "%.1f", (c-o)*100.0/c }')

echo -e "${AZUL}[wawa/build-tinkuy]${RESET} sellado ::        ${TAM_OPT} bytes (${KIB} KiB) — purga ${DELTA} B (${PORC}%)"
echo -e "${AZUL}[wawa/build-tinkuy]${RESET} consolidado en :: ${SALIDA#$RAIZ/}"

# --- 3. Veredicto contra el techo del roadmap -------------------------------
# 200 KiB es el techo declarado en PLAN.md §C3. tinkuy es un motor numérico con
# blake3 + ECS SoA + neighbor-list — más pesado que pluma (Forth puro), por eso
# el umbral es 20× más alto que el de pluma.
TECHO_NOMINAL=204800
if [ "$TAM_OPT" -lt "$TECHO_NOMINAL" ]; then
    echo -e "${VERDE}OK${RESET}    tinkuy.wasm < 200 KiB del roadmap"
else
    EXCESO=$(( TAM_OPT - TECHO_NOMINAL ))
    echo -e "${AMARILLO}AVISO${RESET} tinkuy.wasm excede el techo nominal de 200 KiB por ${EXCESO} B"
    echo "       Revisar dependencias arrastradas (blake3 features, etc.)."
fi
