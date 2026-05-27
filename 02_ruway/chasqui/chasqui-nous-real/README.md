# chasqui-nous-real

> Binary TCP/Unix transport of [chasqui](../README.md).

Implements [`Nous`](../chasqui-nous/README.md) over `tokio` with length-prefix framing + `postcard` serialization (compact, reflection-free). Automatic reconnect with exponential backoff.

## Deps

- [`chasqui-nous`](../chasqui-nous/README.md), [`chasqui-core`](../chasqui-core/README.md)
- `tokio`, `postcard`
