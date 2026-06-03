#!/usr/bin/env bash
# wawa · demo web — pipeline completo.
#
# Pasos:
#   1) Compila cada app del userspace (hello_wasm, ...) a wasm32-unknown-unknown.
#   2) Copia los .wasm a web/wawa-web/assets/ — los embebe el binario host.
#   3) Compila web/wawa-web (el host wasmi del kernel) a wasm32-unknown-unknown.
#   4) Corre wasm-bindgen para generar pkg/{wawa_web.js, wawa_web_bg.wasm}
#      en este directorio.
#
# Para servir:
#   python -m http.server -d "$(dirname "$0")" 8765
#   xdg-open http://127.0.0.1:8765/

set -euo pipefail

aqui="$(cd "$(dirname "$0")" && pwd)"
raiz="$(cd "$aqui/../.." && pwd)"
apps_dir="$raiz/03_ukupacha/wawa/apps"
assets_dir="$raiz/web/wawa-web/assets"
pkg_dir="$aqui/pkg"

apps=( hello_wasm )

mkdir -p "$assets_dir" "$pkg_dir"

# 1 + 2 — userspace -> assets
for app in "${apps[@]}"; do
  echo "[wawa-demo] (1) compilando app $app…"
  (cd "$apps_dir/$app" && cargo build --target wasm32-unknown-unknown --release)
  origen="$apps_dir/$app/target/wasm32-unknown-unknown/release/${app}.wasm"
  if [[ ! -f "$origen" ]]; then
    origen="$raiz/target/wasm32-unknown-unknown/release/${app}.wasm"
  fi
  cp "$origen" "$assets_dir/${app}.wasm"
  echo "[wawa-demo]     -> $assets_dir/${app}.wasm ($(stat -c %s "$assets_dir/${app}.wasm") bytes)"
done

# 3 — host wasmi del kernel -> wasm32
echo "[wawa-demo] (2) compilando wawa-web (host)…"
(cd "$raiz" && cargo build -p wawa-web --target wasm32-unknown-unknown --release)

# 4 — wasm-bindgen -> pkg/
echo "[wawa-demo] (3) wasm-bindgen…"
WB="${WASM_BINDGEN:-wasm-bindgen}"
if ! command -v "$WB" >/dev/null 2>&1; then
  WB="$HOME/.cargo/bin/wasm-bindgen"
fi
"$WB" \
  "$raiz/target/wasm32-unknown-unknown/release/wawa_web.wasm" \
  --out-dir "$pkg_dir" \
  --target web \
  --no-typescript

echo "[wawa-demo] listo. Servir con:"
echo "  python -m http.server -d $aqui 8765"
