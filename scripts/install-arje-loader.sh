#!/usr/bin/env bash
# install-arje-loader.sh — instala arje-loader como bootloader UEFI gráfico, en
# una entrada de arranque APARTE (no-default, reversible). NO toca tu GRUB.
#
# arje-loader vive en la ESP como /EFI/arje/loader.efi; al elegirlo en el menú
# UEFI del firmware, dibuja su menú gráfico (sobre el GOP, cero modo texto) con
# las entries de /loader/entries/*.conf y bootea la elegida. Como el loader lee
# el kernel/initramfs DESDE la ESP (igual que systemd-boot) y tus kernels viven
# en /boot (en el root), el instalador los COPIA a la ESP.
#
# Entries que crea (cloná tu arranque actual, leído de /proc/cmdline):
#   · «arje — tawasuyu»  → tu kernel + initramfs + init=/usr/local/sbin/arje-zero
#                          + flags sin-parpadeo (tu sesión arje nativa).
#   · «tu sistema»       → el mismo kernel/initramfs SIN init= (tu boot normal).
#
#   install-arje-loader.sh             # instala (interactivo)
#   install-arje-loader.sh --yes       # sin preguntas
#   install-arje-loader.sh --uninstall # quita la entrada NVRAM + los archivos
#   install-arje-loader.sh --help
#
# HONESTIDAD: el menú está verificado en QEMU+OVMF (scripts/test-arje-loader-
# qemu.sh) pero NO en tu metal. Es no-default: si algo falla, en el menú UEFI
# del firmware elegís tu GRUB de siempre. Reversible con --uninstall. Y ojo: si
# actualizás el kernel, re-corré esto para recopiar a la ESP (no hay hook aún).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

LOADER_DIR="03_ukupacha/arje/init/arje-loader"
UEFI_TARGET="x86_64-unknown-uefi"
ESP_GUID="c12a7328-f81f-11d2-ba4b-00a0c93ec93b"
LABEL="arje-loader"
ZERO="/usr/local/sbin/arje-zero"
FLICKER="quiet loglevel=0 vt.global_cursor_default=0 rd.systemd.show_status=false i915.fastboot=1"
ASSUME_YES=0
DO_UNINSTALL=0

die()  { echo "✗ $*" >&2; exit 1; }
info() { echo "==> $*"; }
have() { command -v "$1" >/dev/null 2>&1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --yes|-y)    ASSUME_YES=1 ;;
        --uninstall) DO_UNINSTALL=1 ;;
        -h|--help)   sed -n '2,23p' "$0"; exit 0 ;;
        *) die "opción desconocida: $1 (ver --help)" ;;
    esac
    shift
done

[ -d /sys/firmware/efi ] || die "esta máquina no arrancó en UEFI; arje-loader es UEFI-only."
have efibootmgr || die "falta efibootmgr (instalalo) — lo necesito para la opción de arranque."

# ── sudo cacheado para los pasos privilegiados ───────────────────────────────
if [ "$(id -u)" != 0 ]; then
    have sudo || die "necesito sudo para montar la ESP, copiar y efibootmgr."
    sudo -v || die "sudo denegado."
    SUDO="sudo"
else
    SUDO=""
fi

# ── Descubrir + montar la ESP (igual criterio que install-arje.sh) ───────────
TMP_MNT=""
cleanup() { [ -n "$TMP_MNT" ] && { $SUDO umount "$TMP_MNT" 2>/dev/null || true; rmdir "$TMP_MNT" 2>/dev/null || true; }; }
trap cleanup EXIT INT TERM

list_esp() {
    $SUDO lsblk -rno NAME,PARTTYPE 2>/dev/null \
        | awk -v g="$ESP_GUID" 'tolower($2)==g {print "/dev/"$1}'
    have blkid && $SUDO blkid -t TYPE=vfat -o device 2>/dev/null
}
mounted_point_for() { lsblk -rno MOUNTPOINT "$1" 2>/dev/null | grep -m1 -v '^$' || true; }
choose() {
    local prompt="$1"; shift
    if [ "$#" -eq 1 ]; then echo "$1"; return; fi
    echo "$prompt" >&2; local i=1; for o in "$@"; do echo "  $i) $o" >&2; i=$((i+1)); done
    local sel; read -rp "  elegí [1-$#]: " sel </dev/tty
    [ "$sel" -ge 1 ] 2>/dev/null && [ "$sel" -le "$#" ] || die "selección inválida"
    eval "echo \"\${$sel}\""
}
ESP_MNT=""; ESP_DEV=""
resolve_esp() {
    mapfile -t devs < <(list_esp | sort -u | grep -v '^$')
    [ "${#devs[@]}" -gt 0 ] || die "no encontré ninguna partición EFI System."
    ESP_DEV="$(choose "Más de una ESP — ¿cuál uso?" "${devs[@]}")"
    ESP_MNT="$(mounted_point_for "$ESP_DEV")"
    if [ -z "$ESP_MNT" ]; then
        TMP_MNT="$(mktemp -d /tmp/arje-esp.XXXXXX)"
        $SUDO mount "$ESP_DEV" "$TMP_MNT" || die "no pude montar $ESP_DEV"
        ESP_MNT="$TMP_MNT"; info "ESP $ESP_DEV montada en $ESP_MNT"
    fi
}

# Setea DISK/PART desde el device de la ESP, para efibootmgr.
DISK=""; PART=""
disk_and_part() {
    local dev="$1" disk part
    disk="$(lsblk -rno PKNAME "$dev" 2>/dev/null | head -1)"
    part="$(lsblk -rno PARTN "$dev" 2>/dev/null | head -1)"
    if [ -n "$disk" ] && [ -n "$part" ]; then DISK="/dev/$disk"; PART="$part"; return; fi
    if [[ "$dev" =~ ^(/dev/.*[0-9])p([0-9]+)$ ]]; then DISK="${BASH_REMATCH[1]}"; PART="${BASH_REMATCH[2]}";
    elif [[ "$dev" =~ ^(/dev/[a-z]+)([0-9]+)$ ]]; then DISK="${BASH_REMATCH[1]}"; PART="${BASH_REMATCH[2]}";
    else die "no pude derivar disco/partición de $dev"; fi
}

# ════════════════════════════════════════════════════════════════════════════
# Desinstalación
# ════════════════════════════════════════════════════════════════════════════
if [ "$DO_UNINSTALL" = 1 ]; then
    info "desinstalando arje-loader"
    for num in $($SUDO efibootmgr | sed -n "s/^Boot\([0-9A-Fa-f]\{4\}\)\*\? $LABEL\$/\1/p"); do
        info "borrando entrada NVRAM Boot$num"
        $SUDO efibootmgr -b "$num" -B >/dev/null
    done
    resolve_esp
    $SUDO rm -rf "$ESP_MNT/EFI/arje/loader.efi" "$ESP_MNT/arje" \
                 "$ESP_MNT/loader/entries/10-arje.conf" \
                 "$ESP_MNT/loader/entries/20-sistema.conf"
    echo "✓ arje-loader desinstalado. Tu GRUB y tu arranque de siempre, intactos."
    exit 0
fi

# ════════════════════════════════════════════════════════════════════════════
# Instalación
# ════════════════════════════════════════════════════════════════════════════
have cargo || die "falta cargo (https://rustup.rs)."
rustup target list --installed 2>/dev/null | grep -qx "$UEFI_TARGET" \
    || { info "agregando target $UEFI_TARGET"; rustup target add "$UEFI_TARGET"; }

# Kernel/initramfs/cmdline base, leídos de tu arranque actual.
KERNEL_SRC="$(awk '{for(i=1;i<=NF;i++) if($i ~ /^BOOT_IMAGE=/){sub(/^BOOT_IMAGE=/,"",$i); print $i; exit}}' /proc/cmdline)"
[ -n "$KERNEL_SRC" ] && [ -f "$KERNEL_SRC" ] || KERNEL_SRC="$(ls -1t /boot/vmlinuz* 2>/dev/null | head -1)"
[ -n "$KERNEL_SRC" ] && [ -f "$KERNEL_SRC" ] || die "no encontré el kernel (/boot/vmlinuz*)."
KID="$(basename "$KERNEL_SRC" | sed 's/^vmlinuz-//')"   # p.ej. linux-cachyos-rc
INITRD_SRC="$(ls -1 /boot/initramfs-"$KID".img /boot/initrd-"$KID".img /boot/initramfs-linux.img 2>/dev/null | head -1)"
[ -n "$INITRD_SRC" ] && [ -f "$INITRD_SRC" ] || die "no encontré el initramfs de $KID en /boot."
# Microcódigo (si está): se concatena antes del initramfs (orden importa).
UCODE=""; for u in /boot/intel-ucode.img /boot/amd-ucode.img; do [ -f "$u" ] && UCODE="$UCODE $u"; done
# root= y flags base (sin BOOT_IMAGE ni init=), de /proc/cmdline.
BASE_OPTS="$(sed -E 's/BOOT_IMAGE=[^ ]+//; s/\binit=[^ ]+//g' /proc/cmdline | tr -s ' ' | sed 's/^ //; s/ $//')"

[ -x "$ZERO" ] || echo "  ⚠ $ZERO no existe aún — la entry «arje» necesita arje-zero instalado (./scripts/install-tawasuyu.sh --with-init lo deja)."

info "compilando arje-loader ($UEFI_TARGET, release)"
( cd "$LOADER_DIR" && cargo build --release --target "$UEFI_TARGET" )
EFI_BIN="$LOADER_DIR/target/$UEFI_TARGET/release/arje-loader.efi"
[ -f "$EFI_BIN" ] || die "no se compiló $EFI_BIN"

resolve_esp
disk_and_part "$ESP_DEV"

cat <<RESUMEN

  ── arje-loader :: resumen ──────────────────────────────────────────────
   ESP        : $ESP_DEV  (en $ESP_MNT)   disco/part: $DISK / $PART
   loader     : $ESP_MNT/EFI/arje/loader.efi
   kernel     : $KERNEL_SRC  →  ESP:/arje/vmlinuz
   initramfs  : $INITRD_SRC ${UCODE:+(+ ucode)}  →  ESP:/arje/
   entry arje : init=$ZERO  $FLICKER
   entry sist.: (sin init=, tu arranque normal)
   NVRAM      : «$LABEL» (NO-default; tu GRUB sigue primero)
   reversible : ./scripts/install-arje-loader.sh --uninstall
  ─────────────────────────────────────────────────────────────────────────
RESUMEN
if [ "$ASSUME_YES" != 1 ]; then
    read -rp "¿Sigo? [s/N] " ok </dev/tty
    case "$ok" in s|S|y|Y) ;; *) die "cancelado." ;; esac
fi

# ── Copiar loader + kernel + initramfs (con ucode prepended) a la ESP ────────
info "copiando a la ESP"
$SUDO install -Dm644 "$EFI_BIN" "$ESP_MNT/EFI/arje/loader.efi"
$SUDO install -Dm644 "$KERNEL_SRC" "$ESP_MNT/arje/vmlinuz"
# initramfs final = ucode(s) + initramfs concatenados (el kernel los lee en orden).
TMP_INITRD="$(mktemp)"; cat $UCODE "$INITRD_SRC" > "$TMP_INITRD"
$SUDO install -Dm644 "$TMP_INITRD" "$ESP_MNT/arje/initramfs"; rm -f "$TMP_INITRD"

# ── Escribir las entries + loader.conf (paths relativos a la ESP) ────────────
info "escribiendo /loader/entries y loader.conf"
$SUDO install -d "$ESP_MNT/loader/entries"
$SUDO tee "$ESP_MNT/loader/entries/10-arje.conf" >/dev/null <<EOF
title arje — tawasuyu (init alterno)
linux /arje/vmlinuz
initrd /arje/initramfs
options $BASE_OPTS init=$ZERO $FLICKER
EOF
$SUDO tee "$ESP_MNT/loader/entries/20-sistema.conf" >/dev/null <<EOF
title $(. /etc/os-release 2>/dev/null; echo "${NAME:-tu sistema}") (arranque normal)
linux /arje/vmlinuz
initrd /arje/initramfs
options $BASE_OPTS
EOF
$SUDO tee "$ESP_MNT/loader/loader.conf" >/dev/null <<EOF
timeout 5
default 10-arje
EOF

# ── Registrar la entrada NVRAM (NO-default) ──────────────────────────────────
info "registrando la opción de arranque «$LABEL» (no-default)"
$SUDO efibootmgr -c -d "$DISK" -p "$PART" -L "$LABEL" -l '\EFI\arje\loader.efi' >/dev/null
NEW="$($SUDO efibootmgr | sed -n "s/^Boot\([0-9A-Fa-f]\{4\}\)\*\? $LABEL\$/\1/p" | head -1)"
OLD="$($SUDO efibootmgr | sed -n 's/^BootOrder: //p')"
# efibootmgr -c pone la nueva primera; la re-mandamos al final para NO ser default.
if [ -n "$NEW" ] && [ -n "$OLD" ]; then
    CLEAN="$(echo "$OLD" | sed "s/$NEW,\?//g; s/,$//")"
    $SUDO efibootmgr -o "${CLEAN:+$CLEAN,}$NEW" >/dev/null
fi

cat <<FIN

✓ arje-loader instalado como opción de arranque (no-default).

  Probarlo: reiniciá, entrá al menú de arranque del firmware (F12/F9/Esc según
  tu BIOS) y elegí «$LABEL». Vas a ver el menú gráfico; elegí «arje» o tu
  sistema normal. Tu GRUB sigue siendo el default — no cambió nada.

  Revertir:  ./scripts/install-arje-loader.sh --uninstall
FIN
