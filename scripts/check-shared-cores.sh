#!/usr/bin/env bash
# =============================================================================
#  scripts/check-shared-cores.sh — script guardian de simetria no_std
# -----------------------------------------------------------------------------
#  Ley inmutable de Wawa: si una estructura de datos viaja por red Akasha,
#  habita el disco direccionado por contenido, o se reparte entre el kernel
#  bare-metal y un proceso de userspace, su crate DEBE compilar sin std.
#
#  Este script enumera los nucleos no_std obligatorios y verifica dos cosas
#  por cada uno:
#    1. el `lib.rs` declara `#![no_std]`;
#    2. la crate compila para `wasm32-unknown-unknown` (un target sin libstd),
#       sin trampas: si lo logra, no esta acoplada a std por accidente.
#
#  El segundo paso es la prueba dura. Una declaracion `#![no_std]` puede
#  convivir con un `use std::...` si el target del workspace anfitrion oculta
#  el error; un build a wasm32 no perdona — std no existe en ese target.
#
#  Uso:
#     ./scripts/check-shared-cores.sh         # valida todos
#     ./scripts/check-shared-cores.sh format  # valida solo `format`
# =============================================================================

set -euo pipefail

# Raiz del repo: este script vive en `scripts/`, asi que subimos un nivel.
RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# --- Nucleos no_std obligatorios. -------------------------------------------
# Cada entrada: "<nombre-corto>|<ruta-relativa-al-Cargo.toml>"
NUCLEOS=(
    "format|shared/format"
    "akasha|03_ukupacha/wawa/wawa-fs"
    "mirada-layout|02_ruway/mirada/mirada-layout"
)

# Colores del informe — silenciados si la salida no es una TTY.
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

verificar_nucleo() {
    local nombre="$1"
    local relativo="$2"
    local ruta="$RAIZ/$relativo"

    echo
    echo "=== $nombre ($relativo) ==="

    if [ ! -d "$ruta" ]; then
        echo -e "  ${ROJO}AUSENTE${RESET}: no existe el directorio $relativo"
        return 1
    fi

    # 1) Declaracion #![no_std] en el lib.rs. Aceptamos tambien la forma
    #    condicional `#![cfg_attr(not(test), no_std)]`: una crate que solo
    #    pide std para sus pruebas sigue siendo no_std bajo el target real.
    if grep -Eq '^#!\[(no_std|cfg_attr\(not\(test\),\s*no_std\))\]' "$ruta/src/lib.rs" 2>/dev/null; then
        echo -e "  ${VERDE}OK${RESET}    declaracion #![no_std]"
    else
        echo -e "  ${ROJO}FALLO${RESET} sin #![no_std] en src/lib.rs"
        return 1
    fi

    # 2) Build para wasm32-unknown-unknown — un target sin libstd. Si la crate
    #    arrastra std por accidente, este paso lo delata.
    if (
        cd "$ruta"
        cargo check --quiet --target wasm32-unknown-unknown 2>&1
    ); then
        echo -e "  ${VERDE}OK${RESET}    cargo check --target wasm32-unknown-unknown"
    else
        echo -e "  ${ROJO}FALLO${RESET} cargo check --target wasm32-unknown-unknown"
        return 1
    fi
}

# --- Filtrar por argumento, si lo hay. --------------------------------------
filtro="${1:-}"

# --- Verificar el toolchain de wasm32. --------------------------------------
if ! rustup target list --installed 2>/dev/null | grep -q "wasm32-unknown-unknown"; then
    echo -e "${AMARILLO}aviso${RESET}: el target wasm32-unknown-unknown no esta instalado;"
    echo "       ejecuta: rustup target add wasm32-unknown-unknown"
    exit 2
fi

# --- Ejecutar. --------------------------------------------------------------
fallos=0
for entrada in "${NUCLEOS[@]}"; do
    nombre="${entrada%%|*}"
    ruta="${entrada##*|}"
    if [ -n "$filtro" ] && [ "$filtro" != "$nombre" ]; then
        continue
    fi
    if ! verificar_nucleo "$nombre" "$ruta"; then
        fallos=$((fallos + 1))
    fi
done

echo
if [ "$fallos" -eq 0 ]; then
    echo -e "${VERDE}TODOS LOS NUCLEOS NO_STD PASAN${RESET}"
    exit 0
else
    echo -e "${ROJO}$fallos NUCLEO(S) FALLAN LA SIMETRIA NO_STD${RESET}"
    exit 1
fi
