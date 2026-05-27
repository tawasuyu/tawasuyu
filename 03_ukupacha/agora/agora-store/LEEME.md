# agora-store

> Persistencia local de [agora](../README.md).

SQLite en `$XDG_DATA_HOME/agora/`. Schema simple: `messages`, `authors`, `subscriptions`. Cifrado opcional con clave derivada de la passphrase del usuario.

## Deps

- [`agora-core`](../agora-core/README.md), `rusqlite`
