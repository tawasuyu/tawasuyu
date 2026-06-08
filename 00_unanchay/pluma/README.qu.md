<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# pluma

> Kawsasqa qillqakuna. Markdown — átomu-grafu hina, LLM transformadormi, manataq autor.

`pluma` qillqasqata DAG hina qhawan: párrafokuna (átomokuna) sumaq sutiyuq. Tikraspa idskuna waqaychasqa; LLM **ch'uya transformación** hina sub-grafukunapi qharin (kay parte huñuy, chay párrafo tikray) — qhawana diff-wan, kutichinapaq atisqa. Notebook (Python/WASM/LLM/cosmos/dominium kernels), rikuna editor, deck (slides), web reader.

## Churay

```sh
# markdown editor (Llimphi desktop)
cargo run --release -p pluma-app

# notebook
cargo run --release -p pluma-notebook-app

# web reader (WASM)
./scripts/build-tawasuyu-web.sh
```

## Tinkuy

- **Linux / macOS / Windows** — Llimphi natural apps.
- **Wawa** — `pluma` kernel-pa apps-nin hina (`03_ukupacha/wawa/apps/pluma/`).
- **Web** — `pluma-md-reader-web` markdown navegador ukhupi riqsichiq (kay sitio chayta usanqa).

## Crateskuna

Sumaq tabla [README.md](README.md)-pi. Familiakuna:

- **Core + parser**: `pluma-core`, `pluma-cuerpo`, `pluma-md`, `pluma-md-reader-web`, `pluma-graph`, `pluma-semantic`, `pluma-align*`, `pluma-render-plan`, `pluma-store`.
- **Transforms**: `pluma-transform`, `pluma-transform-llm`, `pluma-transform-tabla`.
- **LLM**: `pluma-llm`, `pluma-llm-{core,anthropic,gemini,cohere,openai-compatible,mock}`.
- **Editor**: `pluma-editor-{cuerpo,llimphi}`, `pluma-app`.
- **Deck**: `pluma-deck-{core,web}`.
- **Notebook**: `pluma-notebook-{core,store,exec,llimphi,graph-llimphi,app}` + `pluma-notebook-kernel-{python,wasm,llm,cosmos,dominium}`.

## Yuyaykunaq

- **LLM manan qillqaqchu; tikraqmi.** Mana "ch'iqaq qillqana modo"; sapanka llamayqa atómica mutación kutichin, runa allichaspa utaq mana munaspa.
- Átomu IDmi cheqaq unidad: tikrakuna sutikunata waqaychan, ukhupi rikchakuna hawapi linkkuna ima.
- Notebook kernelkuna **WASM-ñawpaq** (notebook sandbox kasqa).
