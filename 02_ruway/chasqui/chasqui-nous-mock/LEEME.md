# chasqui-nous-mock

> Transport in-process para tests de [chasqui](../README.md).

Implementa [`Nous`](../chasqui-nous/README.md) usando canales `tokio::sync::mpsc` en memoria. Cero red. Tests deterministas.

## Deps

- [`chasqui-nous`](../chasqui-nous/README.md), [`chasqui-core`](../chasqui-core/README.md)
- `tokio`
