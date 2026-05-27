# shuma-daemon

> Session daemon of [shuma](../README.md).

Keeps sessions alive in background; clients (CLI, Llimphi, remote) connect/disconnect without losing state. Replaces `tmux`/`screen` in this monorepo.

## Usage

```sh
cargo run --release -p shuma-daemon -- --listen unix:/tmp/shuma.sock
```

## Deps

- [`shuma-core`](../sandbox/shuma-core/README.md), [`shuma-session`](../sandbox/shuma-session/README.md), [`shuma-protocol`](../sandbox/shuma-protocol/README.md)
