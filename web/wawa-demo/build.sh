#!/usr/bin/env bash
# wawa · demo web — compila las apps WASM y las copia a dist/.
#
# Cada app de 03_ukupacha/wawa/apps/ es un crate aislado (tiene su propio
# [workspace] interno), asi que se compilan una por una, fuera del workspace
# global. Esto evita que el kernel bare-metal entre en la build.

set -euo pipefail

aqui="$(cd "$(dirname "$0")" && pwd)"
raiz="$(cd "$aqui/../.." && pwd)"
apps_dir="$raiz/03_ukupacha/wawa/apps"
salida="$aqui/dist"

apps=( hello_wasm )  # ir sumando a medida que el host JS aprenda mas capacidades

mkdir -p "$salida"

for app in "${apps[@]}"; do
  echo "[wawa-demo] compilando $app…"
  (cd "$apps_dir/$app" && cargo build --target wasm32-unknown-unknown --release)

  # El target dir queda en la raiz del workspace global (al lado de los crates).
  origen="$raiz/target/wasm32-unknown-unknown/release/${app}.wasm"
  if [[ ! -f "$origen" ]]; then
    # Algunas configuraciones de cargo dejan el output dentro del propio crate.
    origen="$apps_dir/$app/target/wasm32-unknown-unknown/release/${app}.wasm"
  fi
  if [[ ! -f "$origen" ]]; then
    echo "[wawa-demo] error: no encontre el .wasm de $app" >&2
    exit 1
  fi
  cp "$origen" "$salida/${app}.wasm"
  echo "[wawa-demo]   -> $salida/${app}.wasm  ($(stat -c %s "$salida/${app}.wasm") bytes)"
done

echo "[wawa-demo] listo. Servir el demo con: python -m http.server -d $aqui 8080"
