#!/usr/bin/env bash
# shots.sh — renderiza pantallazos headless de las apps Llimphi a PNG.
#
# Convierte "deuda de pantalla" en evidencia mirable: cada app trae un example
# `pantallazo_*` (o `*_shot`) que pinta su UI sin abrir ventana (render_to_view
# sobre wgpu, cae a software si no hay GPU). Este script los corre todos, junta
# los PNG en un directorio y escribe un índice.
#
# Uso:
#   scripts/shots.sh                 # todas las apps → /tmp/tawasuyu-shots/
#   scripts/shots.sh media nahual    # sólo las que matcheen esos nombres
#   OUT=dist/shots scripts/shots.sh  # cambia el directorio de salida
#
# Nota: corre en --release; la primera vez compila y tarda. Cada app falla
# de forma aislada (no aborta el resto) y queda registrada en el índice.

set -uo pipefail
cd "$(dirname "$0")/.."

OUT="${OUT:-/tmp/tawasuyu-shots}"
mkdir -p "$OUT"
INDEX="$OUT/INDICE.md"

# label | crate | example
TABLA=(
  "media|media-app|pantallazo_media"
  "nahual|nahual-shell-llimphi|pantallazo_nahual"
  "nahual-dientes|nahual-shell-llimphi|pantallazo_dientes"
  "pluma-lienzos|pluma-editor-llimphi|pantallazo_lienzos"
  "pluma-multilienzo|pluma-editor-llimphi|pantallazo_multilienzo"
  "pluma-pro|pluma-editor-llimphi|pantallazo_pro"
  "nakui|nakui-ui-llimphi|pantallazo_nakui"
  "cosmos|cosmos-app-llimphi|pantallazo_cosmos"
  "dominium|dominium-app-llimphi|pantallazo_dominium"
  "tinkuy|tinkuy-llimphi|pantallazo_tinkuy"
  "iniy|iniy-explorer-llimphi|pantallazo_iniy"
  "chaka|chaka-app-llimphi|pantallazo_chaka"
  "puriy|puriy-llimphi|pantallazo_puriy"
  "khipu|khipu-app|pantallazo_mapa"
  "nada|nada|pantallazo_nada"
  "tullpu|tullpu-app-llimphi|pantallazo_tullpu"
  "takiy|takiy-app-llimphi|pantallazo_takiy"
  "supay|supay-app-llimphi|pantallazo_supay"
  "agora|agora-app|pantallazo_agora"
  "minga|minga-explorer-llimphi|pantallazo_minga"
  "sandokan|sandokan-monitor-llimphi|pantallazo_sandokan"
  "wawa-explorer|wawa-explorer-llimphi|pantallazo_wawa"
  "llimphi-motor|llimphi-compositor|pantallazo_motor"
  "pata-front|pata-llimphi|front_panel_shot"
  "pata-menu|pata-llimphi|menu_inicio_shot"
  "pata-control|pata-llimphi|control_shot"
  "pata-shuma-drawer|pata-llimphi|pantallazo_shuma_drawer"
)

FILTRO=("$@")
matchea() {
  [ ${#FILTRO[@]} -eq 0 ] && return 0
  for f in "${FILTRO[@]}"; do [[ "$1" == *"$f"* ]] && return 0; done
  return 1
}

echo "# Pantallazos headless — $(date +%Y-%m-%d\ %H:%M)" > "$INDEX"
echo "" >> "$INDEX"

ok=0; fail=0
for fila in "${TABLA[@]}"; do
  IFS='|' read -r label crate ejemplo <<< "$fila"
  matchea "$label" || continue
  png="$OUT/$label.png"
  rm -f "$png"
  printf '· %-18s (%s/%s) … ' "$label" "$crate" "$ejemplo"
  # Compilar y renderizar van por separado: el compilado en --release puede
  # tardar minutos y NO debe contar contra el timeout del render (si no, un
  # crate lento se reporta como "falló" cuando sólo tardó en compilar).
  cargo build -q -p "$crate" --example "$ejemplo" --release > "$OUT/$label.log" 2>&1
  if timeout 120 cargo run -q -p "$crate" --example "$ejemplo" --release -- "$png" \
       >> "$OUT/$label.log" 2>&1 && [ -f "$png" ]; then
    dim=$(file "$png" | grep -oE '[0-9]+ x [0-9]+' | head -1)
    echo "OK ($dim)"
    echo "- **$label** — \`$crate\` · ${dim:-?} — ![]($label.png)" >> "$INDEX"
    ok=$((ok+1))
  else
    echo "FALLÓ (ver $OUT/$label.log)"
    echo "- **$label** — ❌ FALLÓ (\`$crate --example $ejemplo\`; log: $label.log)" >> "$INDEX"
    fail=$((fail+1))
  fi
done

echo ""
echo "Listo: $ok OK · $fail fallaron → $OUT"
echo "Índice: $INDEX"
