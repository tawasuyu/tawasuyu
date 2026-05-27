# puriy-app

> Binary of [puriy](../README.md). Auto-detects Llimphi vs headless target.

Minimal CLI. If it finds `WAYLAND_DISPLAY` or `DISPLAY`, opens the Llimphi window with [`puriy-llimphi`](../puriy-llimphi/README.md). Otherwise runs headless dumping the box tree — useful for CI and engine validation without a display.

## Usage

```sh
# auto-detect
cargo run --release -p puriy-app -- https://example.com

# force headless
cargo run --release -p puriy-app -- https://example.com --target headless
```

## Deps

- [`puriy-engine`](../puriy-engine/README.md), [`puriy-llimphi`](../puriy-llimphi/README.md)
