#!/usr/bin/env bash
# Instalador del splash de arranque sin parpadeo de arje, portable a cualquier
# Linux. Dos caminos (no excluyentes):
#
#   --system            Instala el binario + config en el sistema actual:
#                         <prefix>/lib/arje/arje-splash  y  /etc/arje/splash.conf
#                       Opcional: una systemd unit (Plymouth-like) que lo corre
#                       antes del display-manager. La config la edita wawa-panel
#                       (sección «Arranque»).
#   --esp DIR           Arma una ESP booteable (arje arranca la máquina nativo,
#                       con el splash desde el frame cero). Envuelve arje-installer.
#
# Splash: --image PNG  o  --frames DIR  (carpeta de *.png). Sin ninguno, queda el
# splash nativo (logo de marca respirando).
#
# SEGURO POR DEFECTO: instala archivos y *imprime* los pasos de activación; no
# habilita servicios ni toca tu bootloader/initramfs sin que lo pidas
# explícitamente (--enable-service). Requisitos: rust/cargo; para --esp además
# qemu-tools/arje-installer y target musl + musl-gcc.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

MODE=""                # system | esp
PREFIX="${PREFIX:-/usr/local}"
ESP_DIR=""
IMAGE=""; FRAMES=""
ENABLE_SERVICE=0
KERNEL="${KERNEL:-}"
SEED="${SEED:-03_ukupacha/arje/seeds/arje-demo.card.json}"
TARGET="x86_64-unknown-linux-musl"

die() { echo "✗ $*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --system)         MODE="system" ;;
        --esp)            MODE="esp"; ESP_DIR="${2:?--esp requiere un directorio (ESP montada)}"; shift ;;
        --prefix)         PREFIX="${2:?}"; shift ;;
        --image)          IMAGE="${2:?}"; shift ;;
        --frames)         FRAMES="${2:?}"; shift ;;
        --enable-service) ENABLE_SERVICE=1 ;;
        -h|--help)        sed -n '2,22p' "$0"; exit 0 ;;
        *) die "opción desconocida: $1 (usá --system o --esp DIR)" ;;
    esac
    shift
done
[ -n "$MODE" ] || die "elegí un modo: --system  y/o  --esp DIR (ver --help)"
have cargo || die "falta cargo (instalá Rust: https://rustup.rs)"

# Si no se pasó --image/--frames, tomamos la config que dejó wawa-panel
# (sección «Arranque»): ~/.config/arje/splash.conf. Leemos su source y la ruta.
PANEL_CONF="${ARJE_SPLASH_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/arje/splash.conf}"
if [ -z "$IMAGE" ] && [ -z "$FRAMES" ] && [ -f "$PANEL_CONF" ]; then
    psrc=$(sed -n 's/^source *= *//p' "$PANEL_CONF" | tail -1)
    case "$psrc" in
        image)  IMAGE=$(sed -n 's/^image *= *//p' "$PANEL_CONF" | tail -1) ;;
        frames) FRAMES=$(sed -n 's/^frames *= *//p' "$PANEL_CONF" | tail -1) ;;
    esac
    [ -n "$IMAGE$FRAMES" ] && echo "==> usando la config de wawa-panel ($PANEL_CONF)"
fi

# ── Genera /etc/arje/splash.conf según --image/--frames ─────────────────────
write_conf() {  # $1 = destino del .conf, $2 = base de rutas en disco destino
    local conf="$1" base="$2"
    if [ -n "$IMAGE" ]; then
        printf 'source = image\nimage = %s/splash.png\nlogs = auto\n' "$base"
    elif [ -n "$FRAMES" ]; then
        printf 'source = frames\nframes = %s/frames\nlogs = auto\n' "$base"
    else
        printf 'source = builtin\nlogs = auto\n'
    fi > "$conf"
}

# ════════════════════════════════════════════════════════════════════════════
if [ "$MODE" = "system" ]; then
    echo "==> compilando arje-splash (release, nativo)"
    cargo build --release -p arje-splash
    BIN="target/release/arje-splash"

    LIBDIR="$PREFIX/lib/arje"
    SUDO=""; [ -w "$(dirname "$LIBDIR")" ] || SUDO="sudo"
    [ -n "$SUDO" ] && echo "==> usando sudo para escribir en $PREFIX y /etc"

    echo "==> instalando binario en $LIBDIR/arje-splash"
    $SUDO install -Dm755 "$BIN" "$LIBDIR/arje-splash"

    echo "==> instalando config en /etc/arje/"
    $SUDO mkdir -p /etc/arje
    if [ -e /etc/arje/splash.conf ]; then
        echo "   /etc/arje/splash.conf ya existe — no lo piso (lo gestiona wawa-panel)"
    else
        TMP="$(mktemp)"; write_conf "$TMP" "/etc/arje"; $SUDO install -Dm644 "$TMP" /etc/arje/splash.conf; rm -f "$TMP"
    fi
    [ -n "$IMAGE" ]  && { [ -f "$IMAGE" ] || die "no existe $IMAGE"; $SUDO install -Dm644 "$IMAGE" /etc/arje/splash.png; echo "   imagen → /etc/arje/splash.png"; }
    [ -n "$FRAMES" ] && { [ -d "$FRAMES" ] || die "no existe $FRAMES"; $SUDO mkdir -p /etc/arje/frames; $SUDO cp "$FRAMES"/*.png /etc/arje/frames/; echo "   frames → /etc/arje/frames/"; }

    # systemd unit (Plymouth-like): corre el splash antes del display-manager.
    UNIT=/etc/systemd/system/arje-splash.service
    echo "==> instalando systemd unit en $UNIT (no se habilita sola)"
    $SUDO tee "$UNIT" >/dev/null <<UNITEOF
[Unit]
Description=arje splash — arranque sin parpadeo
DefaultDependencies=no
After=systemd-udev-settle.service
Before=display-manager.service plymouth-quit.service
Conflicts=plymouth-start.service

[Service]
Type=oneshot
ExecStart=$LIBDIR/arje-splash
TimeoutStartSec=15

[Install]
WantedBy=graphical.target
UNITEOF

    if [ "$ENABLE_SERVICE" = 1 ] && have systemctl; then
        echo "==> habilitando arje-splash.service"
        $SUDO systemctl daemon-reload
        $SUDO systemctl enable arje-splash.service
        echo "✓ servicio habilitado. Reiniciá para verlo."
    else
        echo
        echo "✓ instalado. Para activarlo como splash del arranque:"
        echo "    sudo systemctl enable arje-splash.service"
        echo "  (o re-corré con --enable-service). La config la editás desde wawa-panel,"
        echo "  sección «Arranque», o a mano en /etc/arje/splash.conf."
        echo "  Nota: el camino SIN parpadeo de punta a punta es el arranque NATIVO de arje"
        echo "  (--esp); como servicio del host el splash aparece, pero el firmware→kernel"
        echo "  previo depende de tu initramfs/Plymouth."
    fi
fi

# ════════════════════════════════════════════════════════════════════════════
if [ "$MODE" = "esp" ]; then
    have musl-gcc || have x86_64-linux-musl-gcc || die "falta musl-gcc (binarios estáticos)"
    rustup target list --installed 2>/dev/null | grep -qx "$TARGET" || die "falta: rustup target add $TARGET"
    [ -d "$ESP_DIR" ] || die "la ESP $ESP_DIR no existe (montala primero)"
    if [ -z "$KERNEL" ]; then
        for k in /boot/vmlinuz-linux /boot/vmlinuz-linux-lts /boot/vmlinuz; do [ -f "$k" ] && KERNEL="$k" && break; done
        [ -n "$KERNEL" ] || KERNEL="$(ls -1t /boot/vmlinuz* 2>/dev/null | head -1)"
    fi
    [ -n "$KERNEL" ] && [ -f "$KERNEL" ] || die "no encontré kernel; pasá KERNEL=/ruta/vmlinuz"

    echo "==> compilando estáticos (musl)"
    cargo build --release --target "$TARGET" -p arje-zero -p arje-splash -p arje-getty-stub
    cargo build --release -p arje-installer
    M="target/$TARGET/release"

    # Config + assets del splash, horneados vía arje-installer --asset/--bin.
    TMP="$(mktemp -d)"; write_conf "$TMP/splash.conf" "/etc/arje"
    ASSETS=(--asset "etc/arje/splash.conf=$TMP/splash.conf")
    [ -n "$IMAGE" ]  && ASSETS+=(--asset "etc/arje/splash.png=$IMAGE")
    if [ -n "$FRAMES" ]; then for f in "$FRAMES"/*.png; do ASSETS+=(--asset "etc/arje/frames/$(basename "$f")=$f"); done; fi

    echo "==> staging a la ESP $ESP_DIR"
    ./target/release/arje-installer to-partition \
        --esp "$ESP_DIR" --kernel "$KERNEL" --seed "$SEED" \
        --bin arje-zero="$M/arje-zero" \
        --bin arje-splash="$M/arje-splash" \
        --bin greeter-sim="$M/arje-splash" \
        --bin agetty-ttyS0="$M/arje-getty-stub" \
        "${ASSETS[@]}"
    rm -rf "$TMP"
    echo "✓ ESP lista en $ESP_DIR — booteá esa partición por UEFI (arje arranca nativo)."
fi
