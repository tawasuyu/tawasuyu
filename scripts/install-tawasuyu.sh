#!/usr/bin/env bash
# install-tawasuyu.sh — instalador único de la CAPA DE SISTEMA tawasuyu en este
# Linux: te deja "vivir adentro" usando mirada como escritorio/DM, sin tocar tu
# distro ni tu bootloader. NO instala apps de dominio sueltas como producto: las
# que existan en target/release las cablea para que los lanzadores funcionen.
#
# No reinventa nada: ORQUESTA los install-*.sh que ya viven en scripts/ y, para
# lo que esos no cubren (desinstalación de mirada-dm), revierte por lista.
#
#   ┌─ etapas ────────────────────────────────────────────────────────────────┐
#   │ desktop  (def) install-mirada-dm.sh — compositor + greeter + pata +      │
#   │                shuma + lanzadores + notificaciones + portal + wallpaper. │
#   │                Es lo que arrancás con `sudo mirada-dm` y donde vivís.     │
#   │ splash   (def) install-arje-splash.sh --system — binario + config del    │
#   │                splash sin parpadeo. NO habilita el servicio (--enable-    │
#   │                splash lo activa).                                         │
#   │ compat   (opt) install-arje-session-gnome.sh --system — shims D-Bus de   │
#   │                arje-compat (logind/hostnamed/…) para correr la sesión     │
#   │                «GNOME» bajo arje. Sólo si --with-compat.                  │
#   │ init     (opt) install-arje-init.sh — arje-zero como INIT ALTERNO (PID 1) │
#   │                sobre TU root actual, en una entrada de GRUB aparte (NO     │
#   │                toca tu default). "arje" arranca splash → mirada-compositor │
#   │                --drm --greeter → sesión mirada/pata/shuma, con la Mesa que │
#   │                ya tenés. El reboot es prueba de metal tuya. --with-init.   │
#   └──────────────────────────────────────────────────────────────────────────┘
#
# Uso:
#   install-tawasuyu.sh                 # desktop + splash (binario, sin habilitar)
#   install-tawasuyu.sh --with-compat   # + shims arje-compat (sesión GNOME)
#   install-tawasuyu.sh --with-init     # + arje como init alterno (entrada GRUB)
#   install-tawasuyu.sh --enable-splash # habilita el servicio de splash del host
#   install-tawasuyu.sh --all           # desktop + splash(habilitado) + compat + init
#   install-tawasuyu.sh --yes           # sin preguntas (asume defaults)
#   install-tawasuyu.sh --uninstall     # revierte TODO lo que instaló este script
#   install-tawasuyu.sh --help
#
# Con --with-init vivís EN arje sobre tu propio root (arje de PID 1, tu kernel,
# tu Mesa) — sin rootfs aparte. El SO tawasuyu sobre DISCO PROPIO (rootfs hammer
# autocontenido) es otro camino, en ../hammer (hammer-install.sh /dev/sdX); no lo
# necesitás para vivir en arje acá.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
SCRIPTS="scripts"

# ── banderas ──────────────────────────────────────────────────────────────────
WITH_COMPAT=0
WITH_INIT=0
ENABLE_SPLASH=0
ASSUME_YES=0
DO_UNINSTALL=0

die()  { echo "✗ $*" >&2; exit 1; }
info() { echo "==> $*"; }
have() { command -v "$1" >/dev/null 2>&1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --with-compat)   WITH_COMPAT=1 ;;
        --with-init)     WITH_INIT=1 ;;
        --enable-splash) ENABLE_SPLASH=1 ;;
        --all)           WITH_COMPAT=1; WITH_INIT=1; ENABLE_SPLASH=1 ;;
        --yes|-y)        ASSUME_YES=1 ;;
        --uninstall)     DO_UNINSTALL=1 ;;
        -h|--help)       sed -n '2,41p' "$0"; exit 0 ;;
        *) die "opción desconocida: $1 (ver --help)" ;;
    esac
    shift
done

# ── inventario de lo que instala la etapa desktop (install-mirada-dm.sh) ───────
# Se replica acá SÓLO para poder revertir: install-mirada-dm.sh no trae
# --uninstall. Mantener en sincronía si esa lista cambia.
MIRADA_BINS=(
    mirada-compositor mirada-greeter pata-llimphi shuma-shell-llimphi
    mirada-launcher mirada-llimphi mirada-ctl mirada-portal mirada-wallpaper
    wawa-panel pacha agora-cli sandokan voz-daemon verbo-daemon
    pata-notify pata-notify-panel pata-notify-triage
    mirada-plugin-host mirada-plugin-sign mirada-session mirada-session-pata
    mirada-session-plugins mirada-dm mirada-supervise
)
SUITE_APPS=(
    nada pluma-editor-llimphi pluma-notebook-llimphi tullpu-app-llimphi
    takiy-app-llimphi media-app cosmos-app-llimphi dominium-app-llimphi
    tinkuy-llimphi chaka-app-llimphi nakui-sheet-llimphi puriy raymi-app
    supay-app-llimphi sandokan-monitor nahual-shell-llimphi
)
WAYLAND_SESSIONS=(mirada mirada-pata mirada-plugins)
COMPAT_BINS=(
    arje-logind-compat arje-hostnamed-compat arje-timedated-compat
    arje-localed-compat arje-polkit-compat arje-systemd1-compat
    arje-journald-compat arje-resolved-compat arje-machined-compat
    arje-policy-provider arje-notify-compat arje-timer-compat arje-activate
)
COMPAT_DBUS_NAMES=(
    org.freedesktop.login1 org.freedesktop.hostname1 org.freedesktop.timedate1
    org.freedesktop.locale1 org.freedesktop.PolicyKit1 org.freedesktop.systemd1
    org.freedesktop.resolve1 org.freedesktop.machine1
)

# ════════════════════════════════════════════════════════════════════════════
# Desinstalación — revierte cada etapa (sub-script --uninstall donde existe,
# borrado por lista donde no). NO toca tu ~/.config (datos tuyos).
# ════════════════════════════════════════════════════════════════════════════
if [ "$DO_UNINSTALL" = 1 ]; then
    have sudo || die "necesito sudo para quitar los archivos de sistema."
    info "desinstalando la capa de sistema tawasuyu (sudo para los archivos de sistema)"
    sudo -v || die "sudo denegado."

    # init: arje como init alterno (binarios + seed + entrada de GRUB).
    info "init: quitando arje como init alterno (si está)"
    "$SCRIPTS/install-arje-init.sh" --uninstall || echo "  (install-arje-init.sh --uninstall no aplicó; sigo)"
    # demo viejo (getty-stub vía install-arje.sh / entrada NVRAM), por si existía.
    "$SCRIPTS/install-arje.sh" --uninstall 2>/dev/null || true

    # splash: servicio + binario + config.
    info "splash: deshabilitando y quitando el servicio del host"
    if have systemctl; then
        sudo systemctl disable --now arje-splash.service 2>/dev/null || true
    fi
    sudo rm -f /etc/systemd/system/arje-splash.service \
               /etc/init.d/arje-splash 2>/dev/null || true
    sudo rm -rf /etc/sv/arje-splash 2>/dev/null || true
    have systemctl && sudo systemctl daemon-reload 2>/dev/null || true
    sudo rm -f /usr/local/lib/arje/arje-splash \
               /etc/arje/splash.conf /etc/arje/splash.png 2>/dev/null || true
    sudo rm -rf /etc/arje/frames 2>/dev/null || true

    # compat: shims + bundle + cards + .service de activación + marcador.
    info "compat: quitando los shims arje-compat"
    for b in "${COMPAT_BINS[@]}"; do sudo rm -f "/usr/local/lib/arje/$b"; done
    for n in "${COMPAT_DBUS_NAMES[@]}"; do
        sudo rm -f "/usr/share/dbus-1/system-services/$n.service"
    done
    sudo rm -f /etc/arje/cards.d/session-gnome.json \
               /etc/arje/cards.d/compat-*.json \
               /etc/arje/cards.d/policy-provider.json \
               /etc/arje/session-gnome.lazy 2>/dev/null || true
    # Limpiar /usr/local/lib/arje y /etc/arje/cards.d si quedaron vacíos.
    sudo rmdir /etc/arje/cards.d /usr/local/lib/arje /etc/arje 2>/dev/null || true

    # desktop: binarios + apps + pam + sesiones wayland.
    info "desktop: quitando binarios de mirada, apps cableadas, PAM y sesiones"
    for b in "${MIRADA_BINS[@]}" "${SUITE_APPS[@]}"; do
        sudo rm -f "/usr/local/bin/$b"
    done
    sudo rm -f /etc/pam.d/mirada
    for s in "${WAYLAND_SESSIONS[@]}"; do
        sudo rm -f "/usr/share/wayland-sessions/$s.desktop"
    done

    echo
    echo "✓ capa de sistema tawasuyu desinstalada. Tu distro, tu kernel y tu"
    echo "  bootloader quedan intactos. NO toqué tu ~/.config/mirada ni los grupos"
    echo "  (seat/video/input); borralos a mano si querés un reset total."
    exit 0
fi

# ════════════════════════════════════════════════════════════════════════════
# Instalación
# ════════════════════════════════════════════════════════════════════════════
[ "$(id -u)" != 0 ] || die "no me corras con sudo: los sub-scripts construyen con tu toolchain y piden sudo solos."
have cargo || die "falta cargo (instalá Rust: https://rustup.rs)."
for s in install-mirada-dm.sh install-arje-splash.sh install-arje-session-gnome.sh install-arje-init.sh; do
    [ -x "$SCRIPTS/$s" ] || die "no encuentro $SCRIPTS/$s (¿estás en el repo correcto?)."
done

# ── resumen + confirmación ────────────────────────────────────────────────────
cat <<RESUMEN

  ── tawasuyu :: capa de sistema sobre este Linux ───────────────────────
   desktop : mirada como escritorio/DM (sudo mirada-dm)             [SÍ]
   splash  : binario + config del arranque sin parpadeo            [$([ "$ENABLE_SPLASH" = 1 ] && echo 'SÍ + servicio' || echo 'SÍ, sin habilitar')]
   compat  : shims arje-compat (sesión GNOME bajo arje)            [$([ "$WITH_COMPAT" = 1 ] && echo SÍ || echo no)]
   init    : arje como init alterno (entrada GRUB → splash/mirada) [$([ "$WITH_INIT" = 1 ] && echo SÍ || echo no)]

   No toca tu distro, tu kernel ni tu bootloader. Reversible:
       ./scripts/install-tawasuyu.sh --uninstall
  ───────────────────────────────────────────────────────────────────────
RESUMEN
if [ "$ASSUME_YES" != 1 ]; then
    read -rp "¿Sigo? [s/N] " ok </dev/tty
    case "$ok" in s|S|y|Y) ;; *) die "cancelado." ;; esac
fi

# ── etapa desktop (siempre) ───────────────────────────────────────────────────
info "desktop — install-mirada-dm.sh (construye con tu toolchain, pide sudo para instalar)"
"$SCRIPTS/install-mirada-dm.sh"

# ── etapa splash (siempre; servicio sólo con --enable-splash) ─────────────────
info "splash — install-arje-splash.sh --system"
if [ "$ENABLE_SPLASH" = 1 ]; then
    "$SCRIPTS/install-arje-splash.sh" --system --enable-service
else
    "$SCRIPTS/install-arje-splash.sh" --system
fi

# ── etapa compat (opt-in) ─────────────────────────────────────────────────────
if [ "$WITH_COMPAT" = 1 ]; then
    info "compat — install-arje-session-gnome.sh --system"
    "$SCRIPTS/install-arje-session-gnome.sh" --system
fi

# ── etapa init (opt-in; arje de PID 1 sobre tu root, entrada GRUB aparte) ─────
if [ "$WITH_INIT" = 1 ]; then
    info "init — install-arje-init.sh (arje como init alterno; agrega entrada GRUB)"
    if [ "$ASSUME_YES" = 1 ]; then
        "$SCRIPTS/install-arje-init.sh" --yes
    else
        "$SCRIPTS/install-arje-init.sh"
    fi
fi

# ── cierre ────────────────────────────────────────────────────────────────────
cat <<FIN

✓ Capa de sistema tawasuyu instalada sobre tu Linux.

  Para vivir adentro (desde una TTY física, Ctrl+Alt+F3):
      sudo mirada-dm
    · login por PAM; elegí escritorio con  ‹ ›  (mirada · pata / mirada / plugins)
    · probar sin PAM:  sudo MIRADA_GREETER_MOCK=demo:demo mirada-dm
    · o elegí «mirada» desde tu display-manager actual (sesión Wayland).
$([ "$ENABLE_SPLASH" = 1 ] && echo "  · splash habilitado: reiniciá para verlo antes del DM.")
$([ "$WITH_COMPAT"   = 1 ] && echo "  · sesión «GNOME» disponible en el greeter (shims arje-compat instalados).")
$([ "$WITH_INIT"     = 1 ] && echo "  · entrada de GRUB «arje» creada: reiniciá y elegila para vivir EN arje (splash → mirada/pata/shuma). Tu arranque default no cambió.")

  Revertir todo:   ./scripts/install-tawasuyu.sh --uninstall
FIN
