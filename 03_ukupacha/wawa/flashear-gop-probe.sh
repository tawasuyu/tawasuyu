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
#
#  Verifica cada paso y al final VUELCA el layout para confirmar que quedo bien
#  antes de rebootear (un flasheo a medias cuelga la barra del firmware).
# =============================================================================
set -euo pipefail

paso() { echo ">> $*"; }
fatal() { echo "!! ERROR: $*" >&2; exit 1; }

DEV="${1:-}"
[ -n "$DEV" ] || fatal "uso: sudo bash $0 /dev/sdX   (tu USB; hoy /dev/sdb)"
[ "$(id -u)" = "0" ] || fatal "corre con sudo: sudo bash $0 $DEV"
BASE="$(basename "$DEV")"

[ -b "$DEV" ] || fatal "$DEV no es un dispositivo de bloque"
[ "$(cat "/sys/block/$BASE/removable" 2>/dev/null || echo 0)" = "1" ] \
    || fatal "$DEV NO es removible — me niego a tocarlo (¿confundiste el disco?)"
if lsblk -nro MOUNTPOINT "$DEV" | grep -qE '^/($|boot|home|var|usr)'; then
    fatal "$DEV tiene particiones de SISTEMA montadas — abortando"
fi

EFI="$(cd "$(dirname "$0")" && pwd)/gop-probe.efi"
[ -f "$EFI" ] || fatal "no encuentro $EFI"

echo ">> Voy a BORRAR TODO en $DEV  ($(lsblk -dno SIZE,MODEL "$DEV"))"
echo ">> Ctrl-C en 5s para abortar."
sleep 5

paso "desmontando particiones del USB"
for p in $(lsblk -nro NAME "$DEV" | tail -n +2); do
    umount "/dev/$p" 2>/dev/null || true
done

paso "borrando firmas anteriores"
wipefs -a "$DEV" >/dev/null
sync

paso "creando GPT + una ESP FAT32 de 100 MiB"
# sfdisk re-lee la tabla solo y es determinista. Campo: start(default 2048),
# size=100M, type=U (EFI System). SIN el flag bootable(*): en GPT ese flag pone
# el atributo «Legacy BIOS Bootable», que hace que algunos firmwares intenten
# arrancar la ESP por CSM/legacy (no tiene boot sector) y se CUELGUEN. Para
# UEFI basta el tipo EFI System; el flag legacy estorba.
echo 'label: gpt
,100M,U' | sfdisk --quiet --wipe always "$DEV"
sync
partprobe "$DEV" 2>/dev/null || true
udevadm settle 2>/dev/null || true
sleep 2

PART="${DEV}1"
[ -b "${DEV}p1" ] && PART="${DEV}p1"
[ -b "$PART" ] || fatal "no aparecio la particion $PART tras particionar"

paso "formateando $PART como FAT32"
mkfs.fat -F32 -n GOPPROBE "$PART" >/dev/null

paso "copiando gop-probe.efi como \\EFI\\BOOT\\BOOTX64.EFI"
MNT="$(mktemp -d)"
mount "$PART" "$MNT"
mkdir -p "$MNT/EFI/BOOT"
cp "$EFI" "$MNT/EFI/BOOT/BOOTX64.EFI"
sync
# Verificar que el archivo quedo y con el tamaño correcto.
DEST="$MNT/EFI/BOOT/BOOTX64.EFI"
[ -f "$DEST" ] || { umount "$MNT"; rmdir "$MNT"; fatal "el .efi no se copio"; }
SRC_SZ=$(stat -c%s "$EFI"); DST_SZ=$(stat -c%s "$DEST")
[ "$SRC_SZ" = "$DST_SZ" ] || { umount "$MNT"; rmdir "$MNT"; fatal "tamaño no coincide ($SRC_SZ vs $DST_SZ)"; }
umount "$MNT"
rmdir "$MNT"
sync

echo
paso "LISTO y verificado. Layout final del USB:"
sfdisk -l "$DEV" 2>/dev/null || true
lsblk -f "$DEV"
echo
echo ">> Apaga, conecta los DOS monitores, enciende y elige el USB en el menu de"
echo ">> boot del firmware (F12/F8/Esc). Secure Boot debe estar OFF."
