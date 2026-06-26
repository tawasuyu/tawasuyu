#!/usr/bin/env bash
# diag-mirada.sh — diagnostica por qué "actualicé mirada pero no se ve nada".
# Corré ESTO EN LA MÁQUINA DONDE USÁS MIRADA (el metal), y pegá la salida.
# No instala ni cambia nada: sólo mira.
set -u

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GRN=$'\e[32m'; RED=$'\e[31m'; YEL=$'\e[33m'; BOLD=$'\e[1m'; RST=$'\e[0m'
[ -t 1 ] || { GRN=""; RED=""; YEL=""; BOLD=""; RST=""; }
ok(){ printf "  ${GRN}✓${RST} %s\n" "$1"; }
no(){ printf "  ${RED}✗${RST} %s\n" "$1"; }
hm(){ printf "  ${YEL}?${RST} %s\n" "$1"; }
hd(){ printf "\n${BOLD}== %s ==${RST}\n" "$1"; }

# Marcadores: cadenas que SÓLO existen si el binario se compiló con los cambios
# nuevos (rótulos del schema + nombres de campos). `strings` los encuentra.
PANEL_MARKERS=("Movimiento" "Esquinas redondeadas" "Glass" "Atenuar ventanas sin foco")
COMP_MARKERS=("window_open_ms" "corner_radius" "glass_blur" "wallpaper_video_fps")

hd "1) Repo (¿están los cambios checked-out?)"
if git -C "$REPO" rev-parse --git-dir >/dev/null 2>&1; then
  echo "  HEAD: $(git -C "$REPO" log -1 --format='%h %ci %s')"
  git -C "$REPO" fetch origin --quiet 2>/dev/null || true
  if git -C "$REPO" merge-base --is-ancestor origin/main HEAD 2>/dev/null; then
    ok "Estás en (o adelante de) origin/main."
  else
    no "Tu HEAD está ATRÁS de origin/main → te falta 'git pull'. (Construir acá da binarios viejos.)"
    echo "    detrás por: $(git -C "$REPO" rev-list --count HEAD..origin/main 2>/dev/null) commits"
  fi
  for c in "window-open\|fade-in" "esquinas redondeadas" "glassmorphism"; do
    git -C "$REPO" log --oneline -i --grep="$c" -1 2>/dev/null | sed 's/^/    commit: /'
  done
else
  no "No es un repo git: $REPO"
fi

hd "2) Binarios instalados (/usr/local/bin) — los que lanza la sesión DM"
for b in mirada-compositor wawa-panel mirada-greeter; do
  f="/usr/local/bin/$b"
  if [ -x "$f" ]; then ok "$b → $(ls -l --time-style=+%Y-%m-%d_%H:%M "$f" 2>/dev/null | awk '{print $6, $7}') ($f)"
  else no "$b NO instalado en /usr/local/bin (¿corriste install-mirada-dm.sh?)"; fi
done

hd "3) ¿El binario instalado contiene los cambios? (decisivo)"
check_markers(){ # $1=ruta  $2..=marcadores
  local f="$1"; shift
  [ -r "$f" ] || { no "no puedo leer $f"; return; }
  local hit=0 tot=0
  for m in "$@"; do tot=$((tot+1)); strings "$f" 2>/dev/null | grep -qiF "$m" && hit=$((hit+1)); done
  if [ "$hit" -eq "$tot" ]; then ok "$(basename "$f"): $hit/$tot marcadores nuevos → ES un binario FRESCO."
  elif [ "$hit" -gt 0 ]; then hm "$(basename "$f"): $hit/$tot marcadores → parcial/raro."
  else no "$(basename "$f"): 0/$tot marcadores → es un binario VIEJO (no tiene los cambios)."; fi
}
[ -r /usr/local/bin/wawa-panel ] && check_markers /usr/local/bin/wawa-panel "${PANEL_MARKERS[@]}"
[ -r /usr/local/bin/mirada-compositor ] && check_markers /usr/local/bin/mirada-compositor "${COMP_MARKERS[@]}"

hd "4) ¿Qué resuelve el PATH? (¿hay copias que tapen?)"
for b in mirada-compositor wawa-panel; do
  p="$(command -v "$b" 2>/dev/null || true)"
  [ -n "$p" ] && { [ "$p" = "/usr/local/bin/$b" ] && ok "$b → $p" || hm "$b → $p  (NO es /usr/local/bin — ¿tapa una copia vieja?)"; } || hm "$b no está en el PATH (la sesión usa ruta directa, ok)"
  for d in "$HOME/.cargo/bin" "$HOME/.local/bin" /usr/bin; do
    [ -e "$d/$b" ] && printf "      copia extra: %s (%s)\n" "$d/$b" "$(date -r "$d/$b" '+%Y-%m-%d %H:%M' 2>/dev/null)"
  done
done

hd "5) ¿Qué lanza la sesión? (.desktop + proceso vivo)"
grep -H "Exec" /usr/share/wayland-sessions/mirada*.desktop 2>/dev/null | sed 's/^/  /' || hm "sin .desktop de mirada en /usr/share/wayland-sessions"
pid="$(pgrep -x mirada-compositor 2>/dev/null | head -1 || true)"
if [ -n "$pid" ]; then ok "mirada-compositor vivo (pid $pid) → exe: $(readlink -f /proc/$pid/exe 2>/dev/null)"
else hm "mirada-compositor no está corriendo ahora (corré esto DENTRO de la sesión para ver el exe real)"; fi

hd "6) Config (puede explicar que el default animado no se vea)"
RON="$HOME/.config/mirada/config.ron"
if [ -f "$RON" ]; then
  ok "config: $RON"
  grep -nE "wallpaper_source|wallpaper_path|reduce_motion|window_open_ms|glass_blur|corner_radius" "$RON" 2>/dev/null | sed 's/^/    /' || echo "    (sin esas claves → usa defaults)"
else hm "no hay $RON (usa todos los defaults — el wallpaper de marca animado debería verse)"; fi

printf "\n${BOLD}Pegá toda esta salida.${RST} Lo decisivo es el bloque (3): si dice VIEJO,\nel install no recompiló desde tus cambios (pull/checkout); si dice FRESCO,\nel problema es runtime (qué binario corre la sesión / config).\n"
