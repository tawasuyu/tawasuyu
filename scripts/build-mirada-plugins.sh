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
# Catálogo base.
forjar "mirada-plugin-dwindle"         "mirada_plugin_dwindle"         "dwindle"

# --- Firma DEMO del reactor (pide CAP_KEYS+CAP_SPAWN → requiere firma) --------
# La semilla es FIJA y PÚBLICA: sólo para que el ejemplo corra de fábrica. NO es
# un secreto real; en tu instalación generás la tuya con `mirada-plugin-sign
# keygen` y reemplazás trust.ron con tu pubkey.
DEMO_SEED="1100110011001100110011001100110011001100110011001100110011001100"
SEED_FILE="$(mktemp)"
printf '%s' "$DEMO_SEED" > "$SEED_FILE"

echo -e "${AZUL}[mirada-plugins]${RESET} firmando example-reactor (demo)…"
SIGN_OUT="$(cd "$RAIZ" && cargo run -q -p mirada-plugin-host --bin mirada-plugin-sign -- \
    sign --seed "$SEED_FILE" --wasm "$ASSETS/example-reactor.wasm" --caps keys,spawn,effects,actions)"
rm -f "$SEED_FILE"

SIGNER="$(printf '%s\n' "$SIGN_OUT" | grep -oE 'ed25519:[0-9a-f]+' | head -1)"
SIGNATURE="$(printf '%s\n' "$SIGN_OUT" | grep -oE 'signature: "[0-9a-f]+"' | grep -oE '[0-9a-f]{128}')"
if [ -z "$SIGNER" ] || [ -z "$SIGNATURE" ]; then
    echo "FALLO: no se pudo firmar el reactor (salida:)"; printf '%s\n' "$SIGN_OUT"; exit 1
fi

cat > "$ASSETS/example-reactor.ron" <<EOF
// Manifest del plugin reactor de ejemplo (terminal + dimming + auto-teselado).
// Pide capacidades peligrosas → requiere firma de una clave de confianza.
// La firma de abajo la regenera build-mirada-plugins.sh con una semilla DEMO
// pública (NO un secreto). En tu instalación: firmá con TU clave.
(
    wasm: "example-reactor.wasm",
    kind: Reactor,
    caps: ["keys", "spawn", "effects", "actions"],
    priority: 0,
    signer: "$SIGNER",
    signature: "$SIGNATURE",
)
EOF

cat > "$ASSETS/trust.ron" <<EOF
// Anillo de confianza de EJEMPLO: la clave DEMO que firma example-reactor.
// En tu instalación, reemplazá esto por TU pubkey (mirada-plugin-sign keygen).
(
    trusted: [
        "$SIGNER",
    ],
)
EOF

echo -e "${VERDE}[mirada-plugins]${RESET} reactor firmado por $SIGNER"
echo -e "${VERDE}[mirada-plugins]${RESET} listo → $ASSETS"
