# minga-store

> Storage local de [minga](../README.md).

Chunks content-addressed en `$XDG_DATA_HOME/minga/chunks/`. Verifica hash al leer. Eviction LRU configurable. Misma forma que [`wawa-fs`](../../wawa/wawa-fs/README.md) — interoperabilidad directa.

## Deps

- [`minga-core`](../minga-core/README.md), `blake3`
