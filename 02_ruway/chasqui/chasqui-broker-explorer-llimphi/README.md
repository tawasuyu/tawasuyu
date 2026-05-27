# chasqui-broker-explorer-llimphi

> UI Llimphi: topics + suscriptores activos del broker de [chasqui](../README.md).

Lista los topics actuales, cuántos suscriptores tiene cada uno, throughput por topic, persistencia activa o no. Útil para diagnosticar el sistema.

## Uso

```sh
cargo run --release -p chasqui-broker-explorer-llimphi
```

## Deps

- [`chasqui-core`](../chasqui-core/README.md), [`chasqui-nous-real`](../chasqui-nous-real/README.md)
- [`llimphi-ui`](../../llimphi/) + widgets `list`, `tabs`
