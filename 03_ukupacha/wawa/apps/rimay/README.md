# rimay (wawa app)

> Bare-metal mirror of the [rimay](../../../../00_unanchay/rimay/) verbo. Deterministic embedding + cosine, drawn on a 480×560 framebuffer inside wawa OS.

The host subdomain `00_unanchay/rimay/` runs a `verbo-daemon` over a Unix socket and serves real ONNX embeddings via fastembed. None of that fits in wawa: there is no socket, no `~/.cache/fastembed`, no ONNX. This app keeps the *contract* (Provider → vector → cosine) and discards the model: it runs the same FNV-1a + LCG mock that `rimay-verbo-mock` uses on the host, so identical text produces an identical vector and different text produces ~orthogonal noise.

Honest demo, not semantic similarity: cosine(A, A) = 1.000, cosine(A, B) ≈ 0 for distinct strings.

## Build

```sh
./scripts/build-rimay.sh        # cargo build → wasm-opt → wawa-kernel/assets/rimay.wasm
./scripts/build-rimay.sh --debug  # raw build, no wasm-opt, no consolidation
```

Output: `~3 KiB` sealed (well under the 10 KiB nominal manifest ceiling).

## Interaction

| Key | Action |
|---|---|
| `SPACE` (0x39) | Cycle to next text pair |
| `ENTER` (0x1C) | Reset to first pair |

Five pre-baked pairs cycle, including one identical pair (`RIMAY / RIMAY`) to make the contract visible: cosine = 1.000.

## Why no shared crate

`rimay-verbo-core` pulls `async_trait`, `tokio`, and `serde` with `std` features — none of them cross the wasmi sandbox. The FNV+LCG body is ~30 lines and was re-implemented inline in `#![no_std]`. If `rimay-verbo-core` ever becomes `no_std`-compatible behind a feature flag, this app should depend on it directly.

## Permisions

`permisos: 0` — no capabilities beyond the universal `sys_render_frame` + `sys_get_scancode`. No object graph, no network, no root.
