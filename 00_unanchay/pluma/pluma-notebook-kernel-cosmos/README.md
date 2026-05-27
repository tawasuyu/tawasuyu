# pluma-notebook-kernel-cosmos

> Astronomy kernel for the [pluma](../README.md) notebook.

Cells calling [`cosmos-sky`](../../../01_yachay/cosmos/cosmos-sky/README.md) with a DSL-like syntax: `observer(lat, lon, alt)`, `at(instant)`, `position("mars")`. Output is a serializable object (coords + magnitude + visibility) the notebook can format as a table or pass to another cell.

## API

```rust
use pluma_notebook_kernel_cosmos::CosmosKernel;

let k = CosmosKernel::new();
let outputs = k.correr(&celda).await?;
```

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md)
- [`cosmos-sky`](../../../01_yachay/cosmos/cosmos-sky/README.md)
