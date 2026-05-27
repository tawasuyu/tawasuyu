# dominium-app-llimphi

> App de [dominium](../README.md): canvas + panel de control + loop 11 Hz.

Binario que monta el canvas ([`dominium-canvas-llimphi`](../dominium-canvas-llimphi/README.md)) + panel con sliders (seed, dt, brightness por capa, conceptos activos), play/pause/step, snapshot dump. Loop a 11 Hz por default (configurable). Carga/guarda config via `wawa-config-llimphi`.

## Uso

```sh
cargo run --release -p dominium-app-llimphi
```

## Deps

- [`dominium-core`](../dominium-core/README.md), [`dominium-physics`](../dominium-physics/README.md), [`dominium-canvas-llimphi`](../dominium-canvas-llimphi/README.md)
- [`llimphi-ui`](../../../02_ruway/llimphi/)
- [`wawa-config-llimphi`](../../../shared/wawa-config-llimphi/)
