# rimay-verbo

Fachada para consumidores de embeddings. Esconde la convención del socket Unix de `verbo-daemon` y da una sola función que devuelve un `Provider` — el daemon real si está corriendo, un mock determinista si no.

```rust
use rimay_verbo::Provider;

let provider = rimay_verbo::conectar_o_mock(384).await;
let v = provider.embed("hola").await?;
```

## API

| Función | Comportamiento |
|---|---|
| `socket_por_defecto() -> PathBuf` | Misma convención que `verbo-daemon`: `$XDG_RUNTIME_DIR/verbo.sock`, fallback `/tmp/verbo-{uid}.sock`. |
| `conectar() -> Result<DaemonClient, EmbedError>` | Conecta al socket por defecto; error si no hay daemon. |
| `conectar_en(path) -> Result<DaemonClient, EmbedError>` | Conecta a un socket explícito (multi-daemon). |
| `conectar_o_mock(dim) -> Arc<dyn Provider>` | Socket por defecto; mock con `dim` dimensiones si no hay daemon. |
| `conectar_o_mock_en(path, dim) -> Arc<dyn Provider>` | Socket explícito; fallback a mock. |

`Provider`, `EmbeddingVector`, `ModelId`, `EmbedError`, `DaemonClient` y `MockProvider` se re-exportan; los consumidores dependen sólo de este crate.

## Consideraciones

- Un vector del daemon y otro del `MockProvider` van etiquetados con `ModelId`s distintos; `EmbeddingVector::cosine` se niega a comparar entre ellos — caer a mock al persistir embeddings a disco es seguro sólo si también persistís el `ModelId`.
- Para health checks sin invocar al modelo, tras conectar usá `DaemonClient::ping().await`.
