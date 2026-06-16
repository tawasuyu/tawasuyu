#!/usr/bin/env bash
# Regenera el showreel del motor Llimphi: frames PNG (example headless) → MP4 + GIF.
# Eye-candy abstracto sobre el tema firma Tawa, loop ~10 s. Determinista (t∈[0,1]).
#
#   scripts/showreel.sh                 # 300 frames @ 1600x900, 30fps
#   scripts/showreel.sh 360 1920 1080   # N frames W H a medida
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

N="${1:-300}"
W="${2:-1600}"
H="${3:-900}"
FPS=30

FRAMES_DIR="$ROOT/showreel_frames"
OUT_DIR="$ROOT/dist/showreel"
MP4="$OUT_DIR/llimphi_showreel.mp4"
GIF="$OUT_DIR/llimphi_showreel.gif"

mkdir -p "$OUT_DIR"
rm -rf "$FRAMES_DIR"

echo "==> Renderizando $N frames a ${W}x${H} (headless, vello → wgpu → PNG)…"
cargo run -p llimphi-compositor --example showreel --release -- "$FRAMES_DIR" "$N" "$W" "$H"

echo "==> Encodeando MP4 ($MP4)…"
ffmpeg -y -framerate "$FPS" -i "$FRAMES_DIR/frame_%04d.png" \
  -c:v libx264 -crf 20 -pix_fmt yuv420p "$MP4"

echo "==> Encodeando GIF loop ($GIF, escalado a 1000px de ancho)…"
ffmpeg -y -framerate "$FPS" -i "$FRAMES_DIR/frame_%04d.png" \
  -vf "scale=1000:-1:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=bayer:bayer_scale=3" \
  -loop 0 "$GIF"

echo "==> Listo:"
ls -lh "$MP4" "$GIF"
