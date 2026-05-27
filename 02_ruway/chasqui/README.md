# chasqui

> `chasqui` (quechua: *mensajero del camino del inca*). Broker de mensajería + bus tipado.

Sistema nervioso del monorepo. Apps publican y se suscriben a topics tipados; el broker rutea y persiste. Backend `nous` con dos implementaciones: `mock` (in-process para tests) y `real` (TCP + binary). Cada mensaje lleva su schema, fail-closed si el receptor no lo conoce.

## Instalación

```sh
# arrancar el broker
cargo run --release -p chasqui-broker

# explorer (ver topics + mensajes en vivo)
cargo run --release -p chasqui-broker-explorer-llimphi
cargo run --release -p chasqui-explorer-llimphi
```

## Compatibilidad

- **Linux / macOS / Windows** — broker + clientes en Rust nativo.
- **Wawa** — broker corre como app del kernel (`apps/`).
- TCP localhost por default; sockets Unix opcionales.

## Crates

| Crate | Rol |
|---|---|
| [`chasqui-core`](chasqui-core/README.md) | Tipos: Topic, Message, Schema, Subscription. |
| [`chasqui-broker`](chasqui-broker/README.md) | Binario del broker. |
| [`chasqui-nous`](chasqui-nous/README.md) | Trait del transport. |
| [`chasqui-nous-mock`](chasqui-nous-mock/README.md) | Transport in-process para tests. |
| [`chasqui-nous-real`](chasqui-nous-real/README.md) | Transport TCP/Unix binario. |
| [`chasqui-card`](chasqui-card/README.md) | Card escritorio (estado del broker). |
| [`chasqui-broker-explorer-llimphi`](chasqui-broker-explorer-llimphi/README.md) | UI: topics + suscriptores activos. |
| [`chasqui-explorer-llimphi`](chasqui-explorer-llimphi/README.md) | UI: log de mensajes en vivo. |

## Consideraciones

- **Schema-first.** Sin schema declarado, ningún mensaje pasa.
- **Persistencia opt-in** por topic; los topics efímeros viven sólo en memoria.
- **No es Kafka.** Diseñado para el monorepo, no para volumen de producción interplanetaria.
