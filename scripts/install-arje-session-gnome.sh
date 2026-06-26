#!/usr/bin/env bash
# Instala el perfil de sesión GNOME de arje: los shims D-Bus de arje-compat
# (logind, hostnamed, timedated, …) + el fragmento de Card que los agrupa.
# Con esto, elegir la sesión «GNOME» en el greeter levanta esos backends al
# login (vía SpawnCardFromDisk → bundle), y `arje.session=gnome` en el cmdline
# los levanta al boot (vía overlay). Ver 03_ukupacha/arje/seeds/fragments/.
#
# Modos (no excluyentes):
#
#   --system        EAGER. Instala en el sistema actual:
#                     <prefix>/lib/arje/arje-*-compat   (binarios, 0755)
#                     /etc/arje/cards.d/session-gnome.json   (el bundle)
#                   El greeter levanta los 12 shims al elegir la sesión.
#
#   --lazy          LAZY. Como --system, y además instala arje-activate + los
#                   .service de activación D-Bus (uno por nombre) en el bus del
#                   host + el marcador session-gnome.lazy. Los ships arrancan
#                   on-demand (al primer request del nombre), no al login; el
#                   greeter no los levanta eager (sí los baja al salir).
#                   Requiere: un dbus-daemon de sistema en el host, jq, y correr
#                   arje-zero con ENTE_BUS_SOCK=/run/arje/bus.sock.
#
#   --emit-flags    Compila los shims estáticos (musl) y EMITE los flags
#                   --asset/--bin para sumar a tu `arje-installer` de host
#                   (arranque nativo). El bundle se superpone a la imagen de
#                   host; este script no rehace la instalación de host.
#
# SEGURO POR DEFECTO: copia archivos e imprime los pasos de activación; no
# habilita nada solo. Requisitos: rust/cargo; --emit-flags además musl-gcc;
# --lazy además jq + un dbus-daemon de sistema.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

MODE=""                # system | lazy | flags
PREFIX="${PREFIX:-/usr/local}"
TARGET="x86_64-unknown-linux-musl"
FRAGMENT="03_ukupacha/arje/seeds/fragments/session-gnome.card.json"

die() { echo "✗ $*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --system)      MODE="system" ;;
        --lazy)        MODE="lazy" ;;
        --emit-flags)  MODE="flags" ;;
        --prefix)      PREFIX="${2:?--prefix requiere un directorio}"; shift ;;
        -h|--help)     sed -n '2,30p' "$0"; exit 0 ;;
        *) die "opción desconocida: $1 (usá --system, --lazy o --emit-flags)" ;;
    esac
    shift
done
[ -n "$MODE" ] || die "elegí un modo: --system | --lazy | --emit-flags (ver --help)"
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

# Nombres D-Bus de los shims que reclaman un nombre `org.freedesktop.*` (los
# que GNOME consulta y, por tanto, los activables on-demand). Mapea label →
# nombre del bus. Los otros shims (journald/notify/timer/policy-provider) no
# son targets de activación por nombre. Fuente: `const BUS_NAME` en cada bin.
DBUS_NAMES=(
    "compat-logind=org.freedesktop.login1"
    "compat-hostnamed=org.freedesktop.hostname1"
    "compat-timedated=org.freedesktop.timedate1"
    "compat-localed=org.freedesktop.locale1"
    "compat-polkit=org.freedesktop.PolicyKit1"
    "compat-systemd1=org.freedesktop.systemd1"
    "compat-resolved=org.freedesktop.resolve1"
    "compat-machined=org.freedesktop.machine1"
)

# ════════════════════════════════════════════════════════════════════════════
# Base común a --system y --lazy: compila e instala shims + el bundle.
if [ "$MODE" = "system" ] || [ "$MODE" = "lazy" ]; then
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
fi

# ── Sólo --system: mensaje eager ────────────────────────────────────────────
if [ "$MODE" = "system" ]; then
    echo
    echo "✓ perfil GNOME instalado (eager). Para usarlo:"
    echo "    • En el greeter: elegí la sesión «GNOME» — el DM pide el bundle al login."
    echo "    • O al boot: agregá  arje.session=gnome  al cmdline del kernel."
    echo "  (Los 12 shims arrancan juntos al elegir la sesión; mirada es el default.)"
fi

# ── Sólo --lazy: arje-activate + .service de activación + marcador ───────────
if [ "$MODE" = "lazy" ]; then
    have jq || die "el modo --lazy necesita jq (para extraer los shims del bundle)"

    echo "==> instalando arje-activate en $LIBDIR/"
    [ -f "$M/arje-activate" ] || die "no se compiló $M/arje-activate"
    $SUDO install -Dm755 "$M/arje-activate" "$LIBDIR/arje-activate"

    # Una card por shim en el store (SpawnCardFromDisk{name} las lee): se
    # extraen del genesis del bundle, así conservan ULID y estructura válidos
    # sin duplicar una fuente. El bundle queda igual (teardown por label).
    echo "==> generando cards por-shim en /etc/arje/cards.d/"
    for pair in "${DBUS_NAMES[@]}"; do
        label="${pair%%=*}"
        TMP="$(mktemp)"
        jq -e --arg l "$label" '.genesis[] | select(.label==$l)' "$FRAGMENT" > "$TMP" \
            || die "no encontré el shim $label en $FRAGMENT"
        $SUDO install -Dm644 "$TMP" "/etc/arje/cards.d/$label.json"
        rm -f "$TMP"
        echo "   /etc/arje/cards.d/$label.json"
    done

    # Un .service de activación D-Bus por nombre, apuntando a arje-activate.
    # El dbus-daemon del host lo dispara al primer request del nombre; el
    # patrón es el de `SystemdService=` pero con arje como manager.
    SVCDIR="/usr/share/dbus-1/system-services"
    echo "==> generando .service de activación en $SVCDIR/"
    for pair in "${DBUS_NAMES[@]}"; do
        label="${pair%%=*}"; bus="${pair#*=}"
        TMP="$(mktemp)"
        printf '[D-BUS Service]\nName=%s\nExec=%s/arje-activate %s\nUser=root\n' \
            "$bus" "$LIBDIR" "$label" > "$TMP"
        $SUDO install -Dm644 "$TMP" "$SVCDIR/$bus.service"
        rm -f "$TMP"
        echo "   $bus.service → arje-activate $label"
    done

    # Marcador: el greeter lo ve y NO levanta gnome eager (dbus lo activa
    # on-demand). El teardown al salir lo sigue haciendo el greeter.
    echo "==> dejando el marcador /etc/arje/session-gnome.lazy"
    $SUDO mkdir -p /etc/arje
    printf 'gnome se activa on-demand vía dbus-daemon + arje-activate.\n' \
        | $SUDO tee /etc/arje/session-gnome.lazy >/dev/null

    echo
    echo "✓ perfil GNOME instalado (lazy). Contrato de deploy:"
    echo "    • Corré arje-zero con  ENTE_BUS_SOCK=/run/arje/bus.sock  (el path que"
    echo "      usa arje-activate cuando el dbus-daemon del host lo invoca)."
    echo "    • Necesitás un dbus-daemon de sistema corriendo en el host."
    echo "  Los shims arrancan al primer request de su nombre; el greeter no los"
    echo "  levanta eager (marcador), pero sí los baja al salir de la sesión."
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
