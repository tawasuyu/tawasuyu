# pluma

> Documentos vivos. Markdown como grafo de átomos editables; LLM como transformador, no como autor.

`pluma` trata un documento como un DAG de párrafos (átomos) con identidad estable. La edición preserva ids; el LLM se invoca como **transformación pura** sobre subgrafos (resumir esta sección, traducir aquel párrafo) — siempre con diff visible y reversible. Incluye notebook (con kernels Python/WASM/LLM/cosmos/dominium), editor visual, deck (slides) y reader web.

## Instalación

```sh
# editor de markdown (Llimphi desktop)
cargo run --release -p pluma-app

# notebook
cargo run --release -p pluma-notebook-app

# reader web (WASM)
./scripts/build-gioser-web.sh
```

## Compatibilidad

- **Linux / macOS / Windows** — apps Llimphi nativas.
- **Wawa** — `pluma` viaja como app del kernel (`03_ukupacha/wawa/apps/pluma/`).
- **Web** — `pluma-md-reader-web` renderiza markdown en navegador (el reader que usa este sitio).

## Crates

| Crate | Rol |
|---|---|
| [`pluma-core`](pluma-core/README.md) | Modelo de documento: átomos, grafo, ids. |
| [`pluma-cuerpo`](pluma-cuerpo/README.md) | Texto del documento como secuencia de átomos. |
| [`pluma-store`](pluma-store/README.md) | Persistencia (`$XDG_DATA_HOME/pluma/`). |
| [`pluma-md`](pluma-md/README.md) | Parser GFM (pulldown-cmark) → HTML temable. |
| [`pluma-md-reader-web`](pluma-md-reader-web/README.md) | Reader markdown para WASM. |
| [`pluma-graph`](pluma-graph/README.md) | DAG de átomos con identidad. |
| [`pluma-graph-transform`](pluma-graph-transform/README.md) | Mutaciones del DAG (insert/mutar/eliminar). |
| [`pluma-transform`](pluma-transform/README.md) | Marco general de transformaciones puras. |
| [`pluma-transform-llm`](pluma-transform-llm/README.md) | Transforms LLM (resumir, traducir, ...). |
| [`pluma-transform-tabla`](pluma-transform-tabla/README.md) | Transforms tabulares. |
| [`pluma-llm`](pluma-llm/README.md) | Fachada `Arc<dyn ChatClient>` con autodetect. |
| [`pluma-llm-core`](pluma-llm-core/README.md) | Trait `ChatClient`. |
| [`pluma-llm-anthropic`](pluma-llm-anthropic/README.md) | Backend Claude API. |
| [`pluma-llm-gemini`](pluma-llm-gemini/README.md) | Backend Gemini. |
| [`pluma-llm-cohere`](pluma-llm-cohere/README.md) | Backend Cohere. |
| [`pluma-llm-openai-compatible`](pluma-llm-openai-compatible/README.md) | OpenAI / DeepSeek / Ollama / proxies. |
| [`pluma-llm-mock`](pluma-llm-mock/README.md) | Backend mock para tests. |
| [`pluma-align`](pluma-align/README.md) | Alineamiento texto–texto. |
| [`pluma-align-embeddings`](pluma-align-embeddings/README.md) | Alineamiento por embeddings. |
| [`pluma-semantic`](pluma-semantic/README.md) | Anotaciones semánticas del documento. |
| [`pluma-editor-cuerpo`](pluma-editor-cuerpo/README.md) | Editor texto↔átomos con diff (greedy). |
| [`pluma-editor-llimphi`](pluma-editor-llimphi/README.md) | Editor visual Llimphi. |
| [`pluma-app`](pluma-app/README.md) | Binario del editor. |
| [`pluma-render-plan`](pluma-render-plan/README.md) | Plan de render del documento. |
| [`pluma-deck-core`](pluma-deck-core/README.md) | Deck (slides) sobre pluma. |
| [`pluma-deck-web`](pluma-deck-web/README.md) | Deck en navegador. |
| [`pluma-notebook-core`](pluma-notebook-core/README.md) | Notebook: celdas + outputs addressable. |
| [`pluma-notebook-store`](pluma-notebook-store/README.md) | Persistencia notebook. |
| [`pluma-notebook-exec`](pluma-notebook-exec/README.md) | Despacho a kernels. |
| [`pluma-notebook-kernel-python`](pluma-notebook-kernel-python/README.md) | Python via RustPython/WASM. |
| [`pluma-notebook-kernel-wasm`](pluma-notebook-kernel-wasm/README.md) | WASM genérico (cranelift AOT). |
| [`pluma-notebook-kernel-llm`](pluma-notebook-kernel-llm/README.md) | Celdas LLM. |
| [`pluma-notebook-kernel-cosmos`](pluma-notebook-kernel-cosmos/README.md) | Kernel astronomía (cosmos-sky). |
| [`pluma-notebook-kernel-dominium`](pluma-notebook-kernel-dominium/README.md) | Kernel simulador (dominium). |
| [`pluma-notebook-llimphi`](pluma-notebook-llimphi/README.md) | Notebook UI Llimphi. |
| [`pluma-notebook-graph-llimphi`](pluma-notebook-graph-llimphi/README.md) | Vista grafo del notebook (celdas como nodos). |
| [`pluma-notebook-app`](pluma-notebook-app/README.md) | Binario del notebook. |

## Estado (2026-05-31)

### Hecho

- Núcleo de documento vivo: `pluma-core`/`pluma-cuerpo`/`pluma-graph` (DAG de átomos con ids estables) + `pluma-graph-transform` + `pluma-store` (sled).
- Transformaciones puras: `pluma-transform` + `pluma-transform-llm` (resumir/traducir) + `pluma-transform-tabla`, con diff visible y reversible.
- Fachada LLM `pluma-llm` con autodetect + backends anthropic/gemini/cohere/openai-compatible/mock.
- Alineación texto-texto (`pluma-align`) + por embeddings (`pluma-align-embeddings`) + anotaciones semánticas (`pluma-semantic`).
- Editor visual `pluma-editor-llimphi` + binario `pluma-app`; reader web `pluma-md-reader-web`.
- Notebook: `pluma-notebook-core`/`-exec`/`-store` + UI `pluma-notebook-llimphi` + vista grafo `pluma-notebook-graph-llimphi` + binario. Kernels reales: LLM, cosmos, dominium, media, tinkuy.
- Deck: `pluma-deck-core`/`-web` + modo Recorrido tipo Prezi (`pluma-deck-recorrido-llimphi`) con autoría completa, persistencia (postcard), camino narrativo visible, modo presentador (autoplay/bucle) y undo/redo; binario `pluma-deck`.
- Menú principal + menús contextuales cableados en las apps.

### Pendiente

- `pluma-notebook-kernel-python` (RustPython) y `-wasm` (wasmi): cimientos funcionando; falta el camino WASM-AOT cranelift completo y librerías nativas.
- Puente `foreign-docx`: import/export DOCX aún parcial; resto de la familia `foreign-*` (xlsx/pptx/psd) no en disco.
- Deuda del deck: split de tullpu + `Camara` (ver PLAN §6.sexies).

## Consideraciones

- **El LLM no escribe; transforma.** No hay "modo redacción libre" — cada llamada devuelve una mutación atómica que el usuario aprueba o rechaza.
- Los IDs de átomo son la unidad de verdad: rename/move conservan referencias internas y links externos.
- Kernels del notebook son **WASM-first** (sandboxing del notebook por defecto).
