# llimphi · demo prezi

Prezi web autocontenido para mostrar llimphi: arquitectura, bucle Elm,
catálogo y apps reales corriendo encima. Usa pantallazos headless de la
suite (cosmos, pluma, nada, takiy, tullpu, supay, dominium, nahual, shuma…).

## Ver

```bash
# Local — abrí con cualquier navegador moderno:
xdg-open 02_ruway/llimphi/demo/index.html

# O servilo (recomendado para que las imágenes carguen sin file://):
python3 -m http.server 8000 -d 02_ruway/llimphi/demo
# → http://localhost:8000
```

## Controles

| tecla        | acción                |
|--------------|-----------------------|
| espacio      | play / pause          |
| → · pgdown   | siguiente             |
| ← · pgup     | anterior              |
| home / end   | primer / último stop  |
| r            | volver al inicio      |
| click        | siguiente             |

Auto-advance a 6s por stop. 17 stops ≈ 100 s de recorrido.

## Grabar como video

```bash
# Con ffmpeg + x11grab (X11):
ffmpeg -video_size 1920x1080 -framerate 30 -f x11grab -i :0.0+0,0 \
  -c:v libx264 -preset slow -crf 18 -pix_fmt yuv420p demo-llimphi.mp4

# Headless con chromium-headless + ffmpeg (sin display):
chromium --headless --window-size=1920,1080 \
  --screen-info='{1920x1080}' \
  --virtual-time-budget=100000 \
  --screenshot-format=png \
  file://$(pwd)/02_ruway/llimphi/demo/index.html
```

Para una grabación limpia: abrir en pantalla completa (F11), esperar a que
arranque, y dejar que el auto-advance haga el resto.

## Estructura

```
demo/
├── index.html        ← prezi: HTML + CSS + JS, sin dependencias
├── assets/           ← pantallazos headless de las apps
│   ├── llimphi.png   ← llimphi-gallery (kit de elegancia)
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

Los pantallazos viven en `<dominio>/pantallazo.png` y se copian acá; cada
uno se genera con `cargo run -p <dominio>-app-llimphi --example pantallazo_<dominio> --release`.
