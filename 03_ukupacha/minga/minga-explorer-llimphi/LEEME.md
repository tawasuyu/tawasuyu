# minga-explorer-llimphi

> UI: peers, content, tráfico de [minga](../README.md).

Tres pestañas: peers (estado de cada conexión), content (lo que tenés localmente), traffic (bandwidth/req-rate por peer). Útil para diagnosticar la red.

## Uso

```sh
cargo run --release -p minga-explorer-llimphi
```

## Deps

- Todos los `minga-*`
- [`llimphi-ui`](../../../02_ruway/llimphi/)
