# chasqui-nous-real

> Transport TCP/Unix binario de [chasqui](../README.md).

Implementa [`Nous`](../chasqui-nous/README.md) sobre `tokio` con framing length-prefix + serialización `postcard` (compacta, sin reflexión). Reconnect automático con backoff exponencial.

## Deps

- [`chasqui-nous`](../chasqui-nous/README.md), [`chasqui-core`](../chasqui-core/README.md)
- `tokio`, `postcard`
