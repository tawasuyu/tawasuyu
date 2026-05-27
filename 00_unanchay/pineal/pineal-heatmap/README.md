# pineal-heatmap

> Dense 2D heatmap canvas for [pineal](../README.md).

Takes an `Array2<f32>` matrix and paints it as a colored grid via a colormap (viridis, magma, plasma, ...). Supports NaN as transparent pixel. Useful for [`dominium`](../../../01_yachay/dominium/README.md) outputs, correlation maps, raster cosmology.

## API

```rust
use pineal_heatmap::{Heatmap, Colormap};

let hm = Heatmap::new(&data)
    .colormap(Colormap::Viridis)
    .range(0.0..1.0);
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
- `ndarray` optional for simulator integration
