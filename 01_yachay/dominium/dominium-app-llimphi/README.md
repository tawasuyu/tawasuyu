# dominium-app-llimphi

> [dominium](../README.md) app: canvas + control panel + 11 Hz loop.

Binary that mounts the canvas ([`dominium-canvas-llimphi`](../dominium-canvas-llimphi/README.md)) + panel with sliders (seed, dt, brightness per layer, active concepts), play/pause/step, snapshot dump. Default loop at 11 Hz (configurable). Loads/saves config via `wawa-config-llimphi`.

## Usage

```sh
cargo run --release -p dominium-app-llimphi
```

## Deps

- [`dominium-core`](../dominium-core/README.md), [`dominium-physics`](../dominium-physics/README.md), [`dominium-canvas-llimphi`](../dominium-canvas-llimphi/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/)
- [`wawa-config-llimphi`](../../../shared/wawa-config-llimphi/)
