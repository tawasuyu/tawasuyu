#!/usr/bin/env bash
# =============================================================================
#  scripts/build-rimay.sh — pipeline de compactación de la app WASM rimay
# -----------------------------------------------------------------------------
#  Mismo pipeline canónico que `build-pluma.sh`: cargo build → wasm-opt →
#  consolidación en wawa-kernel/assets/. La app rimay refleja en bare-metal
#  el subdominio host `00_unanchay/rimay/` — verbo determinista + cosine
#  sobre framebuffer 480x560.
#
#    1. `cargo build --release --target wasm32-unknown-unknown -p rimay`
#       en el crate `03_ukupacha/wawa/apps/rimay`. Profile release con
#       `opt-level = "z" + lto = true + codegen-units = 1`.
#
#    2. `wasm-opt -Os --strip-debug --strip-producers` sobre el artefacto:
#       purga las custom sections de Rust, compacta el LEB128 y aplica
#       DCE agresiva. Añade `--enable-bulk-memory` porque rustc emite
#       `memory.copy`/`memory.fill` por defecto.
#
#    3. Reporta métricas y consolida el binario sellado en
#       `03_ukupacha/wawa/wawa-kernel/assets/rimay.wasm`.
#
#  Política del Manifiesto: el footprint objetivo del bytecode de rimay
#  es < 10 KiB estrictos (igual techo nominal que pluma). Si el optimizado
#  lo cumple, "OK"; si lo excede, advierte sin abortar.
#
#  Localización de `wasm-opt`: PATH → `~/.cargo/bin` → dart-sdk de Flutter
#  (que Artix trae preempaquetado, evitando `cargo install wasm-opt`).
#
#  Uso:
#     ./scripts/build-rimay.sh           # compila + optimiza + consolida
#     ./scripts/build-rimay.sh --debug   # solo compila + reporta tamaño crudo
# =============================================================================

set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$RAIZ/03_ukupacha/wawa/apps/rimay"
TARGET="$APP/target/wasm32-unknown-unknown/release/rimay.wasm"
ASSETS="$RAIZ/03_ukupacha/wawa/wawa-kernel/assets"
SALIDA="$ASSETS/rimay.wasm"

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
echo -e "${AZUL}[wawa/build-rimay]${RESET} wasm-opt ::  $WASM_OPT"

# --- 1. cargo build --release ----------------------------------------------
echo -e "${AZUL}[wawa/build-rimay]${RESET} cargo build --release --target wasm32-unknown-unknown -p rimay"
(cd "$APP" && cargo build --release --target wasm32-unknown-unknown --quiet)

if [ ! -f "$TARGET" ]; then
    echo -e "${ROJO}FALLO${RESET}: cargo no produjo el binario $TARGET"
    exit 1
fi
TAM_CRUDO=$(stat -c '%s' "$TARGET")
echo -e "${AZUL}[wawa/build-rimay]${RESET} crudo ::          ${TAM_CRUDO} bytes"

if [ "${1:-}" = "--debug" ]; then
    echo -e "${AZUL}[wawa/build-rimay]${RESET} --debug ::        salida en $TARGET (sin sellar)"
    exit 0
fi

# --- 2. wasm-opt -----------------------------------------------------------
mkdir -p "$ASSETS"
echo -e "${AZUL}[wawa/build-rimay]${RESET} wasm-opt -Os --strip-debug --strip-producers --enable-bulk-memory --enable-nontrapping-float-to-int"
# `nontrapping-float-to-int` se activa porque rustc emite
# `i32.trunc_sat_f32_u/s` cada vez que un f32 entra en `as u32`/`as usize`
# (formateo del coseno + cálculo del ancho de barra). Sin esa feature
# wasm-opt rechaza el binario como inválido.
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

echo -e "${AZUL}[wawa/build-rimay]${RESET} sellado ::        ${TAM_OPT} bytes (${KIB} KiB) — purga ${DELTA} B (${PORC}%)"
echo -e "${AZUL}[wawa/build-rimay]${RESET} consolidado en :: ${SALIDA#$RAIZ/}"

# --- 3. Veredicto contra el techo del manifiesto ---------------------------
TECHO_NOMINAL=10240
if [ "$TAM_OPT" -lt "$TECHO_NOMINAL" ]; then
    echo -e "${VERDE}OK${RESET}    rimay.wasm < 10 KiB estrictos del manifiesto"
else
    EXCESO=$(( TAM_OPT - TECHO_NOMINAL ))
    echo -e "${AMARILLO}AVISO${RESET} rimay.wasm excede el techo nominal de 10 KiB por ${EXCESO} B"
    echo "       La consolidación procede; revisar el árbol de llamadas si la cifra escala."
fi
