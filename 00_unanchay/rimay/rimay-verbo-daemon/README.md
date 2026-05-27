# rimay-verbo-daemon

> Daemon loop + IPC for [rimay](../README.md).

Service logic: starts a backend (`mock`, `fastembed`, ...), listens for requests via [`chasqui`](../../../02_ruway/chasqui/README.md) or Unix socket, worker pool for large batches, LRU in-memory cache of recent embeddings. Clean shutdown on `SIGTERM`.

## API

```rust
use rimay_verbo_daemon::{Daemon, Config};

let daemon = Daemon::new(Config::from_env()?);
daemon.run().await?;
```

## Deps

- [`rimay-verbo-core`](../rimay-verbo-core/README.md) — trait
- [`chasqui-core`](../../../02_ruway/chasqui/chasqui-core/README.md) — bus
- `tokio` for async IPC
