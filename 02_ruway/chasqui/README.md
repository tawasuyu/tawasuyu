# chasqui

> `chasqui` (Quechua: *messenger of the Inca road*). Message broker + typed bus.

Nervous system of the monorepo. Apps publish and subscribe to typed topics; the broker routes and persists. `nous` backend with two impls: `mock` (in-process for tests) and `real` (binary TCP). Every message carries its schema, fail-closed if the receiver doesn't know it.

## Install

```sh
cargo run --release -p chasqui-broker
cargo run --release -p chasqui-broker-explorer-llimphi
cargo run --release -p chasqui-explorer-llimphi
```

## Compatibility

- **Linux / macOS / Windows** — broker + clients in native Rust.
- **Wawa** — broker runs as a kernel app.
- TCP localhost by default; Unix sockets optional.

## Crates

| Crate | Role |
|---|---|
| [`chasqui-core`](chasqui-core/README.md) | Topic, Message, Schema, Subscription. |
| [`chasqui-broker`](chasqui-broker/README.md) | Broker binary. |
| [`chasqui-nous`](chasqui-nous/README.md) | Transport trait. |
| [`chasqui-nous-mock`](chasqui-nous-mock/README.md) | In-process transport. |
| [`chasqui-nous-real`](chasqui-nous-real/README.md) | Binary TCP/Unix transport. |
| [`chasqui-card`](chasqui-card/README.md) | Desktop card. |
| [`chasqui-broker-explorer-llimphi`](chasqui-broker-explorer-llimphi/README.md) | Topics + active subscribers UI. |
| [`chasqui-explorer-llimphi`](chasqui-explorer-llimphi/README.md) | Live message log UI. |

## Considerations

- **Schema-first.** No schema declared, no message through.
- **Persistence opt-in** per topic; ephemeral topics live in memory only.
- **Not Kafka.** Designed for the monorepo, not interplanetary production volume.
