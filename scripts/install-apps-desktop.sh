#!/usr/bin/env bash
# Instala los lanzadores .desktop + íconos SVG de las apps de la suite.
#
# Hasta ahora las apps existían como binarios pero eran invisibles al
# escritorio (sin .desktop ni íconos). Este script compila y corre
# `tawasuyu-apps-desktop`, que genera el layout freedesktop a partir del
# catálogo de app-bus y los AppIcon de llimphi-icons.
#
#   scripts/install-apps-desktop.sh            # instala al usuario (~/.local/share)
#   scripts/install-apps-desktop.sh --prefix DIR  # staging para empaquetado
#
# Requiere que los binarios de las apps estén en el PATH para que los
# lanzadores funcionen (los instala `install-tawasuyu.sh`).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "· compilando tawasuyu-apps-desktop…"
cargo build --release -p tawasuyu-apps-desktop

echo "· generando .desktop + íconos…"
./target/release/tawasuyu-apps-desktop "$@"

# Refresca cachés de íconos si las herramientas están (no fatal si faltan).
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$DATA/icons/hicolor" 2>/dev/null || true
fi

echo "· listo. Las apps de tawasuyu ya aparecen en el menú/lanzador del escritorio."
