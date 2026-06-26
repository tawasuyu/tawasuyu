# Assets de ejemplo de mirada

## `wallpaper-ejemplo-animado.mp4` — wallpaper en video

Un clip sobrio (1280×720, 24 fps, ~24 s, 94 KB) con un **gradiente que deriva
muy lento** — el «movimiento levísimo» de un fondo vivo, sin ruido. Generado con
ffmpeg (filtro `gradients`), sin audio.

Para usarlo como fondo del escritorio, en `~/.config/mirada/config.ron`:

```ron
wallpaper_source: "video",
wallpaper_path: "/ruta/al/repo/02_ruway/mirada/assets/wallpaper-ejemplo-animado.mp4",
wallpaper_video_fps: 24,   // 0 = el nativo del archivo; bajalo para abaratar
```

…o desde **wawa-panel** → sección **Fondo**: «Fuente» = *Video (animado)*, y
apuntá «Imagen / video de fondo» al archivo.

El compositor decodifica el archivo con `foreign-av` (ffmpeg) en un hilo aparte,
lo reproduce en **loop**, y **pausa** la decodificación cuando una ventana a
pantalla completa tapa el fondo o la sesión está en otra VT (no se ve → no se
gasta). Acepta cualquier formato que ffmpeg lea (mp4/webm/gif/…).

> Regenerar (o hacer el tuyo) — un gradiente calmo de 24 s:
> ```sh
> ffmpeg -f lavfi -i "gradients=s=1280x720:c0=0x0a0e22:c1=0x10243f:c2=0x231a3e:c3=0x0c1330:speed=0.0018:duration=24:r=24" \
>        -t 24 -c:v libx264 -pix_fmt yuv420p -crf 30 -preset slow -movflags +faststart salida.mp4
> ```
> El loop hace un corte al rebobinar; para un bucle sin costura, exportá un clip
> ya pensado para enganchar (fin = principio).
