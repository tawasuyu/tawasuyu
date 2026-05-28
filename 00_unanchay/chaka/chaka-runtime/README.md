# chaka-runtime

> Runtime support for the Rust code that [`chaka-codegen`](../chaka-codegen/README.md) emits.

A small library of types that give a transpiled COBOL program its runtime semantics: fixed-width numeric and alphanumeric fields, decimal-exact arithmetic, edited PICTURE formatting, line-sequential file I/O. The code emitted by `chaka-codegen` is **not** standalone Rust — it links against this crate.

## Types

- `Num` — a numeric field (`PIC 9(5)V99`): a `Decimal` plus the `Picture` that shapes it. Every store fits the value to the declared scale and width, like COBOL's `MOVE`.
- `Text` — a fixed-length alphanumeric field (`PIC X(20)`); every store left-justifies and pads or truncates.
- `format_edited` — applies an edited PICTURE (`ZZ,ZZ9.99`) to a `Decimal`.
- `CobFile` — a line-sequential file (`OPEN INPUT/OUTPUT`, `READ`, `WRITE`, `CLOSE`).
- Re-exports from `chaka-bcd`: `Decimal`, `Picture`, `Rounding`.

## API

```rust
use chaka_runtime::*;

let mut ws_count = Num::with_value(Picture::new(3, 0, false), "0");
ws_count.store(ws_count.value().add(&Decimal::from_integer(1)));
assert_eq!(ws_count.display(), "001");

let mut ws_msg = Text::new(10);
ws_msg.store("HELLO");
assert_eq!(ws_msg.as_str(), "HELLO     ");
```

## Out of scope (v1)

- WASM sandboxing with `wasmtime`/`wasmi` (the original plan; postponed — the transpiled output runs as native Rust today).
- Indexed and relative file organisations (`CobFile` only supports line-sequential).

## Deps

- [`chaka-bcd`](../chaka-bcd/README.md) for `Decimal` / `Picture` / `Rounding`.
