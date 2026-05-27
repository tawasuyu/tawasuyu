<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# chaka

> `chaka` (runa-simi: *chaka — puente*). Mawk'a kódigo monorepuwan tinkunapaq chaka.

Hawa kawsasqakuna (BCD, mawk'a simikuna, mawk'a formatokuna) ñawinchaspa, sistemapa simiman tikran. Pipeline capakunapi: lexer → parser → IR → codegen → runtime. `chaka-shadow`-pi mawk'akunata kuska puriy ruwan, kutichikunata kasqachisqa, mana ñawpaq flow p'akirispa.

## Churay

```sh
cargo build --release -p chaka-app
./target/release/chaka --help
```

## Tinkuy

- **Linux / macOS / Windows** — Rust ch'uya, sistema deps illaq.
- **Wawa** — `chaka-runtime` WASM-man wiñan, `wawa-kernel` ukhupi puriq.

## Crateskuna

| Crate | Ima ruwan |
|---|---|
| [`chaka-app`](chaka-app/README.md) | Haykunapaq CLI/UI. |
| [`chaka-lexer`](chaka-lexer/README.md) | Mawk'a tikrana → tokens. |
| [`chaka-parser`](chaka-parser/README.md) | Tipo AST simipa. |
| [`chaka-ir`](chaka-ir/README.md) | IR chawpipi, kasqasqa. |
| [`chaka-codegen`](chaka-codegen/README.md) | IR → tukuna kódigo. |
| [`chaka-runtime`](chaka-runtime/README.md) | Wiñasqa kódigo phawachiq. |
| [`chaka-bcd`](chaka-bcd/README.md) | BCD ñawinchaq + qillqaq. |
| [`chaka-shadow`](chaka-shadow/README.md) | Llanthu modo: mawk'a + musuq kuska, kasqachiq. |

## Yuyaykunaq

- Llanthu modoqa manan mawk'akunaq lluqsichinchu; **wachawasqachu**, mana waqaq divergencia chayasqankama.
- Sapanka musuq mawk'a fuente ñawpaqman `chaka-lexer` dialecto hina haykun, hinaspa IR-man wichariykun.
