#!/usr/bin/env bash
# =============================================================================
#  scripts/build-tawasuyu-web.sh — build + deploy a un dir estático.
# -----------------------------------------------------------------------------
#  Compila el WASM, corre wasm-bindgen y rsync-ea todo lo necesario a $DEST
#  para que un server estático (Caddy, nginx, python -m http.server) lo sirva
#  como si fuera la raíz del repo:
#
#    $DEST/
#    ├── index.html, styles.css, pkg/      ← shell de la web
#    ├── README.md, PLAN.md                ← docs raíz del monorepo
#    ├── docs/                             ← SUITE.md, MODULES.md
#    ├── 00_unanchay/ 01_yachay/ 02_ruway/ ← sólo los .md, preserva paths
#    ├── 03_ukupacha/ shared/
#    └── web/tawasuyu-web/{README.md,md/*.md}
#
#  Uso:
#    ./scripts/build-tawasuyu-web.sh                  # → /var/www/tawasuyu
#    ./scripts/build-tawasuyu-web.sh /tmp/tawasuyu      # → /tmp/tawasuyu
#    ./scripts/build-tawasuyu-web.sh --dev /tmp/gw    # build dev, deploy a /tmp/gw
#
#  Caddyfile mínimo (apunta a $DEST):
#    tawasuyu.local {
#        root * /var/www/tawasuyu
#        encode gzip zstd
#        file_server
#        header /pkg/*.wasm Content-Type application/wasm
#    }
# =============================================================================

set -euo pipefail

# --- args ---
PROFILE=release
DEST=""
for arg in "$@"; do
  case "$arg" in
    --dev)     PROFILE=dev ;;
    --release) PROFILE=release ;;
    -h|--help) sed -n '2,35p' "$0"; exit 0 ;;
    *)         DEST="$arg" ;;
  esac
done
DEST="${DEST:-/var/www/tawasuyu}"

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WEB="$REPO/web/tawasuyu-web"

echo "==> repo:    $REPO"
echo "==> dest:    $DEST"
echo "==> profile: $PROFILE"

# --- pre-flight: wasm-bindgen-cli ---
if ! command -v wasm-bindgen >/dev/null; then
  VER=$(grep -A1 '^name = "wasm-bindgen"$' "$REPO/Cargo.lock" | grep '^version' | head -1 | cut -d'"' -f2)
  echo "ERROR: wasm-bindgen-cli no instalado." >&2
  echo "  cargo install wasm-bindgen-cli --version ${VER} --locked" >&2
  exit 1
fi

# --- pre-flight: target wasm32 ---
if command -v rustup >/dev/null; then
  if ! rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
    echo "==> instalando target wasm32-unknown-unknown"
    rustup target add wasm32-unknown-unknown
  fi
fi

# --- 1) cargo build ---
echo "==> cargo build ($PROFILE) -p tawasuyu-web"
cd "$REPO"
if [ "$PROFILE" = release ]; then
  cargo build --target wasm32-unknown-unknown --release -p tawasuyu-web
  WASM="$REPO/target/wasm32-unknown-unknown/release/tawasuyu_web.wasm"
else
  cargo build --target wasm32-unknown-unknown -p tawasuyu-web
  WASM="$REPO/target/wasm32-unknown-unknown/debug/tawasuyu_web.wasm"
fi

# --- 2) wasm-bindgen ---
echo "==> wasm-bindgen → $WEB/pkg/"
mkdir -p "$WEB/pkg"
wasm-bindgen --target web --out-dir "$WEB/pkg" "$WASM"

# --- 3) crear / verificar DEST ---
if [ ! -d "$DEST" ]; then
  if ! mkdir -p "$DEST" 2>/dev/null; then
    echo "ERROR: no se pudo crear $DEST. Probá:" >&2
    echo "  sudo mkdir -p $DEST && sudo chown $(id -u):$(id -g) $DEST" >&2
    exit 1
  fi
fi
if [ ! -w "$DEST" ]; then
  echo "ERROR: $DEST no es escribible por $(whoami). Probá:" >&2
  echo "  sudo chown -R $(id -u):$(id -g) $DEST" >&2
  exit 1
fi

# --- 4) limpiar .md viejos en DEST + dirs vacíos resultantes ---
if [ -d "$DEST" ] && [ -n "$(ls -A "$DEST" 2>/dev/null)" ]; then
  echo "==> limpiando .md viejos en $DEST"
  find "$DEST" -name '*.md' -type f -delete 2>/dev/null || true
  find "$DEST" -mindepth 1 -type d -empty -delete 2>/dev/null || true
fi

# --- 5) copia todos los .md del repo a DEST preservando estructura.
#       Usamos cp --parents (coreutils) — sin depender de rsync. ---
echo "==> copiando .md docs → $DEST/"
cd "$REPO"
find . \
  -path './target' -prune -o \
  -path './.git'   -prune -o \
  -path './node_modules' -prune -o \
  -type f \( -name '*.md' \) \
  ! -name '*.bak' ! -name '*.bak2' ! -name '*.bak3' \
  -print0 | while IFS= read -r -d '' f; do
    rel="${f#./}"
    target="$DEST/$rel"
    mkdir -p "$(dirname "$target")"
    cp -p "$f" "$target"
done

# --- 6) shell de la web en la raíz de DEST ---
echo "==> shell (index.html / styles.css / pkg/) → $DEST/"
install -m 0644 "$WEB/index.html" "$DEST/index.html"
install -m 0644 "$WEB/styles.css" "$DEST/styles.css"
rm -rf "$DEST/pkg"
cp -r "$WEB/pkg" "$DEST/pkg"

# --- 7) tamaño y reporte ---
SIZE=$(du -sh "$DEST" 2>/dev/null | awk '{print $1}')
MDS=$(find "$DEST" -name '*.md' -type f 2>/dev/null | wc -l)
echo
echo "==> listo."
echo "    $DEST  ($SIZE, $MDS docs)"
echo
echo "    Caddyfile sugerido:"
cat <<EOF

  tawasuyu.local {
      root * $DEST
      encode gzip zstd
      file_server
      header /pkg/*.wasm Content-Type application/wasm
  }

EOF
echo "    reload Caddy:  sudo systemctl reload caddy"
