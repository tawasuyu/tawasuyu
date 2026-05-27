# minga-vfs

> VFS distribuido de [minga](../README.md).

Path lookup → DHT → chunk fetch. Cache local + cooperative fetching: si un peer cercano ya bajó el chunk, se lo pedimos a él en vez de re-bajar.

## Deps

- [`minga-core`](../minga-core/README.md), [`minga-store`](../minga-store/README.md), [`minga-p2p`](../minga-p2p/README.md)
