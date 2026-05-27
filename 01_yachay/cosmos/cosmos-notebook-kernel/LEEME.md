# pluma-notebook-kernel-cosmos

> Kernel astronomía para el notebook de [pluma](../README.md).

Celdas que llaman a [`cosmos-sky`](../../../01_yachay/cosmos/cosmos-sky/README.md) con sintaxis tipo DSL: `observer(lat, lon, alt)`, `at(instant)`, `position("mars")`. La salida es un objeto serializable (coords + magnitude + visibilidad) que el notebook puede formatear como tabla o pasar a otra celda.

## API

```rust
use pluma_notebook_kernel_cosmos::CosmosKernel;

let k = CosmosKernel::new();
let outputs = k.correr(&celda).await?;
```

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md)
- [`cosmos-sky`](../../../01_yachay/cosmos/cosmos-sky/README.md)
