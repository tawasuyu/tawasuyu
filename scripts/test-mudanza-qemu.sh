#!/usr/bin/env bash
#
# E2E test del flujo agora ↔ mudanza ↔ wawa-kernel.
#
# El autor lo corre manualmente porque (1) requiere display/QEMU vivo
# que esta sesión no tiene, (2) `boot` panica en sandbox por un quirk
# del build.rs de bootloader-x86_64-uefi (ver reference_wawa_boot_bios_stage).
#
# Lo que hace este script:
#
#   1. Forja una propuesta demo firmada con la seed test [42u8;32]
#      vía `agora-cli wawa forjar-propuesta`.
#   2. Sobreescribe `apps/mudanza/src/propuesta_demo.bin`.
#   3. Reconstruye mudanza WASM (`./scripts/build-pluma.sh` no lo hace —
#      construimos a mano con cargo +nightly).
#   4. Lanza wawa en QEMU vía `wawa-boot`.
#   5. Operador pulsa SPACE en la app mudanza.
#
# Comportamiento esperado:
#
#   - mudanza muestra "ESTADO: -2" + "AUTOR AJENO :: RECHAZADO" porque la
#     pubkey demo no está en AGORA_AUTH_RING.
#
# Para que el sobre sea ACEPTADO (re-anclando manifiesto de verdad), hay
# que forjar la propuesta con una de las claves del anillo soberano —
# `agora-cli wawa forjar-clave` imprime el bloque listo para empotrar en
# `wawa-kernel/src/claves.rs:AGORA_AUTH_RING` antes de re-build del
# kernel.

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"

# 1) Construir agora-cli si no existe.
if ! [ -x "$ROOT/target/release/agora-cli" ]; then
    cargo build -p agora-cli --release
fi
CLI="$ROOT/target/release/agora-cli"

# 2) Setup keystore aislado para la demo. NO tocar el ~/.local del operador.
DEMO_HOME=$(mktemp -d)
export HOME="$DEMO_HOME"
export XDG_DATA_HOME="$DEMO_HOME/.local/share"
export AGORA_PASSPHRASE="demo-mudanza"

# 3) Forjar la identidad demo a partir de la seed [42u8;32] (la misma
#    que el example histórico). Eso garantiza que la pubkey resultante
#    coincida con el fixture vivo si nadie la cambió.
SEED_HEX=$(printf '2a%.0s' {1..32})  # 0x2a = 42, repetido 32 veces
echo "$SEED_HEX" | "$CLI" identidad nueva --name demo --seed-stdin >/dev/null
DEMO_ID=$("$CLI" identidad listar | awk '/demo$/ {print $2}')
echo "Identidad demo: $DEMO_ID"

# 4) Forjar la propuesta. El hash es BLAKE3 de
#    "agora-mudanza-demo-manifiesto" — lo que el example clásico usaba.
#    blake3sum no está garantizado; usamos un Python inline.
HASH=$(python3 -c "import hashlib; h=hashlib.blake2b(b'agora-mudanza-demo-manifiesto', digest_size=32); print(h.hexdigest())" 2>/dev/null || \
       printf 'c%.0s' {1..64})  # fallback: 64 chars 'c' si python3 no está

"$CLI" wawa forjar-propuesta \
    --como "$DEMO_ID" \
    --hash "$HASH" \
    --salida "$ROOT/03_ukupacha/wawa/apps/mudanza/src/propuesta_demo.bin"

# 5) Re-build mudanza WASM.
cd "$ROOT/03_ukupacha/wawa/apps/mudanza"
cargo +nightly build --target wasm32-unknown-unknown --release

# 6) Lanzar wawa en QEMU. wawa-boot:
#    - construye la imagen UEFI (bootloader + kernel + assets)
#    - corre QEMU con OVMF
#
#    Variables útiles:
#      RENASER_OVMF=/usr/share/OVMF/x64/OVMF.4m.fd (Artix)
#      RENASER_KERNEL_LOG=1 para tracing extra
cd "$ROOT/03_ukupacha/wawa"
RENASER_OVMF=${RENASER_OVMF:-/usr/share/OVMF/x64/OVMF.4m.fd} \
    cargo +nightly run -p boot -Z bindeps

# Cleanup del HOME demo.
rm -rf "$DEMO_HOME"
