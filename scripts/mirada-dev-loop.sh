#!/usr/bin/env bash
# Loop de desarrollo ANIDADO de mirada con el Cerebro hot-restartable.
#
# Parte el compositor en sus dos mitades (ver mirada-link):
#   - Cuerpo  = mirada-compositor --winit : sostiene las conexiones Wayland de
#               TUS APPS. No se reinicia.
#   - Cerebro = mirada-app-llimphi         : layout, UX, plugins. Lo reiniciás a
#               gusto (o crashea) y el Cuerpo lo re-acepta SIN que las apps se
#               inmuten (App::reconcile_brain en el Cuerpo).
#
# Flujo: `up` una vez; editás el Cerebro; `brain` para verlo en vivo. Si el
# Cerebro paniquea, las apps siguen — relanzás con `brain`.
#
# Anidado = seguro: corre como una ventana dentro de tu sesión actual (sway,
# GNOME…). Si rompés algo, tu escritorio real sigue vivo.
set -euo pipefail

SOCK="${MIRADA_SOCKET:-${XDG_RUNTIME_DIR:-/tmp}/mirada-dev.sock}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUN="${XDG_RUNTIME_DIR:-/tmp}"
CUERPO_PID="$RUN/mirada-dev-cuerpo.pid"
CEREBRO_PID="$RUN/mirada-dev-cerebro.pid"
BIN="$ROOT/target/debug"

lanzar_cerebro() {
    MIRADA_SOCKET="$SOCK" "$BIN/mirada-app-llimphi" &
    echo $! >"$CEREBRO_PID"
}

case "${1:-help}" in
up)
    echo "→ compilando Cuerpo + Cerebro…"
    (cd "$ROOT" && cargo build -p mirada-compositor -p mirada-app-llimphi)
    rm -f "$SOCK"
    echo "→ Cuerpo (compositor anidado) escuchando en $SOCK"
    MIRADA_SOCKET="$SOCK" "$BIN/mirada-compositor" --winit &
    echo $! >"$CUERPO_PID"
    sleep 1
    echo "→ Cerebro"
    lanzar_cerebro
    echo
    echo "listo. Abrí apps dentro de la ventana del Cuerpo (heredan WAYLAND_DISPLAY)."
    echo "Editá el Cerebro y corré:  $0 brain   — lo reinicia; las apps siguen vivas."
    ;;
brain)
    echo "→ recompilando el Cerebro…"
    (cd "$ROOT" && cargo build -p mirada-app-llimphi)
    [ -f "$CEREBRO_PID" ] && kill "$(cat "$CEREBRO_PID")" 2>/dev/null || true
    sleep 0.3
    lanzar_cerebro
    echo "Cerebro reiniciado — el Cuerpo lo re-aceptó y re-sincronizó. Apps intactas."
    ;;
watch)
    # Cuerpo una vez + Cerebro con RESPAWN automático: si el Cerebro crashea,
    # se relanza solo y el Cuerpo lo re-acepta — las apps siguen vivas. Demuestra
    # «error de GUI = parpadeo, no pérdida de sesión». Ctrl-C para salir.
    echo "→ compilando…"
    (cd "$ROOT" && cargo build -p mirada-compositor -p mirada-app-llimphi)
    rm -f "$SOCK"
    MIRADA_SOCKET="$SOCK" "$BIN/mirada-compositor" --winit &
    echo $! >"$CUERPO_PID"
    sleep 1
    trap 'kill "$(cat "$CUERPO_PID" 2>/dev/null)" 2>/dev/null; rm -f "$CUERPO_PID" "$SOCK"; echo; echo bajado.; exit 0' INT TERM
    echo "→ Cerebro en bucle de respawn (Ctrl-C para salir). Matalo y mirá: vuelve solo."
    while true; do
        MIRADA_SOCKET="$SOCK" "$BIN/mirada-app-llimphi" || true
        echo "  · Cerebro salió — relanzando (las apps del Cuerpo siguen vivas)…"
        sleep 0.3
    done
    ;;
down)
    for f in "$CEREBRO_PID" "$CUERPO_PID"; do
        [ -f "$f" ] && kill "$(cat "$f")" 2>/dev/null || true
        rm -f "$f"
    done
    rm -f "$SOCK"
    echo "bajado."
    ;;
*)
    cat <<EOF
uso: $0 {up|brain|watch|down}
  up     — compila y lanza el Cuerpo (anidado, --winit) + el Cerebro
  brain  — recompila y REINICIA sólo el Cerebro (las apps del Cuerpo siguen vivas)
  watch  — Cuerpo + Cerebro con RESPAWN automático (resiliencia a crashes en vivo)
  down   — baja ambos

socket: $SOCK   (override con MIRADA_SOCKET)
EOF
    ;;
esac
