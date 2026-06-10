# pluma

> Documentos vivos. Markdown como grafo de átomos editables; LLM como transformador, no como autor.

`pluma` trata un documento como un DAG de párrafos (átomos) con identidad estable. La edición preserva ids; el LLM se invoca como **transformación pura** sobre subgrafos (resumir esta sección, traducir aquel párrafo) — siempre con diff visible y reversible. Un documento puede ser además un **multilienzo**: un haz de cuerpos (original, traducción, resumen, tono…) sobre el mismo material, alineados párrafo a párrafo por hebras; si la madre cambia, el cuerpo derivado queda *stale* y la UI pinta la hebra punteada hasta regenerarlo. Incluye notebook (con kernels Python/WASM/LLM/media/tinkuy — los de cosmos y dominium viven en sus dominios como `cosmos-notebook-kernel` / `dominium-notebook-kernel`), editor visual multilienzo, deck (slides con Recorrido tipo Prezi) y reader web.

## Instalación

```sh
# editor de markdown (Llimphi desktop)
cargo run --release -p pluma-app

# notebook
cargo run --release -p pluma-notebook-app

# reader web (WASM)
./scripts/build-tawasuyu-web.sh
```

## Compatibilidad

- **Linux / macOS / Windows** — apps Llimphi nativas.
- **Wawa** — `pluma` viaja como app del kernel (`03_ukupacha/wawa/apps/pluma/`).
- **Web** — `pluma-md-reader-web` renderiza markdown en navegador (el reader que usa este sitio).

## Crates

| Crate | Rol |
|---|---|
| [`pluma-core`](pluma-core/README.md) | Modelo de documento: átomos, grafo, ids. |
| [`pluma-cuerpo`](pluma-cuerpo/README.md) | Cuerpo (lienzo) del haz multilienzo: intención, lengua, derivación madre→hija. |
| `foreign-docx` | Puente DOCX: importa `.docx` como cuerpos madre (un átomo por párrafo) y exporta de vuelta; solo contenido, sin formato. |
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
| `pluma-deck-recorrido-llimphi` | Modo Recorrido tipo Prezi (lienzo espacial + camino narrativo). |
| `pluma-deck-app` | Binario `pluma-deck`: presentar + autorear + guardar. |
| [`pluma-deck-web`](pluma-deck-web/README.md) | Deck en navegador (espejo espacial + export HTML autocontenido). |
| [`pluma-notebook-core`](pluma-notebook-core/README.md) | Notebook: celdas + outputs addressable. |
| [`pluma-notebook-store`](pluma-notebook-store/README.md) | Persistencia notebook. |
| [`pluma-notebook-exec`](pluma-notebook-exec/README.md) | Despacho a kernels. |
| [`pluma-notebook-kernel-python`](pluma-notebook-kernel-python/README.md) | Python via RustPython/WASM. |
| [`pluma-notebook-kernel-wasm`](pluma-notebook-kernel-wasm/README.md) | WASM/WAT genérico (wasmi, con fuel cap y memory cap). |
| [`pluma-notebook-kernel-llm`](pluma-notebook-kernel-llm/README.md) | Celdas LLM. |
| `pluma-notebook-kernel-media` | Análisis offline de audio (WAV/MP3) → PNG + observables (dominio media). |
| `pluma-notebook-kernel-tinkuy` | Simulación de partículas Lennard-Jones (tinkuy-core) → PNG + observables. |
| `pluma-notebook-kernel-multi` | Router: despacha al kernel concreto por el lenguaje de la celda (wasm/python/media). |
| [`pluma-notebook-llimphi`](pluma-notebook-llimphi/README.md) | Notebook UI Llimphi. |
| [`pluma-notebook-graph-llimphi`](pluma-notebook-graph-llimphi/README.md) | Vista grafo del notebook (celdas como nodos). |
| [`pluma-notebook-app`](pluma-notebook-app/README.md) | Binario del notebook. |

Los kernels de cosmos y dominium se relocalizaron a sus dominios (`01_yachay/cosmos/cosmos-notebook-kernel`, `01_yachay/dominium/dominium-notebook-kernel`); el notebook los consume igual por el trait `Kernel` de `pluma-notebook-exec`.

## Estado (2026-06-10)

### Hecho

- Núcleo de documento vivo: `pluma-core`/`pluma-cuerpo`/`pluma-graph` (DAG de átomos con ids estables) + `pluma-graph-transform` + `pluma-store` (sled).
- Transformaciones puras: `pluma-transform` + `pluma-transform-llm` (resumir/traducir) + `pluma-transform-tabla`, con diff visible y reversible.
- Fachada LLM `pluma-llm` con autodetect + backends anthropic/gemini/cohere/openai-compatible/mock.
- Alineación texto-texto (`pluma-align`) + por embeddings (`pluma-align-embeddings`) + anotaciones semánticas (`pluma-semantic`).
- Editor visual `pluma-editor-llimphi` + binario `pluma-app`; reader web `pluma-md-reader-web`.
- Multilienzo: haz de cuerpos (`pluma-cuerpo`, madre→derivados con staleness) alineados por hebras (`CartaHebras`); en `pluma-app`: rail de dientes, hebras como cintas Sankey con bandas de color por sección, foco por click + Ctrl+Tab, scroll H/V, tree de lienzos reordenable por drag y botón «regenerar stale».
- Puente `foreign-docx`: importa `.docx` como cuerpos madre (un átomo por párrafo) y exporta de vuelta — solo contenido, sin formato (decisión de alcance).
- Notebook: `pluma-notebook-core`/`-exec`/`-store` + UI `pluma-notebook-llimphi` + vista grafo `pluma-notebook-graph-llimphi` + binario + router `pluma-notebook-kernel-multi`. Kernels reales: LLM, media, tinkuy (acá) + cosmos y dominium (en sus dominios).
- Deck: `pluma-deck-core`/`-web` + modo Recorrido tipo Prezi (`pluma-deck-recorrido-llimphi`) con autoría completa, persistencia (postcard), camino narrativo visible, modo presentador (autoplay/bucle) y undo/redo; binario `pluma-deck`.
- Menú principal + menús contextuales cableados en las apps.

### Pendiente

- `pluma-notebook-kernel-python` (RustPython, expresiones single-line) y `-wasm` (wasmi): cimientos funcionando; faltan los kernels superiores (intérpretes encapsulados js/r) y librerías nativas.
- `foreign-docx` no interpreta formato (negrita, estilos, tablas); `foreign-xlsx`/`-pptx` no en disco (`foreign-psd` ya existe en `shared/`, para tullpu).
- Deuda del deck: split de tullpu + `Camara` (ver PLAN §6.sexies).

## Consideraciones

- **El LLM no escribe; transforma.** No hay "modo redacción libre" — cada llamada devuelve una mutación atómica que el usuario aprueba o rechaza.
- Los IDs de átomo son la unidad de verdad: rename/move conservan referencias internas y links externos.
- Kernels del notebook son **WASM-first** (sandboxing del notebook por defecto).
