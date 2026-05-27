<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# rimay

> `rimay` (runa-simi: *rimay, simi*). Simi: embeddingkuna, verbos, imapas *niy munaq*-nin.

Lokal NLP/embeddings servicio. Ima monorepo aplikacionllapas qhawakun: "kay iskay qillqakunaq tupayniyninnata" utaq "kayqa embeddingninta quway", red-man mana llosqashpa. Daemon `rimay-verbo-daemon` servicio hina puriq; clientes tipo-bus ([`chasqui`](../../02_ruway/chasqui/README.md)-rayku) rimanku.

## Churay

```sh
# daemon kawsachiy
cargo run --release -p rimay-verbo-daemon-bin

# mock modo (mana GPU, mana modelo wasi-chaykuna)
RIMAY_BACKEND=mock cargo run --release -p rimay-verbo-daemon-bin
```

## Tinkuy

- **Linux** — `fastembed` backend ONNX runtimewan (CPU utaq GPU).
- **macOS / Windows / Wawa** — `mock` backend utaq `fastembed` CPU.
- Modelokuna cache `$XDG_CACHE_HOME/rimay/`-pi.

## Crateskuna

| Crate | Ima ruwan |
|---|---|
| [`rimay-verbo-core`](rimay-verbo-core/README.md) | `Verbo` trait + hawa-tiposkuna. |
| [`rimay-verbo-daemon`](rimay-verbo-daemon/README.md) | Daemon loop + IPC. |
| [`rimay-verbo-daemon-bin`](rimay-verbo-daemon-bin/README.md) | Daemon binario. |
| [`rimay-verbo-fastembed`](rimay-verbo-fastembed/README.md) | ONNX backend (BGE, MiniLM). |
| [`rimay-verbo-mock`](rimay-verbo-mock/README.md) | Mock backend tests-paq. |

## Yuyaykunaq

- Daemon **manaña** kikinmanta modelokunata wasi-chayanqachu mana runaq simi-quynin: ñawpaq kuti tapunqa.
- [`pluma-llm`](../pluma/pluma-llm/README.md) `rimay-verbo`-wan ortogonalkuna: ñawpaqqa qillqata wakichin, qhipan **yachan**.
- Wawa-tinkuy: daemon WASM-man wiñakun, `apps/`-pi tiyan.
