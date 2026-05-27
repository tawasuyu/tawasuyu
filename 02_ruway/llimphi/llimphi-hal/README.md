# llimphi-hal

> Abstracción de superficie de [llimphi](../README.md). Multi-plataforma.

Trait `Surface` que abstrae window/framebuffer/canvas. Implementaciones: `winit` (Linux/macOS/Windows desktop), `android` (NDK), `wawa` (framebuffer del kernel). El resto del stack llimphi habla `Surface`; mover Wayland → Wawa es cambiar el HAL, no el árbol gráfico.

## Deps

- `winit`, `raw-window-handle`
- `serde`, `wgpu` (re-export para que widgets puedan paint_with)
