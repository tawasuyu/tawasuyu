# nahual-image-viewer-llimphi

> Image viewer (PNG/JPEG/WebP) of [nahual](../README.md).

Pan/zoom, fit-to-window, magnifier mode, EXIF info when available. Supports GIF/APNG animation.

## Usage

```sh
cargo run --release -p nahual-image-viewer-llimphi -- path/to/image.png
```

## Deps

- `image` crate, [`llimphi-ui`](../../llimphi/)
