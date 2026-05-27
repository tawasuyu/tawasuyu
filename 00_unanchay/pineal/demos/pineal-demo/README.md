# pineal-demo

> Catalog demo gallery for [pineal](../README.md).

Binary that shows all backends (`cartesian`, `polar`, `mesh`, `treemap`, `flow`, `heatmap`, `umbrella`) in a Llimphi grid. Click on a demo opens the full view. Works as **visual regression**: if you break a backend, the demo screams.

## Usage

```sh
cargo run --release -p pineal-demo
```

## Deps

- All pineal backends
- [`llimphi-ui`](../../../02_ruway/llimphi/)
