# rimay-verbo-daemon

> Loop del daemon + IPC para [rimay](../README.md).

Lógica del servicio: arranca un backend (`mock`, `fastembed`, ...), escucha pedidos via [`chasqui`](../../../02_ruway/chasqui/README.md) o socket Unix, pool de workers para batches grandes, cache LRU de embeddings recientes en memoria. Termina limpio al recibir `SIGTERM`.

## API

```rust
use rimay_verbo_daemon::{Daemon, Config};

let daemon = Daemon::new(Config::from_env()?);
daemon.run().await?;
```

## Deps

- [`rimay-verbo-core`](../rimay-verbo-core/README.md) — trait
- [`chasqui-core`](../../../02_ruway/chasqui/chasqui-core/README.md) — bus
- `tokio` para async IPC
