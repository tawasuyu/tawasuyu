# rimay

> `rimay` (quechua: *hablar, decir, palabra*). Lenguaje: embeddings, verbos, lo que *quiere decir* algo.

Servicio local de NLP/embeddings que cualquier app del monorepo consulta cuando necesita "qué tan parecidos son estos dos textos" o "dame el embedding de esto" sin salir a la red. El binario `verbo-daemon` carga un modelo una sola vez y lo expone por un socket Unix; los clientes lo consumen vía `rimay_verbo_daemon::DaemonClient`, que implementa el trait `Provider` — desde la perspectiva del consumidor es indistinguible de tener el backend en proceso.

## Instalación

```sh
# arrancar el daemon con el backend real (fastembed / multilingual-e5-small)
cargo run --release -p rimay-verbo-daemon-bin -- --provider fastembed

# o en modo mock (sin descarga de modelos, vectores deterministas, ideal para CI)
cargo run --release -p rimay-verbo-daemon-bin -- --provider mock
```

El socket por defecto es `$XDG_RUNTIME_DIR/verbo.sock`, con fallback a `/tmp/verbo-{uid}.sock`. Para sobreescribirlo, `--socket /ruta` (multi-daemon = un socket por modelo).

## Consumiendo desde un crate

La fachada [`rimay-verbo`](rimay-verbo/) esconde el descubrimiento del socket y da una línea con fallback a mock:

```rust
use rimay_verbo::Provider;

let provider = rimay_verbo::conectar_o_mock(384).await;
let v = provider.embed("hola").await?;
```

Los consumidores que quieran fallar fuerte si no hay daemon usan `rimay_verbo::conectar()` en su lugar. Para health-check sin invocar al modelo: `DaemonClient::ping().await`.

## Compatibilidad

- **Linux** — backend `fastembed` con ONNX-Runtime en CPU; descarga el modelo a `~/.cache/fastembed` en el primer arranque.
- **macOS / Windows** — `mock` siempre; `fastembed` si su dependencia ONNX compila en el host.
- **Wawa** — pendiente: el daemon todavía no compila a WASM.

## Crates

| Crate | Rol |
|---|---|
| [`rimay-verbo`](rimay-verbo/) | Fachada one-line (`conectar_o_mock`, descubrimiento de socket, re-exports). |
| [`rimay-verbo-core`](rimay-verbo-core/) | Trait `Provider` + tipos públicos (`ModelId`, `EmbeddingVector`, `EmbedError`). |
| [`rimay-verbo-daemon`](rimay-verbo-daemon/) | Loop del daemon + IPC sobre socket Unix (frames postcard + 1 reintento transparente). |
| [`rimay-verbo-daemon-bin`](rimay-verbo-daemon-bin/) | Binario `verbo-daemon`, con apagado limpio en SIGINT/SIGTERM. |
| [`rimay-verbo-fastembed`](rimay-verbo-fastembed/) | Backend ONNX (`multilingual-e5-small` por defecto; catálogo E5/BGE). |
| [`rimay-verbo-mock`](rimay-verbo-mock/) | Backend determinista (FNV + LCG; sin descargas, sin GPU). |

## Estado (2026-05-31)

### Hecho

- Contrato model-agnostic `rimay-verbo-core` (trait `Provider`, `ModelId`, `EmbeddingVector` con `cosine` que rechaza cruzar modelos).
- Daemon `rimay-verbo-daemon` + binario `rimay-verbo-daemon-bin`: IPC por socket Unix (frames postcard, 1 reintento transparente, apagado limpio SIGINT/SIGTERM, descubrimiento de socket con fallback `/tmp`).
- Backends: `rimay-verbo-fastembed` (ONNX real, `multilingual-e5-small` por defecto, catálogo E5/BGE, sin API key) y `rimay-verbo-mock` (FNV+LCG determinista, ideal CI).
- Fachada `rimay-verbo` one-line (`conectar_o_mock`/`conectar`) + `ping` health-check + reconnect. Consumida por pluma/khipu/chasqui.

### Pendiente

- Gating de permiso antes de la descarga del modelo ONNX (hoy descarga sin confirmar).
- Compilar el daemon a WASM para Wawa (hoy es Linux/host-only).
- Vertiente lingüística de rimay (morfología/conjugación quechua) más allá de embeddings — no presente en estos subcrates.

## Consideraciones

- El backend `fastembed` descarga el modelo ONNX en el primer arranque sin pedir confirmación. La cache vive en `~/.cache/fastembed`; borrarla fuerza re-descarga. (El gating de permiso antes de la descarga está pendiente.)
- [`pluma-llm`](../pluma/pluma-llm/) y `rimay-verbo` son ortogonales: el primero *genera* texto, el segundo lo *entiende*.
- Los vectores van etiquetados con su `ModelId`; `EmbeddingVector::cosine` se niega a comparar a través de modelos, así que un vector `mock` y uno `fastembed` no se mezclan silenciosamente.
