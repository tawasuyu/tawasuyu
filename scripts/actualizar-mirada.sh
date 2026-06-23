#!/usr/bin/env bash
# =============================================================================
#  scripts/actualizar-mirada.sh — pone una máquina al día con mirada+pata
# -----------------------------------------------------------------------------
#  Síntoma que resuelve: "hice git pull y el escritorio salió feo / el panel
#  de pata pisa las ventanas / volvieron bugs que ya estaban arreglados".
#
#  La causa casi nunca es la config (~/.config/mirada/*.ron) — eso es estado
#  local por máquina. La causa real es un BINARIO VIEJO: el arreglo de que
#  pata no pise las ventanas necesita que DOS procesos coincidan al día:
#
#    · mirada-compositor  (el compositor, honra la zona exclusiva)
#    · pata-llimphi       (el panel, pide la zona exclusiva)
#
#  Si cualquiera de los dos quedó viejo (no se pulleó, no se recompiló, o se
#  lanza una copia instalada vieja que tapa al recién compilado), la
#  cooperación se rompe y el panel se superpone.
#
#  Este script, sin borrar nada por su cuenta:
#    1. pone el repo al día con origin/main (fast-forward, avisa si no puede);
#    2. recompila LOS DOS binarios en release;
#    3. detecta binarios viejos en el PATH que TAPARÍAN a los recién forjados;
#    4. imprime las rutas canónicas a lanzar y recuerda reiniciar la sesión.
#
#  Uso:  ./scripts/actualizar-mirada.sh          # pull + build + diagnóstico
#        ./scripts/actualizar-mirada.sh --no-pull # sólo build + diagnóstico
# =============================================================================
set -euo pipefail

# --- ubicación: corre desde cualquier cwd, se ancla en la raíz del repo ------
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# --- coloreado sobrio (se apaga si no hay TTY) -------------------------------
if [ -t 1 ]; then
  BOLD=$'\e[1m'; DIM=$'\e[2m'; RED=$'\e[31m'; GRN=$'\e[32m'; YEL=$'\e[33m'; RST=$'\e[0m'
else
  BOLD=""; DIM=""; RED=""; GRN=""; YEL=""; RST=""
fi
paso()  { printf "\n%s==> %s%s\n" "$BOLD" "$1" "$RST"; }
ok()    { printf "  %s✓%s %s\n" "$GRN" "$RST" "$1"; }
warn()  { printf "  %s!%s %s\n" "$YEL" "$RST" "$1"; }
err()   { printf "  %s✗%s %s\n" "$RED" "$RST" "$1"; }

DO_PULL=1
[ "${1:-}" = "--no-pull" ] && DO_PULL=0

# Los dos binarios que tienen que coincidir, como "crate:binario".
BINS=("mirada-compositor:mirada-compositor" "pata-llimphi:pata-llimphi")

# -----------------------------------------------------------------------------
# 1. Poner el repo al día con origin/main
# -----------------------------------------------------------------------------
if [ "$DO_PULL" = 1 ]; then
  paso "Sincronizando con origin/main"
  git fetch origin --quiet
  ANTES="$(git rev-parse --short HEAD)"

  if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    warn "Hay cambios locales sin commitear — NO toco tu working tree."
    warn "Resolvé a mano (git stash / git commit) y volvé a correr, o usá --no-pull."
    git status -sb | sed 's/^/    /'
    exit 1
  fi

  if git merge-base --is-ancestor origin/main HEAD; then
    ok "Ya estás en (o adelante de) origin/main."
  else
    git merge-base --is-ancestor HEAD origin/main \
      && git pull --ff-only origin main --quiet \
      || { err "main divergió de origin/main (no es fast-forward)."
           err "Mirá 'git log --oneline HEAD..origin/main' y resolvé a mano."; exit 1; }
  fi
  DESPUES="$(git rev-parse --short HEAD)"
  [ "$ANTES" = "$DESPUES" ] && ok "Sin cambios nuevos ($DESPUES)." \
                            || ok "Avanzado: $ANTES → $DESPUES."
else
  paso "Saltando el pull (--no-pull)"
fi

printf "  %sHEAD:%s %s\n" "$DIM" "$RST" "$(git log -1 --format='%h %ci %s')"

# -----------------------------------------------------------------------------
# 2. Recompilar los dos binarios en release
# -----------------------------------------------------------------------------
paso "Recompilando mirada-compositor + pata-llimphi (release)"
PKG_ARGS=()
for entry in "${BINS[@]}"; do PKG_ARGS+=(-p "${entry%%:*}"); done
cargo build --release "${PKG_ARGS[@]}"
ok "Build release OK."

# -----------------------------------------------------------------------------
# 3. Detectar binarios viejos en el PATH que taparían a los recién forjados
# -----------------------------------------------------------------------------
paso "Buscando binarios viejos que tapen a los nuevos"
TARGET_DIR="$REPO_ROOT/target/release"
SHADOW=0
for entry in "${BINS[@]}"; do
  bin="${entry##*:}"
  fresco="$TARGET_DIR/$bin"
  [ -x "$fresco" ] || { warn "No se forjó $fresco (¿falló el build?)."; continue; }

  # ¿Qué resuelve el PATH para este nombre? (lo que realmente se lanzaría)
  enpath="$(command -v "$bin" 2>/dev/null || true)"
  if [ -n "$enpath" ] && [ "$enpath" -ef "$fresco" ]; then
    ok "$bin → el PATH ya apunta al recién forjado."
  elif [ -n "$enpath" ]; then
    err "$bin → el PATH lanza una copia VIEJA: $enpath"
    err "       el fresco está en:            $fresco"
    SHADOW=1
  else
    warn "$bin → no está en el PATH; lanzalo por ruta absoluta: $fresco"
  fi

  # Copias sospechosas en lugares de instalación comunes (informativo).
  for d in "$HOME/.cargo/bin" /usr/local/bin /usr/bin "$HOME/.local/bin"; do
    if [ -e "$d/$bin" ] && ! [ "$d/$bin" -ef "$fresco" ]; then
      printf "      %sinstalado viejo:%s %s  (%s)\n" "$DIM" "$RST" "$d/$bin" \
        "$(date -r "$d/$bin" '+%Y-%m-%d %H:%M' 2>/dev/null || echo '?')"
    fi
  done
done

# -----------------------------------------------------------------------------
# 4. Cierre: qué lanzar y el recordatorio de reiniciar la sesión
# -----------------------------------------------------------------------------
paso "Listo"
if [ "$SHADOW" = 1 ]; then
  warn "Hay copias viejas tapando en el PATH. O bien:"
  warn "  · reinstalá:  cargo install --path 02_ruway/mirada/mirada-compositor --force"
  warn "                cargo install --path 02_ruway/pata/pata-llimphi --force"
  warn "  · o borrá la copia vieja y lanzá desde target/release/."
fi
echo   "  Binarios canónicos recién forjados:"
for entry in "${BINS[@]}"; do echo "    $TARGET_DIR/${entry##*:}"; done
echo
echo   "  ${BOLD}Reiniciá la SESIÓN de mirada completa${RST} (salir del compositor y volver"
echo   "  a entrar) — relanzar sólo pata no alcanza: los dos procesos tienen que"
echo   "  arrancar frescos para que la zona exclusiva se honre."
