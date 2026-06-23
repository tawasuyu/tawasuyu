#!/usr/bin/env bash
# Smoke test del arranque sin parpadeo de arje en QEMU+OVMF.
# (ver 03_ukupacha/arje/SDD-ARRANQUE-SIN-PARPADEO.md)
#
# Bootea el kernel host por UEFI para que el kernel reciba el framebuffer GOP de
# OVMF → `simpledrm` crea /dev/dri/card0 → `arje-splash` lo toma reusando el modo
# vigente (sin re-modeset) y anima el splash hasta soltar la pantalla.
#
# Por defecto corre HEADLESS y captura el serial: los logs `[arje-splash]`
# certifican la Fase 1 con texto (Regla 8 del CLAUDE.md), sin mirar píxeles.
# Para VERLO de verdad (la animación): `DISPLAY_ARGS= ./scripts/test-arje-splash-qemu.sh`
# (sin -display none abre la ventana de QEMU).
#
# Requisitos: rustup target x86_64-unknown-linux-musl + musl-gcc (binarios
# estáticos: el initramfs no trae libc), qemu-system-x86_64, OVMF, KVM.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

KERNEL="${KERNEL:-/boot/vmlinuz-linux}"
OVMF_CODE="${OVMF_CODE:-/usr/share/edk2/x64/OVMF_CODE.4m.fd}"
OVMF_VARS_SRC="${OVMF_VARS:-/usr/share/edk2/x64/OVMF_VARS.4m.fd}"
SEED="${SEED:-03_ukupacha/arje/seeds/arje-qemu.card.json}"
TARGET="x86_64-unknown-linux-musl"
OUT="${OUT:-$(mktemp -d)}"
TIMEOUT="${TIMEOUT:-40}"
# Headless por defecto; vaciá DISPLAY_ARGS para abrir la ventana de QEMU.
DISPLAY_ARGS="${DISPLAY_ARGS--display none}"

echo "==> compilando binarios estáticos (musl)"
cargo build --release --target "$TARGET" \
  -p arje-zero -p arje-splash -p arje-getty-stub
cargo build --release -p arje-packager

M="target/$TARGET/release"
echo "==> empaquetando initramfs ($SEED)"
./target/release/arje-packager \
  --seed "$SEED" \
  --bin arje-zero="$M/arje-zero" \
  --bin arje-splash="$M/arje-splash" \
  --bin agetty-ttyS0="$M/arje-getty-stub" \
  --out "$OUT/initramfs.cpio.gz"

cp -f "$OVMF_VARS_SRC" "$OUT/OVMF_VARS.fd"
# Orden de console=: el ÚLTIMO es /dev/console, a donde van stdout/stderr de
# arje-zero y sus hijos (arje-splash). En ttyS0 para capturarlo por serial.
APPEND="console=tty0 console=ttyS0,115200 panic=-1 loglevel=7 vt.global_cursor_default=0 i915.fastboot=1"

echo "==> booteando QEMU+OVMF (timeout ${TIMEOUT}s)"
set +e
timeout "$TIMEOUT" qemu-system-x86_64 \
  -machine q35 -m 512M -enable-kvm \
  -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
  -drive if=pflash,format=raw,file="$OUT/OVMF_VARS.fd" \
  -kernel "$KERNEL" -initrd "$OUT/initramfs.cpio.gz" \
  -append "$APPEND" \
  -vga std $DISPLAY_ARGS \
  -serial mon:stdio -no-reboot </dev/null | tee "$OUT/serial.log"
set -e

echo
echo "==> veredicto Fase 1 (logs del splash):"
grep -aE 'arje-splash|simpledrm|reusando modo|page_flip|set_crtc|soltando|Ente (encarnado|disuelto).*splash' \
  "$OUT/serial.log" | sed 's/\x1b\[[0-9;]*m//g' || echo "  (sin logs de arje-splash — revisá $OUT/serial.log)"
