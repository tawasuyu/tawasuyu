#!/usr/bin/env bash
# test-arje-loader-qemu.sh — smoke del MENÚ de arje-loader en QEMU+OVMF.
#
# Buildea arje-loader (x86_64-unknown-uefi), stagea una ESP con VARIAS entries
# `/loader/entries/*.conf`, arranca el loader bajo OVMF, y como el menú es
# GRÁFICO (lo pinta sobre el framebuffer del GOP, no por texto) lo certifica con
# una CAPTURA de pantalla (screendump del monitor de QEMU → PNG). Headless.
#
#   scripts/test-arje-loader-qemu.sh            # captura a /tmp y la deja lista
#   SHOT_MS=4500 ./scripts/...                  # cuándo capturar (def 4500 ms)
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

LOADER=03_ukupacha/arje/init/arje-loader
SHOT_MS="${SHOT_MS:-6500}"
have() { command -v "$1" >/dev/null 2>&1; }
die()  { echo "✗ $*" >&2; exit 1; }
have qemu-system-x86_64 || die "falta qemu-system-x86_64"
have socat || die "falta socat"
have convert || die "falta imagemagick (convert)"

find_first() { for f in "$@"; do [ -f "$f" ] && { echo "$f"; return; }; done; }
OVMF_CODE="${OVMF_CODE:-$(find_first /usr/share/edk2/x64/OVMF_CODE.4m.fd /usr/share/edk2/ovmf/OVMF_CODE.fd /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd /usr/share/edk2-ovmf/x64/OVMF_CODE.fd)}"
OVMF_VARS="${OVMF_VARS:-$(find_first /usr/share/edk2/x64/OVMF_VARS.4m.fd /usr/share/edk2/ovmf/OVMF_VARS.fd /usr/share/OVMF/OVMF_VARS_4M.fd /usr/share/OVMF/OVMF_VARS.fd /usr/share/edk2-ovmf/x64/OVMF_VARS.fd)}"
[ -n "$OVMF_CODE" ] && [ -n "$OVMF_VARS" ] || die "no encontré OVMF (instalá edk2-ovmf)"

echo "==> build arje-loader (release, x86_64-unknown-uefi)"
( cd "$LOADER" && cargo build --release --target x86_64-unknown-uefi >/dev/null )
EFI="$LOADER/target/x86_64-unknown-uefi/release/arje-loader.efi"
[ -f "$EFI" ] || die "no se buildeó $EFI"

OUT="$(mktemp -d /tmp/arje-loader-test.XXXXXX)"
trap 'rm -rf "$OUT"' EXIT
ESP="$OUT/esp"
mkdir -p "$ESP/EFI/BOOT" "$ESP/loader/entries"
cp -f "$EFI" "$ESP/EFI/BOOT/BOOTX64.EFI"

# Tres entries de muestra: el menú debe mostrar 3 líneas. No tienen kernel real
# (no nos importa que arranquen: queremos VER el menú); el default es la de arje.
cat > "$ESP/loader/entries/10-arje.conf" <<'E'
title arje — tawasuyu (init alterno)
linux /vmlinuz-arje
initrd /initrd-arje
options root=UUID=demo rw init=/usr/local/sbin/arje-zero quiet
E
cat > "$ESP/loader/entries/20-artix.conf" <<'E'
title Artix Linux (systemd-free)
linux /vmlinuz-linux
initrd /initramfs-linux.img
options root=UUID=demo rw
E
cat > "$ESP/loader/entries/30-wawa.conf" <<'E'
title wawa (SO bare-metal)
linux /wawa.efi
initrd /wawa-initrd
options console=ttyS0
E
# loader.conf: timeout largo para que el menú quede estable durante la captura.
cat > "$ESP/loader/loader.conf" <<'E'
timeout 30
default 10-arje
E

cp -f "$OVMF_VARS" "$OUT/vars.fd"
SOCK="$OUT/mon.sock"
PPM="$OUT/shot.ppm"
PNG="${PNG:-$OUT/arje-loader-menu.png}"

echo "==> QEMU+OVMF (headless) — captura a los ${SHOT_MS} ms"
qemu-system-x86_64 -machine q35 -m 256M \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
    -drive if=pflash,format=raw,file="$OUT/vars.fd" \
    -drive format=raw,file="fat:rw:$ESP" \
    -vga std -display none \
    -monitor "unix:$SOCK,server,nowait" \
    -serial "file:$OUT/serial.log" </dev/null &
QPID=$!
trap 'kill $QPID 2>/dev/null || true; rm -rf "$OUT"' EXIT

# Esperar a que OVMF arranque + el loader pinte el menú, y capturar.
sleep "$(awk "BEGIN{print $SHOT_MS/1000}")"
[ -S "$SOCK" ] || die "el monitor de QEMU no levantó (¿QEMU murió? ver $OUT/serial.log)"
echo "screendump $PPM" | socat - "UNIX-CONNECT:$SOCK" >/dev/null 2>&1 || true
sleep 0.6
kill $QPID 2>/dev/null || true

[ -f "$PPM" ] || die "no se generó la captura (serial: $(tail -3 "$OUT/serial.log" 2>/dev/null))"
convert "$PPM" "$PNG"
# Sacar el PNG del OUT (que se borra) a una ruta estable.
FINAL="${PNG_OUT:-/tmp/arje-loader-menu.png}"
cp -f "$PNG" "$FINAL"
echo "✓ captura del menú: $FINAL"
echo "  serial (últimas líneas):"; tail -5 "$OUT/serial.log" 2>/dev/null | sed 's/^/    /' || true
