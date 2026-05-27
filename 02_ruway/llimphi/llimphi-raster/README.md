# llimphi-raster

> Rasterizer vello + cache de scenes de [llimphi](../README.md).

Wrapper sobre `vello`/`wgpu` con cache LRU de `Scene`s pre-renderizadas (para layouts estáticos que no cambian frame a frame). Manejo de antialiasing, clipping, blend modes. Trabaja contra `Surface` del HAL.

## Deps

- [`llimphi-hal`](../llimphi-hal/README.md)
- `vello`, `wgpu`, `peniko`, `kurbo`
