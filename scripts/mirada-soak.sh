#!/bin/sh
# mirada-soak.sh — prueba de remojo (soak test) del compositor en modo
# anidado (winit), para cazar FUGAS: memoria que sólo crece y descriptores
# de archivo que no se liberan. Son la causa #1 de inestabilidad de los
# compositores que "andan bien" las primeras horas y se degradan de noche.
#
# Qué hace: levanta `mirada-compositor --winit` como una ventana dentro de
# tu sesión gráfica actual, y en un lazo abre y cierra un cliente Wayland
# una y otra vez. Mientras tanto, muestrea cada pocos segundos el RSS
# (memoria residente) y el número de descriptores abiertos del compositor,
# y los registra como TEXTO en el directorio de debug. Si la curva sube
# monótona en lugar de estabilizarse, hay una fuga.
#
# NO mira PNGs ni video: la evidencia es numérica (KiB de RSS, conteo de fds),
# barata en tokens y la que de verdad delata una fuga.
#
# Uso:
#   scripts/mirada-soak.sh [segundos]          # default 600s (10 min)
# Entorno:
#   MIRADA_SOAK_CLIENT="foot"   cliente Wayland a ciclar (autodetecta si falta)
#   MIRADA_SOAK_PERIOD=5        segundos entre muestras
#   MIRADA_SOAK_HOLD=2          segundos que vive cada cliente antes de cerrarlo
#   MIRADA_DEBUG_DIR=...        dónde escribir (default ~/.local/state/mirada)
#
# Requisitos: una sesión gráfica anfitriona (X11 o Wayland) — el backend
# winit dibuja la ventana del compositor dentro de ella.
set -u

DUR="${1:-600}"
PERIOD="${MIRADA_SOAK_PERIOD:-5}"
HOLD="${MIRADA_SOAK_HOLD:-2}"
DIR="${MIRADA_DEBUG_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/mirada}"
mkdir -p "$DIR" 2>/dev/null || true
OUT="$DIR/soak.log"

COMPOSITOR="${MIRADA_COMPOSITOR_BIN:-mirada-compositor}"
command -v "$COMPOSITOR" >/dev/null 2>&1 || {
    # Fallback al binario del workspace si no está instalado en el PATH.
    REPO=$(cd "$(dirname "$0")/.." && pwd)
    for cand in "$REPO/target/release/mirada-compositor" "$REPO/target/debug/mirada-compositor"; do
        [ -x "$cand" ] && COMPOSITOR="$cand" && break
    done
}
command -v "$COMPOSITOR" >/dev/null 2>&1 || [ -x "$COMPOSITOR" ] || {
    echo "soak: no encuentro mirada-compositor (instalalo o compilá con cargo build -p mirada-compositor)" >&2
    exit 1
}

# Cliente Wayland a ciclar: el indicado, o el primero disponible.
CLIENT="${MIRADA_SOAK_CLIENT:-}"
if [ -z "$CLIENT" ]; then
    for c in foot weston-terminal alacritty kitty wayland-info weston-info; do
        command -v "$c" >/dev/null 2>&1 && CLIENT="$c" && break
    done
fi
[ -n "$CLIENT" ] || {
    echo "soak: no hay cliente Wayland para ciclar (instalá foot/weston-terminal, o pasá MIRADA_SOAK_CLIENT)" >&2
    exit 1
}

stamp() { date '+%Y-%m-%d %H:%M:%S' 2>/dev/null || echo '?'; }

# Muestrea RSS (KiB) y nº de fds de un PID desde /proc — sin herramientas extra.
rss_kib() { awk '/^VmRSS:/ {print $2}' "/proc/$1/status" 2>/dev/null || echo 0; }
fd_count() { ls "/proc/$1/fd" 2>/dev/null | wc -l | tr -d ' '; }

CKILL=""
COMP_PID=""
cleanup() {
    [ -n "$CKILL" ] && kill "$CKILL" 2>/dev/null
    [ -n "$COMP_PID" ] && kill "$COMP_PID" 2>/dev/null
}
trap cleanup EXIT INT TERM

echo "soak: compositor=$COMPOSITOR  cliente=$CLIENT  duración=${DUR}s  período=${PERIOD}s"
echo "soak: registrando muestras numéricas en $OUT"
{
    echo "# ── soak $(stamp) · compositor=$COMPOSITOR cliente=$CLIENT dur=${DUR}s ──"
    echo "# t_seg  rss_kib  fds  ciclos_cliente"
} >>"$OUT"

# Levantamos el compositor anidado. Hereda el WAYLAND_DISPLAY/DISPLAY del host
# para anidarse; luego republica el suyo para los clientes que lancemos.
"$COMPOSITOR" --winit >"$DIR/soak-compositor.log" 2>&1 &
COMP_PID=$!

# Esperamos a que publique su socket (lo imprime: "WAYLAND_DISPLAY=wayland-N").
SOCK=""
i=0
while [ "$i" -lt 50 ]; do
    SOCK=$(sed -n 's/.*WAYLAND_DISPLAY=\([a-z0-9-]*\).*/\1/p' "$DIR/soak-compositor.log" 2>/dev/null | head -1)
    [ -n "$SOCK" ] && break
    kill -0 "$COMP_PID" 2>/dev/null || { echo "soak: el compositor no arrancó; mirá $DIR/soak-compositor.log" >&2; exit 1; }
    i=$((i + 1)); sleep 0.2
done
[ -n "$SOCK" ] || { echo "soak: no detecté el WAYLAND_DISPLAY del compositor; sigo con el del entorno" >&2; SOCK="${WAYLAND_DISPLAY:-}"; }
echo "soak: compositor pid=$COMP_PID  WAYLAND_DISPLAY=$SOCK"

start=$(date +%s 2>/dev/null || echo 0)
cycles=0
next_sample=0

while : ; do
    now=$(date +%s 2>/dev/null || echo 0)
    t=$(( now - start ))
    [ "$t" -ge "$DUR" ] && break
    kill -0 "$COMP_PID" 2>/dev/null || { echo "soak: el compositor murió a los ${t}s (ver crash-*.log)" >&2; break; }

    # Ciclo de cliente: abrir, sostener, cerrar — ejercita open/map/unmap/close.
    WAYLAND_DISPLAY="$SOCK" "$CLIENT" >/dev/null 2>&1 &
    CKILL=$!
    sleep "$HOLD"
    kill "$CKILL" 2>/dev/null
    wait "$CKILL" 2>/dev/null
    CKILL=""
    cycles=$(( cycles + 1 ))

    # Muestra periódica (no en cada ciclo, para no inundar el log).
    if [ "$t" -ge "$next_sample" ]; then
        echo "$t  $(rss_kib "$COMP_PID")  $(fd_count "$COMP_PID")  $cycles" >>"$OUT"
        next_sample=$(( t + PERIOD ))
    fi
done

# Muestra final + veredicto rápido (primera vs última lectura de RSS/fds).
echo "$(( $(date +%s 2>/dev/null || echo 0) - start ))  $(rss_kib "$COMP_PID")  $(fd_count "$COMP_PID")  $cycles" >>"$OUT"
echo "soak: $cycles ciclos de cliente. Curva en $OUT:"
# Resumen textual: muestra las filas de datos y deja que el ojo (o un diff)
# juzgue si RSS/fds se estabilizan o crecen monótonos.
grep -E '^[0-9]' "$OUT" | tail -n 20
echo "soak: si rss_kib o fds suben sin amesetar → fuga. Compará primeras vs últimas filas."
