# minga-vfs

> Distributed VFS of [minga](../README.md).

Path lookup → DHT → chunk fetch. Local cache + cooperative fetching: if a nearby peer already has the chunk, request it from them instead of re-downloading.

## Deps

- [`minga-core`](../minga-core/README.md), [`minga-store`](../minga-store/README.md), [`minga-p2p`](../minga-p2p/README.md)
