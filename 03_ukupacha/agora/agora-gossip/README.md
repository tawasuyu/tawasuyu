# agora-gossip

> Gossip protocol of [agora](../README.md) over [`chasqui`](../../../02_ruway/chasqui/README.md).

Periodic push/pull of new messages with known peers. Anti-flooding (per-peer token bucket), dedup by hash. Compatible with [`minga`](../../minga/README.md) for peer discovery.

## Deps

- [`agora-core`](../agora-core/README.md), [`chasqui-core`](../../../02_ruway/chasqui/chasqui-core/README.md)
