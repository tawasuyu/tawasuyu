#!/usr/bin/env bash
# install-arje.sh — instalador automático de arje como opción de arranque UEFI.
#
# Un solo comando, sin parámetros. Detecta solo la ESP, el kernel y la GPU;
# compila lo necesario; copia arje bajo <ESP>/EFI/arje/ (al lado de tu GRUB, sin
# pisarlo) y registra una entrada NVRAM con efibootmgr. Pide sudo solo para los
# pasos privilegiados (montar/copiar/efibootmgr); la compilación corre como vos.
# Lo único que pregunta es lo ambiguo (varias ESP/kernels) y cómo activar la
# entrada. Todo es reversible:  ./scripts/install-arje.sh --uninstall
#
# QUÉ INSTALA HOY (honestidad — ver SDD-ARRANQUE-SIN-PARPADEO.md y el RUNBOOK):
# el arranque NATIVO de arje es por ahora un DEMO del boot-chain sin parpadeo:
# muestra el splash gráfico en la GPU real desde el frame cero y baja a una
# consola de PRUEBA DE VIDA en tty1 (arje-getty-stub: abre la TTY e imprime un
# banner, NO es un login). El greeter gráfico y un login real necesitan rootfs
# con Mesa / getty estático, todavía no listos para el arranque nativo.
#
#   install-arje.sh              # instala el demo de arranque (interactivo)
#   install-arje.sh --yes        # sin confirmaciones (asume defaults)
#   install-arje.sh --uninstall  # quita la entrada NVRAM y los archivos de la ESP
#   install-arje.sh --help
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

TARGET="x86_64-unknown-linux-musl"
SEED="03_ukupacha/arje/seeds/arje-laptop.card.json"
LABEL="arje"
ESP_GUID="c12a7328-f81f-11d2-ba4b-00a0c93ec93b"   # GUID de partición "EFI System"
ASSUME_YES=0
DO_UNINSTALL=0

die()  { echo "✗ $*" >&2; exit 1; }
info() { echo "==> $*"; }
have() { command -v "$1" >/dev/null 2>&1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --yes|-y)     ASSUME_YES=1 ;;
        --uninstall)  DO_UNINSTALL=1 ;;
        -h|--help)    sed -n '2,30p' "$0"; exit 0 ;;
        *) die "opción desconocida: $1 (corré sin argumentos, o --uninstall)" ;;
    esac
    shift
done

[ -d /sys/firmware/efi ] || die "esta máquina no arrancó en modo UEFI; arje sólo instala por UEFI (no GRUB/BIOS legacy)."

# ── sudo: lo pedimos una vez y queda cacheado para los pasos privilegiados ────
if [ "$(id -u)" != 0 ]; then
    have sudo || die "necesito sudo para montar la ESP y correr efibootmgr."
    info "voy a pedir sudo para los pasos privilegiados (montar ESP, copiar, efibootmgr)"
    sudo -v || die "sudo denegado."
    SUDO="sudo"
else
    SUDO=""
fi

# ── Descubrir la ESP ──────────────────────────────────────────────────────────
# Devuelve, por stdout, "MOUNTPOINT DEVICE" de la ESP elegida. Si no está
# montada, la monta en un tmpdir y deja apuntado TMP_MNT para desmontarla al salir.
TMP_MNT=""
cleanup() { [ -n "$TMP_MNT" ] && { $SUDO umount "$TMP_MNT" 2>/dev/null || true; rmdir "$TMP_MNT" 2>/dev/null || true; }; }
trap cleanup EXIT INT TERM

list_esp_devices() {  # imprime los /dev/... que son ESP (EFI System por GUID)
    # Primario: PARTTYPE == GUID de "EFI System" (necesita leer la tabla → sudo).
    $SUDO lsblk -rno NAME,PARTTYPE 2>/dev/null \
        | awk -v g="$ESP_GUID" 'tolower($2)==g {print "/dev/"$1}'
    # Fallback: particiones vfat con la flag esp/boot (algunas tablas no pueblan
    # PARTTYPE en lsblk). blkid + sgdisk/parted no siempre están; usamos blkid.
    if have blkid; then
        $SUDO blkid -t TYPE=vfat -o device 2>/dev/null
    fi
}

mounted_point_for() {  # $1=device → su mountpoint actual (vacío si no montada)
    lsblk -rno MOUNTPOINT "$1" 2>/dev/null | grep -m1 -v '^$' || true
}

choose() {  # menú interactivo simple; $1=prompt, resto=opciones. Ecoa la elegida.
    local prompt="$1"; shift
    if [ "$#" -eq 1 ]; then echo "$1"; return; fi
    echo "$prompt" >&2
    local i=1; for o in "$@"; do echo "  $i) $o" >&2; i=$((i+1)); done
    local sel; read -rp "  elegí [1-$#]: " sel </dev/tty
    [ "$sel" -ge 1 ] 2>/dev/null && [ "$sel" -le "$#" ] || die "selección inválida"
    eval "echo \"\${$sel}\""
}

# Setea las globales ESP_MNT y ESP_DEV (y TMP_MNT si hubo que montarla). NO usar
# en subshell: si montamos, TMP_MNT debe vivir en el shell padre para el cleanup.
ESP_MNT=""; ESP_DEV=""
resolve_esp() {
    mapfile -t devs < <(list_esp_devices | sort -u | grep -v '^$')
    [ "${#devs[@]}" -gt 0 ] || die "no encontré ninguna partición EFI System (GUID $ESP_GUID). ¿UEFI?"
    ESP_DEV="$(choose "Más de una ESP encontrada — ¿cuál uso?" "${devs[@]}")"
    ESP_MNT="$(mounted_point_for "$ESP_DEV")"
    if [ -z "$ESP_MNT" ]; then
        TMP_MNT="$(mktemp -d /tmp/arje-esp.XXXXXX)"
        $SUDO mount "$ESP_DEV" "$TMP_MNT" || die "no pude montar la ESP $ESP_DEV"
        ESP_MNT="$TMP_MNT"
        info "ESP $ESP_DEV montada temporalmente en $ESP_MNT"
    fi
}

# ── Descubrir el kernel ───────────────────────────────────────────────────────
resolve_kernel() {
    local cands=()
    for k in /boot/vmlinuz-linux /boot/vmlinuz-linux-lts /boot/vmlinuz-linux-zen /boot/vmlinuz; do
        [ -f "$k" ] && cands+=("$k")
    done
    if [ "${#cands[@]}" -eq 0 ]; then
        mapfile -t cands < <(ls -1t /boot/vmlinuz* 2>/dev/null)
    fi
    [ "${#cands[@]}" -gt 0 ] || die "no encontré ningún kernel en /boot/vmlinuz*"
    choose "Más de un kernel — ¿cuál booteo con arje?" "${cands[@]}"
}

# Setea DISK y PART (índice 1-based) desde el device de la ESP, para efibootmgr.
DISK=""; PART=""
disk_and_part() {  # $1=/dev/nvme0n1p2
    local dev="$1" disk part
    disk="$(lsblk -rno PKNAME "$dev" 2>/dev/null | head -1)"
    part="$(lsblk -rno PARTN "$dev" 2>/dev/null | head -1)"
    if [ -n "$disk" ] && [ -n "$part" ]; then
        DISK="/dev/$disk"; PART="$part"; return
    fi
    # Fallback por regex: nvme0n1p2 → nvme0n1 / 2 ; sda1 → sda / 1
    if [[ "$dev" =~ ^(/dev/.*[0-9])p([0-9]+)$ ]]; then
        DISK="${BASH_REMATCH[1]}"; PART="${BASH_REMATCH[2]}"
    elif [[ "$dev" =~ ^(/dev/[a-z]+)([0-9]+)$ ]]; then
        DISK="${BASH_REMATCH[1]}"; PART="${BASH_REMATCH[2]}"
    else
        die "no pude derivar disco/partición de $dev — registrá la entrada a mano con efibootmgr"
    fi
}

# ════════════════════════════════════════════════════════════════════════════
# Desinstalación
# ════════════════════════════════════════════════════════════════════════════
if [ "$DO_UNINSTALL" = 1 ]; then
    info "desinstalando arje"
    if have efibootmgr; then
        # Borra TODAS las entradas etiquetadas exactamente "arje".
        for num in $($SUDO efibootmgr | sed -n "s/^Boot\([0-9A-Fa-f]\{4\}\)\*\? $LABEL\$/\1/p"); do
            info "borrando entrada NVRAM Boot$num ($LABEL)"
            $SUDO efibootmgr -b "$num" -B >/dev/null
        done
    fi
    resolve_esp
    if [ -d "$ESP_MNT/EFI/arje" ]; then
        info "quitando $ESP_MNT/EFI/arje y loader/entries/arje.conf"
        $SUDO rm -rf "$ESP_MNT/EFI/arje" "$ESP_MNT/loader/entries/arje.conf"
    fi
    echo "✓ arje desinstalado. Tu GRUB y tu sistema quedan intactos."
    exit 0
fi

# ════════════════════════════════════════════════════════════════════════════
# Instalación
# ════════════════════════════════════════════════════════════════════════════
have cargo || die "falta cargo (instalá Rust: https://rustup.rs)"
have efibootmgr || die "falta efibootmgr (instalalo con el gestor de paquetes de tu distro) — lo necesito para crear la opción de arranque."
have musl-gcc || have x86_64-linux-musl-gcc || die "falta musl-gcc (instalá 'musl' / 'musl-tools') — el initramfs usa binarios estáticos."
if ! rustup target list --installed 2>/dev/null | grep -qx "$TARGET"; then
    info "agregando el target Rust $TARGET (rustup, por-usuario)"
    rustup target add "$TARGET" || die "no pude agregar $TARGET"
fi

# GPU (informativo): el cmdline canónico ya trae i915.fastboot=1 (inocuo en
# otras GPUs); amdgpu hace seamless por defecto; NVIDIA propietario no garantiza
# el handover sin parpadeo.
GPU="$(lspci -nn 2>/dev/null | grep -iE 'vga|3d|display' | head -1 || true)"
[ -n "$GPU" ] && info "GPU: ${GPU#*: }"
echo "$GPU" | grep -qi nvidia && echo "  ⚠ NVIDIA: el arranque sin parpadeo no está garantizado en el driver propietario."

resolve_esp
KERNEL="$(resolve_kernel)"
disk_and_part "$ESP_DEV"

# ── Resumen + confirmación ────────────────────────────────────────────────────
cat <<RESUMEN

  ── arje :: resumen de instalación ─────────────────────────────
   ESP        : $ESP_DEV  (montada en $ESP_MNT)
   disco/part : $DISK  partición $PART
   kernel     : $KERNEL
   seed       : $SEED  (splash + consola de prueba en tty1)
   destino    : $ESP_MNT/EFI/arje/   (NO toca tu GRUB)
   entrada    : NVRAM "$LABEL" vía efibootmgr
  ───────────────────────────────────────────────────────────────
RESUMEN
if [ "$ASSUME_YES" != 1 ]; then
    read -rp "¿Sigo? [s/N] " ok </dev/tty
    case "$ok" in s|S|y|Y) ;; *) die "cancelado." ;; esac
fi

# ── Compilar los binarios estáticos del initramfs ────────────────────────────
info "compilando binarios estáticos (musl): arje-zero, arje-splash, arje-getty-stub, arje-installer"
cargo build --release --target "$TARGET" -p arje-zero -p arje-splash -p arje-getty-stub
cargo build --release -p arje-installer
M="target/$TARGET/release"
INSTALLER="target/release/arje-installer"

# Config mínima del splash (panel de logs automático). En producción la edita
# wawa-panel; acá la horneamos para que el demo traiga la config por defecto.
TMP_CONF="$(mktemp)"; printf 'source = builtin\nlogs = auto\n' > "$TMP_CONF"

# ── Copiar a la ESP + registrar la entrada NVRAM ─────────────────────────────
info "instalando arje en la ESP y registrando la entrada de arranque"
$SUDO "$INSTALLER" to-partition \
    --esp "$ESP_MNT" \
    --kernel "$KERNEL" \
    --seed "$SEED" \
    --bin arje-zero="$M/arje-zero" \
    --bin arje-splash="$M/arje-splash" \
    --bin console-tty1="$M/arje-getty-stub" \
    --asset "etc/arje/splash.conf=$TMP_CONF" \
    --label "$LABEL" \
    --register --disk "$DISK" --part "$PART"
rm -f "$TMP_CONF"

# ── Activación: qué hacemos con el orden de arranque ─────────────────────────
NEWNUM="$($SUDO efibootmgr | sed -n "s/^Boot\([0-9A-Fa-f]\{4\}\)\*\? $LABEL\$/\1/p" | head -1)"
OLDORDER="$($SUDO efibootmgr | sed -n 's/^BootOrder: //p')"
if [ -n "$NEWNUM" ]; then
    ACT="1"
    if [ "$ASSUME_YES" != 1 ]; then
        echo
        echo "¿Cómo activo arje?"
        echo "  1) Sólo dejar la entrada (booteás arje desde el menú UEFI cuando quieras)  [default]"
        echo "  2) Probar UNA vez en el próximo reinicio (BootNext; vuelve solo a lo de siempre)"
        echo "  3) Hacer arje el arranque por DEFECTO"
        read -rp "  elegí [1-3]: " ACT </dev/tty
    fi
    case "${ACT:-1}" in
        2) $SUDO efibootmgr -n "$NEWNUM" >/dev/null
           echo "✓ arje arrancará en el PRÓXIMO reinicio (sólo esa vez)." ;;
        3) echo "✓ arje quedó como arranque por defecto (efibootmgr ya lo puso primero)." ;;
        *) # Dejar la entrada pero NO como default: re-poné tu orden previo + arje al final.
           if [ -n "$OLDORDER" ]; then
               CLEAN="$(echo "$OLDORDER" | sed "s/$NEWNUM,\?//g; s/,$//")"
               $SUDO efibootmgr -o "${CLEAN:+$CLEAN,}$NEWNUM" >/dev/null
           fi
           echo "✓ entrada arje creada (no es el default; tu arranque de siempre no cambió)." ;;
    esac
else
    echo "⚠ no pude leer el número de la entrada NVRAM; revisá con: efibootmgr -v | grep $LABEL"
fi

cat <<FIN

✓ Listo. arje quedó instalado como opción de arranque, sin tocar tu GRUB.

  Qué vas a ver al bootear arje: el splash gráfico sin parpadeo en tu pantalla,
  y luego una consola de prueba de vida en tty1 (es un DEMO del boot-chain, no
  un login/escritorio todavía — ver SDD-ARRANQUE-SIN-PARPADEO.md).

  Revertir todo:   ./scripts/install-arje.sh --uninstall
FIN
