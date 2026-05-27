# llimphi-raster

> Vello rasterizer + scene cache of [llimphi](../README.md).

Wrapper over `vello`/`wgpu` with LRU cache of pre-rendered `Scene`s (for static layouts that don't change frame to frame). Antialiasing, clipping, blend modes. Works against the HAL's `Surface`.

## Deps

- [`llimphi-hal`](../llimphi-hal/README.md)
- `vello`, `wgpu`, `peniko`, `kurbo`
