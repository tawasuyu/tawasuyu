# iniy

> Laboratorio semántico. Modela grados de creencia y dirección de subjetividad.

![el banco de verificación de afirmaciones iniy: cuatro fuentes con barras de reputación (un blog detox en −1.00, el consenso IARC/OMS, una revista médica, wikipedia), once aserciones sobre el café puntuadas con creencia/incertidumbre (b/u/π), y las hebras de relación — apoyo y contradicción — cruzando el lienzo entre ellas](https://tawasuyu.net/01_yachay/iniy/pantallazo.png)

`iniy` aplica **Subjective Logic** + un eje explícito de "dirección de subjetividad" (autoría, fuente, posicionalidad) para auditar afirmaciones en textos largos. Piloto: auditoría de libros y wikis. Pipeline: ingest → extract → graph → NLI → reporte.

## Instalación

```sh
cargo run --release -p iniy-cli -- ingest /path/to/libro.md
cargo run --release -p iniy-cli -- extract <doc_id>   # después: nli, contradictions, testimonio, consenso, ask...
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

## Estado (2026-05-31)

### Hecho

- `iniy-core`: `Opinion` binomial de Jøsang (creencia/descreencia/
  incertidumbre/base_rate) con operadores **fusión acumulativa**, **descuento
  por confianza**, **inversión** de polaridad y probabilidad esperada; tipos
  `Asercion`, `Fuente`, `RelacionNli`, `Implicacion`. 15 tests del núcleo.
- Pipeline completo de CLI (`iniy-cli`, ~1.2k LOC): ingest (TXT/MD/PDF/EPUB +
  OCR tesseract), extract heurístico + bulk extract-all, NLI léxico **o** LLM,
  contradicciones, testimonio, consenso (con descuento por NLI + reputación),
  propagación por el grafo, Ask (RAG con citas), stats, timeline, reputación,
  tags, export/import (JSON y SQLite).
- Escala: prefiltro por embeddings + ANN (HNSW vía instant-distance) para
  NLI sobre millones de aserciones; embeddings vía fastembed con fallback mock.
- Persistencia SQLite (`iniy-store`, splitteado en módulos desde ~1.5k LOC):
  documentos, aserciones, reputaciones, tags, dump federable.
- `iniy-server` (API HTTP read-only, axum), `iniy-wiki` (crawler MediaWiki),
  `iniy-graph` (grafo de creencias con top-contradicciones y propagación).
- `iniy-explorer-llimphi`: UI del grafo con highlight de vecinos al seleccionar.

### Pendiente

- NLI léxico es heurístico: la calidad real depende del backend LLM o de
  embeddings — falta un motor NLI local más fuerte que el overlap léxico.
- Auditoría visual end-to-end en `iniy-explorer-llimphi` (hoy es sobre todo
  visor de grafo; falta panel de auditoría/consenso interactivo).
- Opiniones multinomiales (más de dos resultados); hoy sólo binomiales.
- Operadores SL adicionales (deducción, abducción, transitividad multi-salto).
- Piloto real documentado de auditoría de un libro completo.
