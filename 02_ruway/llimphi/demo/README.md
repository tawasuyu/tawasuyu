# llimphi · demo prezi

Self-contained web prezi to showcase llimphi: architecture, Elm loop, catalog
and real apps running on top. It uses headless screenshots of the suite (cosmos,
pluma, nada, takiy, tullpu, supay, dominium, nahual, shuma…).

## View

```bash
# Local — open with any modern browser:
xdg-open 02_ruway/llimphi/demo/index.html

# Or serve it (recommended so the images load without file://):
python3 -m http.server 8000 -d 02_ruway/llimphi/demo
# → http://localhost:8000
```

## Controls

| key          | action                |
|--------------|-----------------------|
| space        | play / pause          |
| → · pgdown   | next                  |
| ← · pgup     | previous              |
| home / end   | first / last stop     |
| r            | back to start         |
| click        | next                  |

Auto-advance at 6s per stop. 17 stops ≈ 100 s of run time.

## Record as video

```bash
# With ffmpeg + x11grab (X11):
ffmpeg -video_size 1920x1080 -framerate 30 -f x11grab -i :0.0+0,0 \
  -c:v libx264 -preset slow -crf 18 -pix_fmt yuv420p demo-llimphi.mp4

# Headless with chromium-headless + ffmpeg (no display):
chromium --headless --window-size=1920,1080 \
  --screen-info='{1920x1080}' \
  --virtual-time-budget=100000 \
  --screenshot-format=png \
  file://$(pwd)/02_ruway/llimphi/demo/index.html
```

For a clean recording: open fullscreen (F11), wait for it to start, and let the
auto-advance do the rest.

## Structure

```
demo/
├── index.html        ← prezi: HTML + CSS + JS, no dependencies
├── assets/           ← headless screenshots of the apps
│   ├── llimphi.png   ← llimphi-gallery (elegance kit)
│   ├── cosmos.png
│   ├── pluma.png
│   ├── nada.png
│   ├── takiy.png
│   ├── tullpu.png
│   ├── supay.png
│   ├── dominium.png
│   ├── nahual.png
│   ├── shuma.png
│   ├── chaka.png
│   └── pineal.png
└── README.md
```

The screenshots live in `<domain>/pantallazo.png` and are copied here; each one
is generated with `cargo run -p <domain>-app-llimphi --example pantallazo_<domain> --release`.
