#!/usr/bin/env bash
# install-arje-init.sh — instala arje-zero como INIT ALTERNO (PID 1) sobre tu
# Linux ACTUAL, en una entrada de arranque APARTE que NO toca tu default. Al
# elegir "arje" en el menú de GRUB, el MISMO kernel monta tu MISMO root, pero
# arranca `init=/usr/local/sbin/arje-zero` en vez de systemd: arje encarna el
# fractal de la seed `arje-tawasuyu-host` → net-up → splash (mirada temprana,
# sin parpadeo) → mirada-compositor --drm --greeter (con la Mesa de tu sistema)
# → tras el login, la sesión mirada/pata/shuma como tu usuario.
#
# Tu systemd/KDE quedan intactos en la entrada default; "arje" es opcional.
#
# Requisitos previos: la capa de escritorio ya instalada (mirada-compositor en
# /usr/local/bin) — esto lo deja `install-mirada-dm.sh` / `install-tawasuyu.sh`.
#
# Uso:
#   install-arje-init.sh             # instala (interactivo: confirma antes de GRUB)
#   install-arje-init.sh --yes       # sin preguntas
#   install-arje-init.sh --uninstall # quita binarios, seed y la entrada de GRUB
#   install-arje-init.sh --help
#
# HONESTIDAD: el reboot a arje es prueba de METAL — nunca se booteó arje de PID 1
# sobre una distro systemd. Si algo falla, REINICIÁ y elegí tu entrada de siempre
# (no se tocó). Rescate dentro de arje: Ctrl+Alt+F2 (getty). Revertí todo con
# --uninstall.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

SEED_SRC="03_ukupacha/arje/seeds/arje-tawasuyu-host.card.json"
SEED_DST="/ente/seed.card.json"
PREFIX="/usr/local"
LIBDIR="$PREFIX/lib/arje"
ZERO_DST="$PREFIX/sbin/arje-zero"
GRUB_SNIPPET="/etc/grub.d/41_arje"
GRUB_CFG="/boot/grub/grub.cfg"
GRUB_TITLE="arje — tawasuyu (init alterno · no-default)"
# Flags de arranque sin parpadeo (SDD-ARRANQUE-SIN-PARPADEO §Fase 0) + init.
FLICKER_FLAGS="quiet loglevel=0 vt.global_cursor_default=0 rd.systemd.show_status=false i915.fastboot=1"
INIT_FLAG="init=$ZERO_DST"

ASSUME_YES=0
DO_UNINSTALL=0
WITH_SERVICES=0

die()  { echo "✗ $*" >&2; exit 1; }
info() { echo "==> $*"; }
have() { command -v "$1" >/dev/null 2>&1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --yes|-y)        ASSUME_YES=1 ;;
        --uninstall)     DO_UNINSTALL=1 ;;
        --with-services) WITH_SERVICES=1 ;;
        -h|--help)       sed -n '2,24p' "$0"; exit 0 ;;
        *) die "opción desconocida: $1 (ver --help)" ;;
    esac
    shift
done

[ "$(id -u)" != 0 ] || die "no me corras con sudo: compilo con tu toolchain y pido sudo solo para los pasos de sistema."
have sudo || die "necesito sudo para escribir en /usr/local, /ente y /etc/grub.d."

# ════════════════════════════════════════════════════════════════════════════
# Desinstalación
# ════════════════════════════════════════════════════════════════════════════
if [ "$DO_UNINSTALL" = 1 ]; then
    info "quitando arje como init alterno (sudo)"
    sudo -v || die "sudo denegado."
    sudo rm -f "$ZERO_DST" "$LIBDIR/arje-splash" "$LIBDIR/net-bring-up" "$SEED_DST"
    sudo rmdir /ente 2>/dev/null || true
    if [ -f "$GRUB_SNIPPET" ]; then
        sudo rm -f "$GRUB_SNIPPET"
        if have grub-mkconfig; then
            info "regenerando $GRUB_CFG (sin la entrada arje)"
            sudo grub-mkconfig -o "$GRUB_CFG" >/dev/null 2>&1 || echo "  ⚠ regenerá vos: sudo grub-mkconfig -o $GRUB_CFG"
        fi
    fi
    echo "✓ arje-init quitado. Tu entrada de arranque default no se tocó. (No borré"
    echo "  /etc/arje ni tu ~/.config/mirada/autostart — son datos compartidos/tuyos.)"
    exit 0
fi

# ════════════════════════════════════════════════════════════════════════════
# Instalación
# ════════════════════════════════════════════════════════════════════════════
have cargo || die "falta cargo (instalá Rust: https://rustup.rs)."
have grub-mkconfig || die "no encuentro grub-mkconfig — esta máquina no usa GRUB; pasá la entrada a mano (ver --help)."
[ -f "$GRUB_CFG" ] || die "no existe $GRUB_CFG."
[ -x "$PREFIX/bin/mirada-compositor" ] || die "falta $PREFIX/bin/mirada-compositor — corré primero la capa de escritorio (./scripts/install-tawasuyu.sh o install-mirada-dm.sh)."
[ -f "$SEED_SRC" ] || die "no encuentro la seed $SEED_SRC."

info "GPU: $(lspci -nn 2>/dev/null | grep -iE 'vga|3d|display' | head -1 | sed 's/.*: //' || echo '¿?')"
echo "$(lspci 2>/dev/null)" | grep -qi nvidia && \
    echo "  ⚠ NVIDIA: el handoff sin parpadeo del splash no está garantizado en el driver propietario."

# ── Construir los binarios de init (glibc del root real, no musl: arje corre
#    sobre tu rootfs que ya tiene libc) ────────────────────────────────────────
info "compilando arje-zero, arje-splash, arje-absorb (release)"
cargo build --release -p arje-zero -p arje-splash -p arje-absorb
M="target/release"
for b in arje-zero arje-splash arje-absorb; do
    [ -x "$M/$b" ] || die "no se compiló $M/$b"
done

# ── Resumen + confirmación (antes de tocar GRUB) ─────────────────────────────
ROOT_SRC="$(awk '{for(i=1;i<=NF;i++) if($i ~ /^root=/){print $i; exit}}' /proc/cmdline)"
KERN_SRC="$(awk '{for(i=1;i<=NF;i++) if($i ~ /^BOOT_IMAGE=/){sub(/^BOOT_IMAGE=/,"",$i); print $i; exit}}' /proc/cmdline)"
cat <<RESUMEN

  ── arje como init alterno — resumen ────────────────────────────────────
   seed       : generada del host (arje-absorb)  →  $SEED_DST
   binarios   : $ZERO_DST
                $LIBDIR/{arje-splash,net-bring-up}
   DM         : $PREFIX/bin/mirada-compositor --drm --greeter (seat builtin)
   entrada    : GRUB "$GRUB_TITLE"
                kernel ${KERN_SRC:-<actual>}  ${ROOT_SRC:-root=<actual>}  $INIT_FLAG
                (clona tu entrada default; NO la reemplaza, NO cambia el default)
   reversible : ./scripts/install-arje-init.sh --uninstall
  ─────────────────────────────────────────────────────────────────────────
  El reboot a "arje" es prueba de metal tuya. Si falla, reiniciá y elegí tu
  entrada de siempre (intacta); rescate en arje: Ctrl+Alt+F2.
RESUMEN
if [ "$ASSUME_YES" != 1 ]; then
    read -rp "¿Instalo y agrego la entrada de GRUB? [s/N] " ok </dev/tty
    case "$ok" in s|S|y|Y) ;; *) die "cancelado (no se tocó nada)." ;; esac
fi

# ── Instalar binarios + seed ─────────────────────────────────────────────────
info "instalando binarios de init en $PREFIX (sudo)"
sudo install -Dm755 "$M/arje-zero"   "$ZERO_DST"
sudo install -Dm755 "$M/arje-splash" "$LIBDIR/arje-splash"

# Config del splash (logo respirando + panel de logs automático) si no existe.
if [ ! -e /etc/arje/splash.conf ]; then
    sudo install -d /etc/arje
    printf 'source = builtin\nlogs = auto\n' | sudo tee /etc/arje/splash.conf >/dev/null
fi

# Backup de cualquier seed previa ANTES de pisarla (sólo la primera vez).
if [ -f "$SEED_DST" ] && [ ! -f "$SEED_DST.pre-arje-init.bak" ]; then
    sudo cp -a "$SEED_DST" "$SEED_DST.pre-arje-init.bak"
    info "backup de la seed previa en $SEED_DST.pre-arje-init.bak"
fi
# La seed se DERIVA del host, no se hardcodea: arje-absorb detecta tu init
# (openrc/runit/dinit/sysvinit) + sus servicios, y suma el overlay gráfico
# tawasuyu con paths DETECTADOS (udev/seatd/red/splash/compositor/getty). Así el
# MISMO instalador sirve en cualquier distro libre, no sólo en esta máquina.
sudo install -d "$(dirname "$SEED_DST")"
# Por defecto sólo el escritorio (--overlay-only); con --with-services además
# absorbe tus servicios HABILITADOS del init (caddy, sshd, scripts…) — los que
# declaran el daemon real corren supervisados, el resto por el wrapper init.d.
# Los DESACTIVADOS y los DMs ajenos (sddm/ly) NO entran.
if [ "$WITH_SERVICES" = 1 ]; then
    ABSORB=(--from auto --with-carmen)
    info "generando la seed: tus servicios habilitados + escritorio (arje-absorb --from auto --with-carmen)"
else
    ABSORB=(--overlay-only)
    info "generando la seed: sólo el escritorio detectado (arje-absorb --overlay-only; sumá --with-services para tus servicios)"
fi
if sudo "$M/arje-absorb" "${ABSORB[@]}" --label arje.seed.host --output "$SEED_DST"; then
    info "seed derivada de tu host en $SEED_DST"
else
    echo "  ⚠ arje-absorb no autodetectó tu init; caigo a la seed de referencia."
    sudo install -Dm644 "$SEED_SRC" "$SEED_DST"
fi
# Chequeo mínimo: el compositor y arje-zero existen (el resto la seed los detectó).
for exe in "$ZERO_DST" "$LIBDIR/arje-splash" "$PREFIX/bin/mirada-compositor"; do
    [ -x "$exe" ] || echo "  ⚠ falta $exe en disco — revisá."
done

# ── Sembrar el ecosistema en el autostart de la sesión (tu usuario) ──────────
# La sesión "mirada" (exec vacío) corre ~/.config/mirada/autostart como tu
# usuario tras el login del greeter. Sembramos pata + notificaciones (idempotente,
# sin pisar lo que ya tengas) para que el escritorio venga entero. NO sembramos
# shuma: pata ya abre el shell embebido; una instancia standalone de
# shuma-shell-llimphi al arranque es una ventana suelta redundante (lección metal
# 2026-06-30). Quien lo quiera, lo agrega a mano.
AUTO="${HOME}/.config/mirada/autostart"
mkdir -p "$(dirname "$AUTO")"
# Audio: pipewire es un daemon de USUARIO (el init no lo arranca). Lo sembramos
# en el autostart de la sesión —si está instalado— para tener sonido. wireplumber
# es su session-manager; pipewire-pulse da compat PulseAudio. Van primero.
for a in pipewire wireplumber pipewire-pulse; do
    have "$a" && { grep -qxF "$a" "$AUTO" 2>/dev/null || echo "$a" >> "$AUTO"; }
done
for line in pata-llimphi pata-notify; do
    grep -qxF "$line" "$AUTO" 2>/dev/null || echo "$line" >> "$AUTO"
done
info "ecosistema sembrado en $AUTO (pata-llimphi · pata-notify)"

# ── Entrada de GRUB: clona tu menuentry default y le cambia el init ──────────
# Tomamos el cuerpo del PRIMER menuentry de nivel superior de tu grub.cfg (tu
# arranque default: search/insmod/linux/initrd ya correctos para esta máquina) y
# lo reescribimos con init=arje-zero + flags sin-parpadeo. Quitamos `quiet`
# duplicado y los `echo` cosméticos. Si no encontramos un kernel ahí, abortamos
# (mejor que adivinar el arranque).
info "clonando tu entrada de arranque default desde $GRUB_CFG"
BODY="$(sudo awk '
    !seen && /^[[:space:]]*menuentry .*\{/ { seen=1; next }   # entra al 1er menuentry
    seen && /^[[:space:]]*}[[:space:]]*$/  { exit }           # cierra en su } (con o sin sangría)
    seen { print }
' "$GRUB_CFG")"
echo "$BODY" | grep -qE '^[[:space:]]*linux.*vmlinuz' || \
    die "no pude leer un menuentry con kernel en $GRUB_CFG — agregá la entrada a mano (init=$ZERO_DST + $FLICKER_FLAGS)."

NEW_BODY="$(printf '%s\n' "$BODY" | awk -v add="$INIT_FLAG $FLICKER_FLAGS" '
    /^[[:space:]]*echo/  { next }                         # descarta echos cosméticos
    /^[[:space:]]*linux/ { gsub(/ quiet/,""); print $0 " " add; next }
    { print }
')"

info "escribiendo $GRUB_SNIPPET (entrada extra, no-default)"
TMP_SNIP="$(mktemp)"
{
    echo '#!/bin/sh'
    echo '# Generado por scripts/install-arje-init.sh — entrada arje como init alterno.'
    echo '# Quitalo con: ./scripts/install-arje-init.sh --uninstall'
    echo "exec cat <<'ARJEGRUBEOF'"
    echo "menuentry '$GRUB_TITLE' {"
    printf '%s\n' "$NEW_BODY"
    echo "}"
    echo "ARJEGRUBEOF"
} > "$TMP_SNIP"
sudo install -Dm755 "$TMP_SNIP" "$GRUB_SNIPPET"
rm -f "$TMP_SNIP"

info "regenerando $GRUB_CFG (agrega la entrada arje; tu default no cambia)"
sudo grub-mkconfig -o "$GRUB_CFG" >/dev/null 2>&1 || \
    die "grub-mkconfig falló — quitá $GRUB_SNIPPET y regenerá; tu arranque previo sigue intacto."
sudo grep -q "$GRUB_TITLE" "$GRUB_CFG" && info "entrada '$GRUB_TITLE' presente en el menú." || \
    echo "  ⚠ no veo la entrada en $GRUB_CFG — revisá el snippet."

cat <<FIN

✓ arje quedó instalado como init alterno (entrada de GRUB aparte).

  Probarlo (metal, prueba tuya):
    1. Reiniciá y, en el menú de GRUB, elegí «$GRUB_TITLE».
    2. Vas a ver: splash sin parpadeo → greeter de mirada → login (tu usuario PAM)
       → escritorio mirada/pata/shuma. Seat por libseat builtin (arje es PID 1).
    3. Rescate si el gráfico falla: Ctrl+Alt+F2 (getty de tty2).
    4. Volver a lo de siempre: reiniciá y elegí tu entrada default (intacta).

  Notas:
    · La entrada clona el kernel actual ($([ -n "$KERN_SRC" ] && basename "$KERN_SRC" || echo '?')). Si cambiás/borrás ese
      kernel, re-corré este script para reapuntarla.
    · Revertir todo:  ./scripts/install-arje-init.sh --uninstall
FIN
