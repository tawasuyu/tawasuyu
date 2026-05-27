# cosmos-render

> Backend-agnostic render of [cosmos](../README.md): skymap + 3D.

Produces a `Vec<Shape>` (lines, circles, polygons, text) to represent the sky from an observer at an instant. Output consumable by Llimphi/vello, SVG (export), or WebGL2 (web). Zero graphics deps — just geometry.

## API

```rust
use cosmos_render::{skymap, View};

let shapes = skymap(obs, t, View::stereographic(zoom));
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-catalog`](../cosmos-catalog/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
- Zero graphics deps
