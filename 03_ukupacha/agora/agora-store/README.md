# agora-store

> Local persistence of [agora](../README.md).

SQLite at `$XDG_DATA_HOME/agora/`. Simple schema: `messages`, `authors`, `subscriptions`. Optional encryption with key derived from user passphrase.

## Deps

- [`agora-core`](../agora-core/README.md), `rusqlite`
