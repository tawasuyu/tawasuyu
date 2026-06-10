# rimay

> `rimay` (Quechua: *to speak, word*). Language: embeddings, verbs, what *wants to say* something.

Local NLP/embeddings service that any monorepo app queries when it needs "how similar are these two texts" or "give me the embedding for this" without going out to the network. The `verbo-daemon` binary loads a model once and exposes it over a Unix socket; clients consume it via `rimay_verbo_daemon::DaemonClient`, which itself implements the `Provider` trait, so consumer code is identical to running the backend in-process.

## Install

```sh
# start the daemon with the default real backend (fastembed / multilingual-e5-small)
# (--allow-download authorizes the one-time ONNX model download)
cargo run --release -p rimay-verbo-daemon-bin -- --provider fastembed --allow-download

# or in mock mode (no model download, deterministic vectors, fine for CI)
cargo run --release -p rimay-verbo-daemon-bin -- --provider mock
```

The socket defaults to `$XDG_RUNTIME_DIR/verbo.sock`, with `/tmp/verbo-{uid}.sock` as fallback. Pass `--socket /path` to override (multi-daemon = one socket per model).

## Consuming from a crate

The thin façade [`rimay-verbo`](rimay-verbo/) hides socket discovery and gives a one-liner with mock fallback:

```rust
use rimay_verbo::Provider;

let provider = rimay_verbo::conectar_o_mock(384).await;
let v = provider.embed("hola").await?;
```

Consumers that want to fail loud when the daemon is missing use `rimay_verbo::conectar()` instead. For health checks without invoking the model, `DaemonClient::ping().await`.

## Compatibility

- **Linux** — `fastembed` backend with ONNX runtime on CPU; downloads the model to `~/.cache/fastembed` the first time.
- **macOS / Windows** — `mock` backend always; `fastembed` if its ONNX dependency builds on the host.
- **Wawa** — pending: the daemon does not yet build to WASM.

## Crates

| Crate | Role |
|---|---|
| [`rimay-verbo`](rimay-verbo/) | One-line façade (`conectar_o_mock`, socket discovery, re-exports). |
| [`rimay-verbo-core`](rimay-verbo-core/) | `Provider` trait + public types (`ModelId`, `EmbeddingVector`, `EmbedError`). |
| [`rimay-verbo-daemon`](rimay-verbo-daemon/) | Daemon loop + Unix-socket IPC (postcard frames + 1-retry reconnect). |
| [`rimay-verbo-daemon-bin`](rimay-verbo-daemon-bin/) | Daemon binary (`verbo-daemon`), with SIGINT/SIGTERM graceful shutdown. |
| [`rimay-verbo-fastembed`](rimay-verbo-fastembed/) | ONNX backend (`multilingual-e5-small` by default; E5/BGE catalog). |
| [`rimay-verbo-mock`](rimay-verbo-mock/) | Deterministic mock backend (FNV + LCG; no downloads, no GPU). |

## Considerations

- The `fastembed` backend only downloads the ONNX model with explicit opt-in: env var `RIMAY_VERBO_ALLOW_DOWNLOAD=1` (or the daemon's `--allow-download` flag, which sets it). Without it, the provider returns an error with the opt-in recipe instead of silently pulling 100+ MB. The cache lives in `~/.cache/fastembed`; delete it to force a re-download.
- [`pluma-llm`](../pluma/pluma-llm/) and `rimay-verbo` are orthogonal: the former generates text, the latter *understands* it.
- Vectors are tagged with their `ModelId`; `EmbeddingVector::cosine` refuses to compare across models, so a `mock` vector and a `fastembed` vector are never silently mixed.
- [`shared/rimay-localize`](../../shared/rimay-localize/) — the desktop i18n layer (fluent catalogs, es/en/qu) — carries the rimay name as its cross-cutting localization utility; it is not part of these subcrates.
