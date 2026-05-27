# agora-gossip

> Gossip protocol de [agora](../README.md) sobre [`chasqui`](../../../02_ruway/chasqui/README.md).

Push/pull periódico de mensajes nuevos con peers conocidos. Anti-flooding (token bucket por peer), dedup por hash. Compatible con [`minga`](../../minga/README.md) para peer discovery.

## Deps

- [`agora-core`](../agora-core/README.md), [`chasqui-core`](../../../02_ruway/chasqui/chasqui-core/README.md)
