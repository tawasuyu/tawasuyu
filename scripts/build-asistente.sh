#!/usr/bin/env bash
# =============================================================================
#  scripts/build-asistente.sh — FASE 60 v5 :: consolidacion del asistente.wasm
# -----------------------------------------------------------------------------
#  Espejo de `build-pluma.sh` para la app `asistente.wasm` (conversacional
#  wawa <-> LLM via puente Linux). El pipeline es identico al de Pluma:
#
#    1. `cargo build --release --target wasm32-unknown-unknown` dentro de
#       `03_ukupacha/wawa/apps/asistente`. El crate vive fuera del
#       workspace global (kernel + apps wawa cruzan la frontera del
#       no_std bare-metal) — su `Cargo.toml` declara su propio
#       `[workspace]`. Por eso entramos al directorio.
#
#    2. `wasm-opt -Os --strip-debug --strip-producers --enable-bulk-memory`
#       sobre el artefacto. Misma utilidad de Binaryen que usa Pluma; los
#       mismos flags. La feature `bulk-memory` es necesaria porque rustc
#       emite `memory.copy` / `memory.fill` por defecto.
#
#    3. Consolida el binario sellado en
#       `03_ukupacha/wawa/wawa-kernel/assets/asistente.wasm` — el
#       directorio que `boot` lee al sembrar el grafo de objetos.
#
#  Politica de techo nominal: el asistente.wasm hoy ronda los 7 KiB
#  release; le damos 16 KiB de techo (el doble) para tener margen sin
#  alertar en cada commit que sume un puñado de bytes. Si excede, el
#  script avisa pero no aborta.
#
#  Localizacion de `wasm-opt`: misma cascada que build-pluma.sh (PATH,
#  cargo bin, dart-sdk de Flutter).
#
#  Uso:
#     ./scripts/build-asistente.sh           # compila + optimiza + consolida
# =============================================================================

set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$RAIZ/03_ukupacha/wawa/apps/asistente"
TARGET="$APP/target/wasm32-unknown-unknown/release/asistente.wasm"
ASSETS="$RAIZ/03_ukupacha/wawa/wawa-kernel/assets"
SALIDA="$ASSETS/asistente.wasm"

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
echo -e "${AZUL}[wawa/build-asistente]${RESET} wasm-opt ::  $WASM_OPT"

# --- 1. cargo build --release ----------------------------------------------
echo -e "${AZUL}[wawa/build-asistente]${RESET} cargo build --release --target wasm32-unknown-unknown"
(cd "$APP" && cargo build --release --target wasm32-unknown-unknown --quiet)

if [ ! -f "$TARGET" ]; then
    echo -e "${ROJO}FALLO${RESET}: cargo no produjo el binario $TARGET"
    exit 1
fi
TAM_CRUDO=$(stat -c '%s' "$TARGET")
echo -e "${AZUL}[wawa/build-asistente]${RESET} crudo ::          ${TAM_CRUDO} bytes"

# --- 2. wasm-opt -----------------------------------------------------------
mkdir -p "$ASSETS"
echo -e "${AZUL}[wawa/build-asistente]${RESET} wasm-opt -Os --strip-debug --strip-producers --enable-bulk-memory"
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

echo -e "${AZUL}[wawa/build-asistente]${RESET} sellado ::        ${TAM_OPT} bytes (${KIB} KiB) — purga ${DELTA} B (${PORC}%)"
echo -e "${AZUL}[wawa/build-asistente]${RESET} consolidado en :: ${SALIDA#$RAIZ/}"

# --- 3. Veredicto contra el techo nominal -----------------------------------
TECHO_NOMINAL=16384
if [ "$TAM_OPT" -lt "$TECHO_NOMINAL" ]; then
    echo -e "${VERDE}OK${RESET}    asistente.wasm < 16 KiB del techo nominal"
else
    EXCESO=$(( TAM_OPT - TECHO_NOMINAL ))
    echo -e "${AMARILLO}AVISO${RESET} asistente.wasm excede el techo nominal de 16 KiB por ${EXCESO} B"
    echo "       La consolidacion procede; revisar el arbol de llamadas si la cifra escala."
fi
