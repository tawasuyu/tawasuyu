# shuma-daemon

> Daemon de sesiones de [shuma](../README.md).

Mantiene sesiones vivas en background; clientes (CLI, Llimphi, remoto) se conectan/desconectan sin perder estado. Reemplaza `tmux`/`screen` en este monorepo.

## Uso

```sh
cargo run --release -p shuma-daemon -- --listen unix:/tmp/shuma.sock
```

## Deps

- [`shuma-core`](../shuma-core/README.md), [`shuma-session`](../shuma-session/README.md), [`shuma-protocol`](../shuma-protocol/README.md)
