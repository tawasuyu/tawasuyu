# rimay

> `rimay` (quechua: *hablar, decir, palabra*). Lenguaje: embeddings, verbos, lo que *quiere decir* algo.

Servicio local de NLP/embeddings que cualquier app del monorepo consulta cuando necesita "qué tan parecidos son estos dos textos" o "dame el embedding de esto" sin salir a la red. El daemon `rimay-verbo-daemon` corre como servicio; los clientes hablan por bus tipado (`chasqui`).

## Instalación

```sh
# arrancar el daemon
cargo run --release -p rimay-verbo-daemon-bin

# correr en modo mock (sin GPU, sin descargar modelos)
RIMAY_BACKEND=mock cargo run --release -p rimay-verbo-daemon-bin
```

## Compatibilidad

- **Linux** — backend `fastembed` con ONNX runtime (CPU o GPU).
- **macOS / Windows / Wawa** — backend `mock` o `fastembed` CPU.
- Modelos cacheados en `$XDG_CACHE_HOME/rimay/`.

## Crates

| Crate | Rol |
|---|---|
| [`rimay-verbo-core`](rimay-verbo-core/README.md) | Trait `Verbo` + tipos públicos. |
| [`rimay-verbo-daemon`](rimay-verbo-daemon/README.md) | Loop del daemon + IPC. |
| [`rimay-verbo-daemon-bin`](rimay-verbo-daemon-bin/README.md) | Binario del daemon. |
| [`rimay-verbo-fastembed`](rimay-verbo-fastembed/README.md) | Backend ONNX (BGE, MiniLM). |
| [`rimay-verbo-mock`](rimay-verbo-mock/README.md) | Backend determinista para tests. |

## Consideraciones

- El daemon NO se autodescarga modelos sin permiso del usuario: la primera vez pide confirmación + el path.
- `pluma-llm` y `rimay-verbo` son ortogonales: el primero genera texto, el segundo lo *entiende*.
- Compatible con el `wawa-kernel`: el daemon también compila a WASM y vive en `apps/`.
