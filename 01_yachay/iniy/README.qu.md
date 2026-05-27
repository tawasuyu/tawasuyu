<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# iniy

> Semantik laboratorio. Iñiy grados + subjetividad direcciónpa modelu.

`iniy` **Subjective Logic** + sutilla "subjetividadpa dirección" eje (autoría, fuente, posición)-wan, hatun qillqakuna ñiqipi auditoría ruwan. Piloto: libros + wiki auditorías. Pipeline: ingest → extract → graph → NLI → reporte.

## Churay

```sh
cargo run --release -p iniy-cli -- ingest /path/to/libro.md
cargo run --release -p iniy-cli -- audit  /path/to/libro.md
cargo run --release -p iniy-explorer-llimphi
cargo run --release -p iniy-server
```

## Tinkuy

- **Linux / macOS / Windows** — CLI + Llimphi UI.
- NLI backend: lokal (`iniy-nli`) utaq LLM (`iniy-nli-llm` `pluma-llm`-rayku).
- Lokal SQLite store (`iniy-store`).

## Crateskuna

[README.md](README.md)-pi. Pipeline: `iniy-{core, ingest, extract, graph, nli, nli-llm, store, wiki, cli, server, explorer-llimphi}`.

## Yuyaykunaq

- **iniy manan munanchu** — iñiy gradoskuna, mana cheqaqkay sutillapaq kutichin. Runaq tukuchayqa hawapi.
- LLM-NLI opcional; mana imapas redman riy mañakun.
- Runa-reviewer **sapanka step** kutichinapaq ruwana.
