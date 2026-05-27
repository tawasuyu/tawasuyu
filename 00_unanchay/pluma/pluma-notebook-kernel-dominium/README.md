# pluma-notebook-kernel-dominium

> Simulator kernel for the [pluma](../README.md) notebook.

Cells running [`dominium`](../../../01_yachay/dominium/README.md) with a fixed seed + ticks. Outputs: final snapshot + visualization via [`pineal-heatmap`](../../pineal/pineal-heatmap/README.md). Ideal for reproducible simulation notebooks (papers, workshops).

## API

```rust
use pluma_notebook_kernel_dominium::DominiumKernel;

let k = DominiumKernel::new();
let outputs = k.correr(&celda).await?;
```

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md)
- [`dominium-core`](../../../01_yachay/dominium/dominium-core/README.md), [`dominium-physics`](../../../01_yachay/dominium/dominium-physics/README.md)
