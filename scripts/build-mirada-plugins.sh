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
forjar "mirada-plugin-asignador"       "mirada_plugin_asignador"       "asignador"
# Layouts del catálogo (sin firma — no importan nada del host; su .ron es estático).
forjar "mirada-plugin-three-column"    "mirada_plugin_three_column"    "three-column"
forjar "mirada-plugin-fibonacci"       "mirada_plugin_fibonacci"       "fibonacci"
forjar "mirada-plugin-grid"            "mirada_plugin_grid"            "grid"
# Reactores del catálogo (piden caps peligrosas → se firman abajo).
forjar "mirada-plugin-scratchpads"      "mirada_plugin_scratchpads"      "scratchpads"
forjar "mirada-plugin-orientacion"      "mirada_plugin_orientacion"      "orientacion"
forjar "mirada-plugin-nueva-al-maestro" "mirada_plugin_nueva_al_maestro" "nueva-al-maestro"
forjar "mirada-plugin-media-keys"       "mirada_plugin_media_keys"       "media-keys"
forjar "mirada-plugin-efecto-por-app"   "mirada_plugin_efecto_por_app"   "efecto-por-app"

# --- Firma DEMO de los reactores (piden caps peligrosas → requieren firma) ----
# La semilla es FIJA y PÚBLICA: sólo para que el ejemplo corra de fábrica. NO es
# un secreto real; en tu instalación generás la tuya con `mirada-plugin-sign
# keygen` y reemplazás trust.ron con tu pubkey.
DEMO_SEED="1100110011001100110011001100110011001100110011001100110011001100"
SEED_FILE="$(mktemp)"
printf '%s' "$DEMO_SEED" > "$SEED_FILE"

# Firma `blake3(wasm) ‖ caps`; setea las globales SIGNER (pubkey) y SIG (firma
# hex). Se llama SIN `$(...)` para que las globales sobrevivan (la subshell de la
# sustitución se las llevaría).
firma_de() {  # $1 = salida, $2 = caps (csv)
    local out
    out="$(cd "$RAIZ" && cargo run -q -p mirada-plugin-host --bin mirada-plugin-sign -- \
        sign --seed "$SEED_FILE" --wasm "$ASSETS/$1.wasm" --caps "$2")"
    SIGNER="$(printf '%s\n' "$out" | grep -oE 'ed25519:[0-9a-f]+' | head -1)"
    SIG="$(printf '%s\n' "$out" | grep -oE 'signature: "[0-9a-f]+"' | grep -oE '[0-9a-f]{128}')"
    if [ -z "$SIGNER" ] || [ -z "$SIG" ]; then
        echo "FALLO: no se pudo firmar $1 (salida:)"; printf '%s\n' "$out"; exit 1
    fi
}

echo -e "${AZUL}[mirada-plugins]${RESET} firmando reactores (demo)…"
firma_de example-reactor keys,spawn,effects,actions
SIG_REACTOR="$SIG"
firma_de asignador actions
SIG_ASIGNADOR="$SIG"
firma_de scratchpads keys,actions
SIG_SCRATCHPADS="$SIG"
firma_de orientacion actions
SIG_ORIENTACION="$SIG"
firma_de nueva-al-maestro actions
SIG_NUEVA="$SIG"
firma_de media-keys keys,spawn
SIG_MEDIA="$SIG"
firma_de efecto-por-app effects
SIG_EFECTO="$SIG"
rm -f "$SEED_FILE"

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
    signature: "$SIG_REACTOR",
)
EOF

cat > "$ASSETS/asignador.ron" <<EOF
// Manifest del plugin «asignador»: enruta ventanas por app_id (CAP_ACTIONS →
// requiere firma). El campo 'config' (NO entra en la firma) trae las reglas;
// editalo a mano o desde wawa-panel. Por defecto, sólo ejemplos comentados (no-op).
(
    wasm: "asignador.wasm",
    kind: Reactor,
    caps: ["actions"],
    priority: 0,
    signer: "$SIGNER",
    signature: "$SIG_ASIGNADOR",
    config: "# Reglas: <app_id-substring>  <escritorio 1-9 y/o «float»>\n# Descomenta y ajusta a tus apps:\n# firefox      2\n# Alacritty    1\n# pavucontrol  float\n",
)
EOF

cat > "$ASSETS/scratchpads.ron" <<EOF
// Manifest del plugin «scratchpads con nombre»: atajos → escritorios especiales
// (CAP_KEYS + CAP_ACTIONS → requiere firma). El campo 'config' (NO firmado) trae
// los binds; editalo a mano o desde wawa-panel. Sin binds no registra atajos.
(
    wasm: "scratchpads.wasm",
    kind: Reactor,
    caps: ["keys", "actions"],
    priority: 0,
    signer: "$SIGNER",
    signature: "$SIG_SCRATCHPADS",
    config: "# <tecla>  [verbo]  <nombre>   ·   verbo: toggle (default) | send\n# Descomenta y ajusta a tu gusto:\n# Super+grave        dev\n# Super+Shift+grave  send  dev\n# Super+n            notas\n# Super+Shift+n      send  notas\n",
)
EOF

cat > "$ASSETS/orientacion.ron" <<EOF
// Manifest del plugin «orientación adaptativa»: vertical→rows, apaisado→columns
// (CAP_ACTIONS → requiere firma). Sin config.
(
    wasm: "orientacion.wasm",
    kind: Reactor,
    caps: ["actions"],
    priority: 0,
    signer: "$SIGNER",
    signature: "$SIG_ORIENTACION",
)
EOF

cat > "$ASSETS/nueva-al-maestro.ron" <<EOF
// Manifest del plugin «nueva al maestro»: promueve cada ventana nueva al área
// maestra (CAP_ACTIONS → requiere firma). Sin config.
(
    wasm: "nueva-al-maestro.wasm",
    kind: Reactor,
    caps: ["actions"],
    priority: 0,
    signer: "$SIGNER",
    signature: "$SIG_NUEVA",
)
EOF

cat > "$ASSETS/media-keys.ron" <<EOF
// Manifest del plugin «teclas de medios»: teclas XF86 → wpctl/brightnessctl/
// playerctl/grim (CAP_KEYS + CAP_SPAWN → requiere firma). Trae defaults; el
// campo 'config' (NO firmado) los ajusta — línea con sólo la tecla la borra.
(
    wasm: "media-keys.wasm",
    kind: Reactor,
    caps: ["keys", "spawn"],
    priority: 0,
    signer: "$SIGNER",
    signature: "$SIG_MEDIA",
    config: "# <tecla XF86>  <comando…>   (línea con sólo la tecla = borrar ese default)\n# Ejemplos:\n# XF86AudioRaiseVolume  wpctl set-volume @DEFAULT_AUDIO_SINK@ 10%+\n# Print  grim -g \"\$(slurp)\" ~/Pictures/recorte.png\n",
)
EOF

cat > "$ASSETS/efecto-por-app.ron" <<EOF
// Manifest del plugin «efecto por app»: opacidad/sombra por app_id (CAP_EFFECTS
// → requiere firma). El campo 'config' (NO firmado) trae las reglas; editalo a
// mano o desde wawa-panel. Sin reglas no hace nada.
(
    wasm: "efecto-por-app.wasm",
    kind: Reactor,
    caps: ["effects"],
    priority: 0,
    signer: "$SIGNER",
    signature: "$SIG_EFECTO",
    config: "# <app_id-substring>  <opacidad 0-100>  [shadow|noshadow]\n# Descomenta y ajusta a tus apps:\n# Alacritty   88\n# foot        85  noshadow\n# mpv         100 noshadow\n",
)
EOF

cat > "$ASSETS/trust.ron" <<EOF
// Anillo de confianza de EJEMPLO: la clave DEMO que firma los reactores.
// En tu instalación, reemplazá esto por TU pubkey (mirada-plugin-sign keygen).
(
    trusted: [
        "$SIGNER",
    ],
)
EOF

echo -e "${VERDE}[mirada-plugins]${RESET} reactores firmados por $SIGNER"
echo -e "${VERDE}[mirada-plugins]${RESET} listo → $ASSETS"
