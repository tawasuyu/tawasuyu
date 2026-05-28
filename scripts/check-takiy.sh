#!/usr/bin/env bash
# =============================================================================
#  scripts/check-takiy.sh — verificación end-to-end de takiy
# -----------------------------------------------------------------------------
#  Ley práctica: takiy se considera saludable cuando, en una sola corrida,
#    1. todos sus crates compilan limpios,
#    2. todos sus tests unitarios pasan,
#    3. el example smoke (sin audio device, sin display) corre y sale 0,
#    4. el test de determinismo del WAV se mantiene byte-equal contra el
#       hash registrado (regresión silenciosa = error).
#
#  Pensado para CI y para devs locales que tocan el render, el modelo o el
#  formato de archivos.
#
#  Uso:
#      ./scripts/check-takiy.sh            # todo
#      ./scripts/check-takiy.sh fast       # sólo check + smoke (sin tests)
# =============================================================================

set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$RAIZ"

if [ -t 1 ]; then
    ROJO='\033[31m'
    VERDE='\033[32m'
    AMARILLO='\033[33m'
    RESET='\033[0m'
else
    ROJO=''
    VERDE=''
    AMARILLO=''
    RESET=''
fi

modo="${1:-full}"

CRATES=(
    "takiy-core"
    "takiy-synth"
    "takiy-midi"
    "takiy-playback"
    "takiy-app-llimphi"
)

paso() {
    echo
    echo -e "${AMARILLO}=== $1 ===${RESET}"
}

fallar() {
    echo -e "${ROJO}FALLA${RESET}: $1"
    exit 1
}

# 1) cargo check.
paso "cargo check de los crates takiy"
for c in "${CRATES[@]}"; do
    if cargo check --quiet -p "$c"; then
        echo -e "  ${VERDE}OK${RESET}    $c"
    else
        fallar "cargo check -p $c"
    fi
done

# 2) cargo test (omitido en modo "fast").
if [ "$modo" != "fast" ]; then
    paso "cargo test de los crates takiy"
    for c in "${CRATES[@]}"; do
        if cargo test --quiet -p "$c"; then
            echo -e "  ${VERDE}OK${RESET}    $c"
        else
            fallar "cargo test -p $c"
        fi
    done
fi

# 3) Smoke example — corre la lógica del editor sin abrir ventana ni device.
paso "example smoke (headless)"
if cargo run --quiet -p takiy-app-llimphi --example smoke; then
    echo -e "  ${VERDE}OK${RESET}    smoke example"
else
    fallar "cargo run -p takiy-app-llimphi --example smoke"
fi

# 4) Determinismo WAV — el test ya corre en (2) pero lo aislamos arriba para
#    no perder visibilidad si alguien comenta el resto.
if [ "$modo" != "fast" ]; then
    paso "determinismo del WAV (hash)"
    if cargo test --quiet -p takiy-synth --test wav_determinism; then
        echo -e "  ${VERDE}OK${RESET}    wav_determinism"
    else
        fallar "wav_determinism — actualizar EXPECTED_BLAKE3 si el cambio fue intencional"
    fi
fi

echo
echo -e "${VERDE}TAKIY VERDE${RESET}"
