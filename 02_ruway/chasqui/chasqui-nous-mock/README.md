# chasqui-nous-mock

> In-process transport for tests of [chasqui](../README.md).

Implements [`Nous`](../chasqui-nous/README.md) using in-memory `tokio::sync::mpsc` channels. Zero network. Deterministic tests.

## Deps

- [`chasqui-nous`](../chasqui-nous/README.md), [`chasqui-core`](../chasqui-core/README.md)
- `tokio`
