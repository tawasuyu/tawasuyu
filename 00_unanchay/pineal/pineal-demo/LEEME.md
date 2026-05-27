# pineal-demo

> Demo gallery del catálogo de [pineal](../README.md).

Binario que muestra todos los backends (`cartesian`, `polar`, `mesh`, `treemap`, `flow`, `heatmap`, `umbrella`) en una grid Llimphi. Click en una demo abre la vista completa. Sirve como **regression visual**: si rompés un backend, la demo lo grita.

## Uso

```sh
cargo run --release -p pineal-demo
```

## Deps

- Todos los backends de pineal
- [`llimphi-ui`](../../../02_ruway/llimphi/)
