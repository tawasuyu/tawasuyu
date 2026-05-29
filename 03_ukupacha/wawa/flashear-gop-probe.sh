#!/usr/bin/env bash
# =============================================================================
#  flashear-gop-probe.sh — deja gop-probe.efi arrancable en un USB UEFI
# -----------------------------------------------------------------------------
#  Crea GPT + una ESP FAT32 con \EFI\BOOT\BOOTX64.EFI = gop-probe.efi, de modo
#  que el firmware lo arranque solo al elegir el USB en el menu de boot.
#
#  USO:   sudo bash flashear-gop-probe.sh /dev/sdX
#         (sdX = tu USB; hoy es /dev/sdb segun lsblk)
#
#  SEGURIDAD: aborta si el dispositivo NO es removible o si tiene particiones de
#  sistema montadas. Aun asi, BORRA TODO el USB indicado — revisa el nombre.
# =============================================================================
set -euo pipefail

DEV="${1:-}"
if [ -z "$DEV" ]; then
    echo "uso: sudo bash $0 /dev/sdX   (tu USB; hoy /dev/sdb)"
    exit 1
fi
BASE="$(basename "$DEV")"

[ -b "$DEV" ] || { echo "ERROR: $DEV no es un dispositivo de bloque"; exit 1; }
if [ "$(cat "/sys/block/$BASE/removable" 2>/dev/null || echo 0)" != "1" ]; then
    echo "ERROR: $DEV NO es removible — me niego a tocarlo (¿confundiste el disco?)"
    exit 1
fi
if lsblk -nro MOUNTPOINT "$DEV" | grep -qE '^/($|boot|home|var|usr)'; then
    echo "ERROR: $DEV tiene particiones de SISTEMA montadas — abortando"
    exit 1
fi

EFI="$(cd "$(dirname "$0")" && pwd)/gop-probe.efi"
[ -f "$EFI" ] || { echo "ERROR: no encuentro $EFI"; exit 1; }

echo ">> Voy a BORRAR TODO en $DEV  ($(lsblk -dno SIZE,MODEL "$DEV"))"
echo ">> Ctrl-C en los proximos 5s para abortar."
sleep 5

# Desmontar cualquier particion del USB.
for p in $(lsblk -nro NAME "$DEV" | tail -n +2); do
    umount "/dev/$p" 2>/dev/null || true
done

wipefs -a "$DEV"
parted -s "$DEV" mklabel gpt
parted -s "$DEV" mkpart ESP fat32 1MiB 100MiB
parted -s "$DEV" set 1 esp on

# Esperar a que aparezca el nodo de la particion.
udevadm settle 2>/dev/null || true
sleep 1
PART="${DEV}1"
[ -b "${DEV}p1" ] && PART="${DEV}p1"
[ -b "$PART" ] || { echo "ERROR: no aparecio la particion $PART"; exit 1; }

mkfs.fat -F32 "$PART"
MNT="$(mktemp -d)"
mount "$PART" "$MNT"
mkdir -p "$MNT/EFI/BOOT"
cp "$EFI" "$MNT/EFI/BOOT/BOOTX64.EFI"
sync
umount "$MNT"
rmdir "$MNT"

echo ">> LISTO. $DEV arranca gop-probe."
echo ">> Apaga, conecta los DOS monitores, enciende y elige el USB en el menu de boot del firmware (F12/F8/Esc segun la maquina). Secure Boot debe estar OFF."
