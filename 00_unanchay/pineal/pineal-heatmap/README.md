# pineal-heatmap

> Canvas de heatmap denso 2D para [pineal](../README.md).

Toma una matriz `Array2<f32>` y la pinta como una grilla coloreada según un colormap (viridis, magma, plasma, ...). Soporta NaN como pixel transparente. Útil para resultados de [`dominium`](../../../01_yachay/dominium/README.md), correlation maps, raster cosmology.

## API

```rust
use pineal_heatmap::{Heatmap, Colormap};

let hm = Heatmap::new(&data)
    .colormap(Colormap::Viridis)
    .range(0.0..1.0);
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
- `ndarray` opcional para integración con simuladores
