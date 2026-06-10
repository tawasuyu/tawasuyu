# pluma

> Living documents. Markdown as a graph of editable atoms; LLM as transformer, not author.

![pluma's multilienzo: one text living in several languages at once — a Spanish mother body, its English translation and a summary, aligned paragraph-to-paragraph by threads](https://tawasuyu.net/00_unanchay/pluma/pantallazo.png)

`pluma` treats a document as a DAG of paragraphs (atoms) with stable identity. Editing preserves ids; the LLM is invoked as **pure transformation** over subgraphs (summarize this section, translate that paragraph) — always with visible, reversible diff. A document can also be a **multilienzo**: a bundle of bodies (original, translation, summary, tone…) over the same material, aligned paragraph-to-paragraph by threads; if the mother changes, the derived body goes *stale* and the UI paints the thread dotted until regenerated. Includes notebook (with Python/WASM/LLM/media/tinkuy kernels — the cosmos and dominium kernels live in their own domains as `cosmos-notebook-kernel` / `dominium-notebook-kernel`), visual multilienzo editor, deck (slides with a Prezi-style Recorrido mode) and web reader.

## Install

```sh
# markdown editor (Llimphi desktop)
cargo run --release -p pluma-app

# notebook
cargo run --release -p pluma-notebook-app

# web reader (WASM)
./scripts/build-tawasuyu-web.sh
```

## Compatibility

- **Linux / macOS / Windows** — native Llimphi apps.
- **Wawa** — `pluma` ships as a kernel app (`03_ukupacha/wawa/apps/pluma/`).
- **Web** — `pluma-md-reader-web` renders markdown in the browser (the reader this site uses).

## Crates

Core + parser: `pluma-core`, `pluma-cuerpo`, `pluma-store`, `pluma-md`, `pluma-md-reader-web`, `pluma-graph`, `pluma-graph-transform`, `pluma-semantic`, `pluma-align`, `pluma-align-embeddings`, `pluma-render-plan`.

Transforms: `pluma-transform`, `pluma-transform-llm`, `pluma-transform-tabla`.

LLM facade: `pluma-llm`, `pluma-llm-core`, `pluma-llm-anthropic`, `pluma-llm-gemini`, `pluma-llm-cohere`, `pluma-llm-openai-compatible`, `pluma-llm-mock`.

Editor: `pluma-editor-cuerpo`, `pluma-editor-llimphi`, `pluma-app`.

Deck: `pluma-deck-core`, `pluma-deck-recorrido-llimphi`, `pluma-deck-app` (binary `pluma-deck`), `pluma-deck-web`.

Notebook: `pluma-notebook-{core,store,exec,llimphi,graph-llimphi,app,kernel-python,kernel-wasm,kernel-llm,kernel-media,kernel-tinkuy,kernel-multi}`. The cosmos and dominium kernels were relocated to their domains (`01_yachay/cosmos/cosmos-notebook-kernel`, `01_yachay/dominium/dominium-notebook-kernel`); the notebook consumes them through the same `Kernel` trait.

Foreign bridge: `foreign-docx` imports `.docx` as mother bodies of the multilienzo (one atom per paragraph) and exports back — content only, no formatting.

Full table in [LEEME.md](LEEME.md).

## Status (2026-06-10)

### Done

- Living-document core: `pluma-core`/`pluma-cuerpo`/`pluma-graph` (atom DAG with stable ids) + `pluma-graph-transform` + `pluma-store` (sled).
- Pure transformations: `pluma-transform` + `pluma-transform-llm` (summarize/translate) + `pluma-transform-tabla`, with visible, reversible diff.
- LLM facade `pluma-llm` with autodetect + anthropic/gemini/cohere/openai-compatible/mock backends.
- Text-to-text alignment (`pluma-align`) + embeddings alignment (`pluma-align-embeddings`) + semantic annotations (`pluma-semantic`).
- Visual editor `pluma-editor-llimphi` + binary `pluma-app`; web reader `pluma-md-reader-web`.
- Multilienzo: bundle of bodies (`pluma-cuerpo`, mother→derived with staleness) aligned by threads (`CartaHebras`); in `pluma-app`: dock rail, threads as Sankey ribbons with per-section color bands, focus by click + Ctrl+Tab, H/V scroll, drag-reorderable canvas tree and a "regenerate stale" button.
- `foreign-docx` bridge: imports `.docx` as mother bodies (one atom per paragraph) and exports back — content only, no formatting (a scope decision).
- Notebook: `pluma-notebook-core`/`-exec`/`-store` + UI `pluma-notebook-llimphi` + graph view `pluma-notebook-graph-llimphi` + binary + router `pluma-notebook-kernel-multi`. Real kernels: LLM, media, tinkuy (here) + cosmos and dominium (in their domains).
- Deck: `pluma-deck-core`/`-web` + Prezi-style Recorrido mode (`pluma-deck-recorrido-llimphi`) with full authoring, persistence (postcard), visible narrative path, presenter mode (autoplay/loop) and undo/redo; binary `pluma-deck`.
- Main menu + contextual menus wired into the apps.

### Pending

- `pluma-notebook-kernel-python` (RustPython, single-line expressions) and `-wasm` (wasmi): foundations working; higher kernels (encapsulated js/r interpreters) and native libraries are missing.
- `foreign-docx` doesn't interpret formatting (bold, styles, tables); `foreign-xlsx`/`-pptx` not on disk (`foreign-psd` already exists in `shared/`, for tullpu).
- Deck debt: tullpu split + `Camara` (see PLAN §6.sexies).

## Considerations

- **The LLM doesn't write; it transforms.** No "free writing mode" — each call returns an atomic mutation that the user approves or rejects.
- Atom IDs are the unit of truth: rename/move preserves internal refs and outside links.
- Notebook kernels are **WASM-first** (notebook sandboxing by default).
