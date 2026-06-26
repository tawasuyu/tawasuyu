#!/usr/bin/env bash
# Instala el perfil de sesión GNOME de arje: los shims D-Bus de arje-compat
# (logind, hostnamed, timedated, …) + el fragmento de Card que los agrupa.
# Con esto, elegir la sesión «GNOME» en el greeter levanta esos backends al
# login (vía SpawnCardFromDisk → bundle), y `arje.session=gnome` en el cmdline
# los levanta al boot (vía overlay). Ver 03_ukupacha/arje/seeds/fragments/.
#
# Dos modos (no excluyentes):
#
#   --system        Instala en el sistema actual:
#                     <prefix>/lib/arje/arje-*-compat   (binarios, 0755)
#                     /etc/arje/cards.d/session-gnome.json   (el bundle)
#                   Es lo que necesita la vía login-time del greeter.
#
#   --emit-flags    Compila los shims estáticos (musl) y EMITE los flags
#                   --asset/--bin para sumar a tu `arje-installer` de host
#                   (arranque nativo). El bundle se superpone a la imagen de
#                   host; este script no rehace la instalación de host.
#
# SEGURO POR DEFECTO: copia archivos e imprime los pasos de activación; no
# habilita nada solo. Requisitos: rust/cargo; para --emit-flags además
# target musl + musl-gcc.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

MODE=""                # system | flags
PREFIX="${PREFIX:-/usr/local}"
TARGET="x86_64-unknown-linux-musl"
FRAGMENT="03_ukupacha/arje/seeds/fragments/session-gnome.card.json"

die() { echo "✗ $*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --system)      MODE="system" ;;
        --emit-flags)  MODE="flags" ;;
        --prefix)      PREFIX="${2:?--prefix requiere un directorio}"; shift ;;
        -h|--help)     sed -n '2,24p' "$0"; exit 0 ;;
        *) die "opción desconocida: $1 (usá --system o --emit-flags)" ;;
    esac
    shift
done
[ -n "$MODE" ] || die "elegí un modo: --system  y/o  --emit-flags (ver --help)"
have cargo || die "falta cargo (instalá Rust: https://rustup.rs)"
[ -f "$FRAGMENT" ] || die "no encuentro el fragmento $FRAGMENT"

# Mapa label-del-fragmento → binario de arje-compat. Los labels deben casar
# con los del genesis de session-gnome.card.json (los lee el installer para
# saber qué binarios hornear); los exec del fragmento apuntan a
# /usr/lib/arje/<binario>.
SHIMS=(
    "compat-logind=arje-logind-compat"
    "compat-hostnamed=arje-hostnamed-compat"
    "compat-timedated=arje-timedated-compat"
    "compat-localed=arje-localed-compat"
    "compat-polkit=arje-polkit-compat"
    "compat-systemd1=arje-systemd1-compat"
    "compat-journald=arje-journald-compat"
    "compat-resolved=arje-resolved-compat"
    "compat-machined=arje-machined-compat"
    "policy-provider=arje-policy-provider"
    "compat-notify=arje-notify-compat"
    "compat-timer=arje-timer-compat"
)

# ════════════════════════════════════════════════════════════════════════════
if [ "$MODE" = "system" ]; then
    echo "==> compilando arje-compat (release, nativo)"
    cargo build --release -p arje-compat
    M="target/release"

    LIBDIR="$PREFIX/lib/arje"
    SUDO=""; [ -w "$(dirname "$LIBDIR")" ] || SUDO="sudo"
    [ -n "$SUDO" ] && echo "==> usando sudo para escribir en $PREFIX y /etc"

    echo "==> instalando shims en $LIBDIR/"
    for pair in "${SHIMS[@]}"; do
        bin="${pair#*=}"
        [ -f "$M/$bin" ] || die "no se compiló $M/$bin"
        $SUDO install -Dm755 "$M/$bin" "$LIBDIR/$bin"
        echo "   $bin"
    done

    echo "==> instalando bundle en /etc/arje/cards.d/session-gnome.json"
    $SUDO install -Dm644 "$FRAGMENT" /etc/arje/cards.d/session-gnome.json

    echo
    echo "✓ perfil GNOME instalado. Para usarlo:"
    echo "    • En el greeter: elegí la sesión «GNOME» — el DM pide el bundle al login."
    echo "    • O al boot: agregá  arje.session=gnome  al cmdline del kernel."
    echo "  (Los shims se encarnan on-demand; mirada sigue siendo el default.)"
fi

# ════════════════════════════════════════════════════════════════════════════
if [ "$MODE" = "flags" ]; then
    have musl-gcc || have x86_64-linux-musl-gcc || die "falta musl-gcc (binarios estáticos)"
    rustup target list --installed 2>/dev/null | grep -qx "$TARGET" \
        || die "falta: rustup target add $TARGET"

    echo "==> compilando shims estáticos (musl)" >&2
    cargo build --release --target "$TARGET" -p arje-compat >&2
    M="target/$TARGET/release"

    # El fragmento como asset (JSON, 0644) + un --bin por shim (por LABEL del
    # fragmento). El installer ahora recoge los execs de las cards de
    # /etc/arje/cards.d/ y hornea sus binarios (0755). Ver lib.rs
    # collect_card_execs.
    echo "==> flags para sumar a tu  arje-installer to-partition|to-usb ...:" >&2
    echo "  --asset etc/arje/cards.d/session-gnome.json=$FRAGMENT"
    for pair in "${SHIMS[@]}"; do
        label="${pair%%=*}"; bin="${pair#*=}"
        [ -f "$M/$bin" ] || die "no se compiló $M/$bin"
        echo "  --bin $label=$M/$bin"
    done
    echo >&2
    echo "✓ Pegá esos flags en tu invocación de arje-installer (modo ESP/USB)." >&2
fi
