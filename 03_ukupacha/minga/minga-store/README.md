# minga-store

> Local storage of [minga](../README.md).

Content-addressed chunks at `$XDG_DATA_HOME/minga/chunks/`. Verifies hash on read. Configurable LRU eviction. Same shape as [`wawa-fs`](../../wawa/wawa-fs/README.md) — direct interop.

## Deps

- [`minga-core`](../minga-core/README.md), `blake3`
