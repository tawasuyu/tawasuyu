# chasqui-explorer-llimphi

> UI Llimphi: log de mensajes en vivo del broker de [chasqui](../README.md).

Filtros por topic + autor + schema; pause/resume del stream; inspección del mensaje en detalle. Útil para debug de protocolos entre apps.

## Uso

```sh
cargo run --release -p chasqui-explorer-llimphi
```

## Deps

- [`chasqui-core`](../chasqui-core/README.md), [`chasqui-nous-real`](../chasqui-nous-real/README.md)
- [`llimphi-ui`](../../llimphi/) + widgets `list`, `text-area`
