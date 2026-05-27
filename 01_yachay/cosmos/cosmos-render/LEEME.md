# cosmos-render

> Render agnóstico de [cosmos](../README.md): skymap + 3D.

Producir un `Vec<Shape>` (líneas, círculos, polígonos, texto) para representar el cielo desde un observador en un instante. Salida consumible por Llimphi/vello, SVG (export), o WebGL2 (web). Cero deps gráficas — sólo geometría.

## API

```rust
use cosmos_render::{skymap, View};

let shapes = skymap(obs, t, View::stereographic(zoom));
```

## Deps

- [`cosmos-core`](../cosmos-core/README.md), [`cosmos-catalog`](../cosmos-catalog/README.md), [`cosmos-pointing`](../cosmos-pointing/README.md)
- Cero deps gráficas
