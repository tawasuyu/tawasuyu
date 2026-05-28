# rimay-verbo

Façade for embedding consumers. Hides the Unix-socket convention of `verbo-daemon` and gives a single function that returns a `Provider` — real daemon if it is running, deterministic mock otherwise.

```rust
use rimay_verbo::Provider;

let provider = rimay_verbo::conectar_o_mock(384).await;
let v = provider.embed("hola").await?;
```

## API

| Function | Behavior |
|---|---|
| `socket_por_defecto() -> PathBuf` | Same convention as `verbo-daemon`: `$XDG_RUNTIME_DIR/verbo.sock`, fallback `/tmp/verbo-{uid}.sock`. |
| `conectar() -> Result<DaemonClient, EmbedError>` | Connect to the default socket; error if no daemon. |
| `conectar_en(path) -> Result<DaemonClient, EmbedError>` | Connect to an explicit socket (multi-daemon scenarios). |
| `conectar_o_mock(dim) -> Arc<dyn Provider>` | Default socket; mock with `dim` dimensions if no daemon. |
| `conectar_o_mock_en(path, dim) -> Arc<dyn Provider>` | Explicit socket; mock fallback. |

`Provider`, `EmbeddingVector`, `ModelId`, `EmbedError`, `DaemonClient`, and `MockProvider` are re-exported so consumers only depend on this crate.

## Considerations

- A vector from the daemon and one from `MockProvider` are tagged with different `ModelId`s; `EmbeddingVector::cosine` refuses to compare across them — falling back to mock when persisting embeddings to disk is safe only if you also persist the `ModelId`.
- For health checks without invoking the model, after connecting use `DaemonClient::ping().await`.
