#!/usr/bin/env bash
# build-brand.sh — rasteriza los assets de marca (SVG → PNG) de docs/brand/.
#
# La fuente de verdad de la marca es vectorial (docs/brand/*.svg); los PNG son
# derivados reproducibles. El wallpaper se genera en las resoluciones más
# comunes de escritorio. Requiere `rsvg-convert` (paquete librsvg).
#
# Uso:  scripts/build-brand.sh
set -euo pipefail
cd "$(dirname "$0")/.."
BR=docs/brand

command -v rsvg-convert >/dev/null || { echo "falta rsvg-convert (librsvg)"; exit 1; }

# Wallpaper en 16:9 (la geometría es responsiva al viewBox 2560x1440).
rsvg-convert -w 2560 -h 1440 "$BR/wallpaper.svg" -o "$BR/wallpaper-2560x1440.png"
rsvg-convert -w 3840 -h 2160 "$BR/wallpaper.svg" -o "$BR/wallpaper-3840x2160.png"
rsvg-convert -w 1920 -h 1080 "$BR/wallpaper.svg" -o "$BR/wallpaper-1920x1080.png"

# Marca suelta (para README/web/favicon).
rsvg-convert -w 512 -h 512 "$BR/chakana.svg" -o "$BR/chakana-512.png"

echo "marca rasterizada en $BR/:"
ls -1 "$BR"/*.png
