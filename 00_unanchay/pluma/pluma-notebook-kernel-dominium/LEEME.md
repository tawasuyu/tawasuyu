# pluma-notebook-kernel-dominium

> Kernel simulador para el notebook de [pluma](../README.md).

Celdas que corren [`dominium`](../../../01_yachay/dominium/README.md) con un seed + ticks fijos. Salidas: el snapshot final + visualización vía [`pineal-heatmap`](../../pineal/pineal-heatmap/README.md). Ideal para notebooks reproducibles de simulación (papers, talleres).

## API

```rust
use pluma_notebook_kernel_dominium::DominiumKernel;

let k = DominiumKernel::new();
let outputs = k.correr(&celda).await?;
```

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md)
- [`dominium-core`](../../../01_yachay/dominium/dominium-core/README.md), [`dominium-physics`](../../../01_yachay/dominium/dominium-physics/README.md)
