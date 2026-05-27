# iniy

> Laboratorio semántico. Modela grados de creencia y dirección de subjetividad.

`iniy` aplica **Subjective Logic** + un eje explícito de "dirección de subjetividad" (autoría, fuente, posicionalidad) para auditar afirmaciones en textos largos. Piloto: auditoría de libros y wikis. Pipeline: ingest → extract → graph → NLI → reporte.

## Instalación

```sh
cargo run --release -p iniy-cli -- ingest /path/to/libro.md
cargo run --release -p iniy-cli -- audit  /path/to/libro.md
cargo run --release -p iniy-explorer-llimphi
cargo run --release -p iniy-server
```

## Compatibilidad

- **Linux / macOS / Windows** — CLI + UI Llimphi.
- Backend NLI: local (`iniy-nli`) o LLM (`iniy-nli-llm` vía `pluma-llm`).
- Store local SQLite (`iniy-store`).

## Crates

| Crate | Rol |
|---|---|
| [`iniy-core`](iniy-core/README.md) | Tipos: opiniones, evidencia, ejes de subjetividad. |
| [`iniy-ingest`](iniy-ingest/README.md) | Lectura de fuentes (md/pdf/wiki). |
| [`iniy-extract`](iniy-extract/README.md) | Extracción de afirmaciones. |
| [`iniy-graph`](iniy-graph/README.md) | Grafo de afirmaciones + relaciones. |
| [`iniy-nli`](iniy-nli/README.md) | Inferencia local (rules + embeddings via rimay). |
| [`iniy-nli-llm`](iniy-nli-llm/README.md) | Inferencia delegada a LLM. |
| [`iniy-store`](iniy-store/README.md) | Persistencia. |
| [`iniy-wiki`](iniy-wiki/README.md) | Crawler/parser para Wikipedia/MediaWiki. |
| [`iniy-cli`](iniy-cli/README.md) | CLI. |
| [`iniy-server`](iniy-server/README.md) | HTTP. |
| [`iniy-explorer-llimphi`](iniy-explorer-llimphi/README.md) | UI Llimphi: grafo + auditoría. |

## Consideraciones

- **Iniy no opina** — devuelve grados de creencia con incertidumbre explícita. La conclusión humana queda fuera del sistema.
- LLM-NLI es opcional y bandera-by-bandera: ningún flow obliga a salir a la red.
- Diseñado para que un revisor humano pueda **reproducir cada paso** del audit.
