# llimphi-hal

> Surface abstraction of [llimphi](../README.md). Multi-platform.

`Surface` trait that abstracts window/framebuffer/canvas. Implementations: `winit` (Linux/macOS/Windows desktop), `android` (NDK), `wawa` (kernel framebuffer). The rest of the llimphi stack talks to `Surface`; moving Wayland → Wawa is swapping the HAL, not the scene tree.

## Deps

- `winit`, `raw-window-handle`
- `serde`, `wgpu` (re-export so widgets can paint_with)
