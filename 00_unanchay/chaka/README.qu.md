<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# chaka

> `chaka` (runa-simi: *chaka — puente*). Mawk'a COBOL kódigo monorepuwan tinkunapaq chaka.

COBOL'85 fuentekuna ñawinchaspa Rust compilable-man tikran. Pipeline capakunapi: `lexer → parser → ir → codegen` Rust lluqsichipaq, `chaka-shadow`-wan kuska — proceso ukhupi intérprete-pi IR-pa puriynin transpiladowan iguallasqachu chayqachan, GnuCOBOL harness opt-in-piwan iskayninta COBOL compilador cheqaqwan chayqachan.

## Churay

```sh
cargo build --release -p chaka-app
./target/release/chaka --help
```

## Tinkuy

- **Linux / macOS / Windows** — Rust ch'uya, sistema deps illaq.
- **GnuCOBOL** (`cobc`) opcional kachan; kasqaptin, `chaka-shadow::cobc`-qa proceso ukhupi intérprete-ta compilador cheqaqwan chayqachan.

## Crateskuna

| Crate | Ima ruwan |
|---|---|
| [`chaka-app`](chaka-app/README.qu.md) | CLI: `transpile`, `scaffold`, `run`, `check`. |
| [`chaka-lexer`](chaka-lexer/README.qu.md) | COBOL fuente → tokens; `COPY` mast'arichin. |
| [`chaka-parser`](chaka-parser/README.qu.md) | Tipo AST: divisiones, DATA sach'a, sentenciakuna. |
| [`chaka-ir`](chaka-ir/README.qu.md) | AST → tipo statementkuna (`MOVE`, `IF`, `PERFORM`, `CALL`, `SEARCH`...). |
| [`chaka-codegen`](chaka-codegen/README.qu.md) | IR → Rust fuente (sapaqlla) utaq IR → JSON. |
| [`chaka-runtime`](chaka-runtime/README.qu.md) | Tikrasqa kódigo enlazapaq runtime: `Num`, `Text`, `CobFile`, `format_edited`. |
| [`chaka-bcd`](chaka-bcd/README.qu.md) | Decimal yupay COBOL semanticawan + packed-decimal (`COMP-3`) codec. |
| [`chaka-shadow`](chaka-shadow/README.qu.md) | Proceso ukhu intérprete + GnuCOBOL harness — cheqaqwan diff. |

## Mana kanchu (v1)

- Mana COBOL dialecto: `Dialect` enum `chaka-lexer`-pi kachan, `Cobol`-llan ruwasqa.
- WASM target `chaka-codegen`-pi nin sandbox WASM `chaka-runtime`-pi — iskayllan plansqa, iskayllan `no_std` rework suyaspan.
- Llimphi UI `chaka-app`-paq — kunan binario CLI-lla.
- `REPLACE` directiva (preprocesador `COPY`-llata mast'arin).
- Indexada utaq relativa fichero: `START`, `REWRITE`, `DELETE` parsesqakun pero line-sequential-pi no-op hina.
