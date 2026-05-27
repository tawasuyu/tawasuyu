# nahual

> `nahual` (Nahuatl: *companion spirit*). Everyday viewers over Llimphi.

Minimal set of viewers a desktop user expects: file shell, text viewer, image viewer. Built on the same UI framework; share preferences with the rest of the monorepo via `wawa-config`. Plus a meta-runtime to define new viewers via schema without writing Rust from scratch.

## Install

```sh
cargo run --release -p nahual-shell-llimphi
cargo run --release -p nahual-file-explorer-llimphi
cargo run --release -p nahual-text-viewer-llimphi
cargo run --release -p nahual-image-viewer-llimphi
```

## Compatibility

- **Linux / macOS / Windows** — native Llimphi UI.
- **Wawa** — viewers compile inside the kernel; file explorer speaks `wawa-fs`.

Crates listed in [README.md](README.md).

## Considerations

- **Viewers, not editors.** Edit the file → `nada`. Edit the image → `pineal` or external.
- The meta-runtime lets you **define a viewer in JSON** and get a Llimphi app without code.
