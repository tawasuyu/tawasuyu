# rimay

> `rimay` (Quechua: *to speak, word*). Language: embeddings, verbs, what *wants to say* something.

Local NLP/embeddings service that any monorepo app queries when it needs "how similar are these two texts" or "give me the embedding for this" without going out to the network. The daemon `rimay-verbo-daemon` runs as a service; clients talk over the typed bus ([`chasqui`](../../02_ruway/chasqui/README.md)).

## Install

```sh
# start the daemon
cargo run --release -p rimay-verbo-daemon-bin

# run in mock mode (no GPU, no model downloads)
RIMAY_BACKEND=mock cargo run --release -p rimay-verbo-daemon-bin
```

## Compatibility

- **Linux** — `fastembed` backend with ONNX runtime (CPU or GPU).
- **macOS / Windows / Wawa** — `mock` backend or `fastembed` CPU.
- Models cached in `$XDG_CACHE_HOME/rimay/`.

## Crates

| Crate | Role |
|---|---|
| [`rimay-verbo-core`](rimay-verbo-core/README.md) | `Verbo` trait + public types. |
| [`rimay-verbo-daemon`](rimay-verbo-daemon/README.md) | Daemon loop + IPC. |
| [`rimay-verbo-daemon-bin`](rimay-verbo-daemon-bin/README.md) | Daemon binary. |
| [`rimay-verbo-fastembed`](rimay-verbo-fastembed/README.md) | ONNX backend (BGE, MiniLM). |
| [`rimay-verbo-mock`](rimay-verbo-mock/README.md) | Deterministic mock backend for tests. |

## Considerations

- The daemon does NOT auto-download models without user permission: first-run asks for confirmation + path.
- [`pluma-llm`](../pluma/pluma-llm/README.md) and `rimay-verbo` are orthogonal: the former generates text, the latter *understands* it.
- Wawa-compatible: the daemon also builds to WASM and lives in `apps/`.
