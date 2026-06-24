#!/usr/bin/env bash
# test-drm-greeter-metal.sh — verificación de HARDWARE del crossfade
# arje-splash → mirada-greeter sobre DRM real (lo que la SDD marca como
# "observación visual pendiente" y sólo es visible en metal con GPU Intel/AMD).
#
# Qué prueba (SDD-ARRANQUE-SIN-PARPADEO.md §Verificación):
#   1. El crossfade splash→greeter sin gap de BG estático visible.
#   2. Evidencia de texto: epoch_ms del `RELEASED` del splash vs el epoch_ms
#      del primer `queue_frame` presentado por mirada → la duración del gap.
#
# IMPORTANTE: toma DRM master. NO lo corras desde tu sesión gráfica viva.
#   1. Logueá en un VT libre:  Ctrl+Alt+F3  (login)
#   2. Corré este script ahí COMO USUARIO (en el grupo `seat`). Volvés con Ctrl+Alt+F7.
#
# Esta máquina usa `seatd` (no logind). mirada va por libseat→seatd; el splash
# toma el master directo (como arje en el boot) y mirada lo recibe vía seatd
# tras el handoff. No corras esto como root: chocaría con el seatd vigente.
#
# Uso:  ./scripts/test-drm-greeter-metal.sh [/dev/dri/cardN]
set -u

if [ "$(id -u)" = 0 ]; then
  echo "AVISO: corriendo como root — si hay un seatd vigente, libseat puede chocar."
  echo "       En esta máquina conviene correrlo como usuario (grupo seat)."
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEV="${1:-/dev/dri/card1}"

# Wayland (mirada y el greeter) exige un XDG_RUNTIME_DIR válido (0700, propio)
# para crear el socket del display. root y los logins de TTY a veces no lo
# tienen. Si falta o no sirve, fabricamos uno temporal — así el test anda igual
# como usuario o como root.
if [ -z "${XDG_RUNTIME_DIR:-}" ] || [ ! -d "${XDG_RUNTIME_DIR:-/nonexistent}" ] || [ ! -w "${XDG_RUNTIME_DIR:-/nonexistent}" ]; then
  XDG_RUNTIME_DIR="$(mktemp -d /tmp/xdg-rt.XXXXXX)"
  chmod 700 "$XDG_RUNTIME_DIR"
  export XDG_RUNTIME_DIR
  echo "AVISO: XDG_RUNTIME_DIR no válido — usando temporal $XDG_RUNTIME_DIR"
fi

SOCK="$XDG_RUNTIME_DIR/arje-splash.sock"
SPLASH="$ROOT/target/release/arje-splash"
MIRADA="$ROOT/target/release/mirada-compositor"
GREETER="$ROOT/target/release/mirada-greeter"
LOGDIR="$(mktemp -d /tmp/drm-greeter-metal.XXXXXX)"
SPLASH_LOG="$LOGDIR/splash.log"
MIRADA_LOG="$LOGDIR/mirada.log"

for b in "$SPLASH" "$MIRADA" "$GREETER"; do
  [ -x "$b" ] || { echo "FALTA binario: $b — corré: cargo build --release -p arje-splash -p mirada-compositor -p mirada-greeter"; exit 1; }
done
[ -e "$DEV" ] || { echo "no existe el nodo DRM $DEV — revisá /dev/dri"; exit 1; }
[ -r "$DEV" ] || echo "AVISO: no tenés lectura sobre $DEV (¿grupo video? ¿VT activo?) — puede fallar el master"

echo "== test-drm-greeter-metal =="
echo "  device : $DEV"
echo "  socket : $SOCK"
echo "  logs   : $LOGDIR"
echo

cleanup() {
  [ -n "${SPLASH_PID:-}" ] && kill "$SPLASH_PID" 2>/dev/null
  rm -f "$SOCK" 2>/dev/null
}
trap cleanup EXIT INT TERM
rm -f "$SOCK" 2>/dev/null

# 1 · Splash toma el DRM y escucha el handoff. max_ms = red de seguridad: si
#     mirada nunca conecta, se suelta solo a los 20 s (no deja el DRM colgado).
echo "[1] arrancando arje-splash (toma DRM master, espera handoff)…"
ARJE_SPLASH_DEVICE="$DEV" \
ARJE_SPLASH_SOCK="$SOCK" \
ARJE_SPLASH_MAX_MS=20000 \
  "$SPLASH" >"$SPLASH_LOG" 2>&1 &
SPLASH_PID=$!

# Darle tiempo a pintar el splash y bindear el socket antes de que mirada pida.
sleep 2
if ! kill -0 "$SPLASH_PID" 2>/dev/null; then
  echo "arje-splash murió antes del handoff — log:"; sed 's/^/    /' "$SPLASH_LOG"; exit 1
fi

# 2 · mirada arranca como greeter sobre DRM: manda READY, espera RELEASED, toma
#     master y compone el greeter. Foreground: observá el crossfade en pantalla.
echo "[2] arrancando mirada-compositor --drm --greeter (mirá la pantalla)…"
echo "    (salí con Ctrl+C o cambiá de VT cuando hayas visto el greeter)"
MIRADA_GREETER_BIN="$GREETER" \
ARJE_SPLASH_SOCK="$SOCK" \
  "$MIRADA" --drm --greeter 2>&1 | tee "$MIRADA_LOG"

# 3 · Veredicto por texto: gap RELEASED → primer frame de mirada.
echo
echo "== evidencia de texto =="
REL=$(grep -oE 'RELEASED enviado · epoch_ms=[0-9]+' "$SPLASH_LOG" | grep -oE '[0-9]+' | head -1)
FRM=$(grep -oE 'primer queue_frame presentado · epoch_ms=[0-9]+' "$MIRADA_LOG" | grep -oE '[0-9]+' | head -1)
echo "  splash RELEASED        epoch_ms = ${REL:-<no capturado>}"
echo "  mirada primer frame    epoch_ms = ${FRM:-<no capturado>}"
if [ -n "${REL:-}" ] && [ -n "${FRM:-}" ]; then
  echo "  GAP de BG estático     = $((FRM - REL)) ms"
else
  echo "  GAP: no calculable — revisá los logs en $LOGDIR"
  echo "  --- splash.log (cola) ---"; tail -n 15 "$SPLASH_LOG" | sed 's/^/    /'
fi
echo
echo "logs completos en: $LOGDIR"
