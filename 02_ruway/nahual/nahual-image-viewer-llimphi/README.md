# nahual-image-viewer-llimphi

> Viewer de imagen (PNG/JPEG/WebP) de [nahual](../README.md).

Pan/zoom, fit-to-window, modo lupa, info EXIF cuando hay. Soporta animación GIF/APNG.

## Uso

```sh
cargo run --release -p nahual-image-viewer-llimphi -- path/to/image.png
```

## Deps

- `image` crate, [`llimphi-ui`](../../llimphi/)
