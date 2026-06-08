# pluma

> Living documents. Markdown as a graph of editable atoms; LLM as transformer, not author.

`pluma` treats a document as a DAG of paragraphs (atoms) with stable identity. Editing preserves ids; the LLM is invoked as **pure transformation** over subgraphs (summarize this section, translate that paragraph) — always with visible, reversible diff. Includes notebook (with Python/WASM/LLM/cosmos/dominium kernels), visual editor, deck (slides) and web reader.

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

Deck: `pluma-deck-core`, `pluma-deck-web`.

Notebook: `pluma-notebook-{core,store,exec,llimphi,graph-llimphi,app,kernel-python,kernel-wasm,kernel-llm,kernel-cosmos,kernel-dominium}`.

Full table in [README.md](README.md).

## Considerations

- **The LLM doesn't write; it transforms.** No "free writing mode" — each call returns an atomic mutation that the user approves or rejects.
- Atom IDs are the unit of truth: rename/move preserves internal refs and outside links.
- Notebook kernels are **WASM-first** (notebook sandboxing by default).
