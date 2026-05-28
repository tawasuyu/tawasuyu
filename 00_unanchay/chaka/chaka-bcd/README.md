# chaka-bcd

> Decimal arithmetic with COBOL semantics and BCD packed-decimal codec for [chaka](../README.md).

Two pieces of the numerical core:

- **Exact fixed-point arithmetic** (`Decimal`, `Picture`, `Rounding`): COBOL doesn't compute in binary floating-point — it operates on decimal fields of fixed precision declared with a `PICTURE` clause. Reproducing a COBOL program faithfully means reproducing that digit-by-digit arithmetic. Deterministic, platform-independent.
- **Packed-decimal codec** (`pack`, `unpack`, `packed_size`): the `COMP-3` byte format used by mainframes and COBOL data files. One digit per nibble, sign in the last nibble (`C` positive, `D` negative, `F` unsigned).

## API

```rust
use chaka_bcd::{Decimal, Picture, pack, unpack};

// Aritmética decimal exacta.
let pic = Picture::parse("S9(5)V99")?;
let total = Decimal::parse("123.45")?.add(&Decimal::parse("67.89")?);

// Packed-decimal: pack y unpack.
let bytes = pack(&total, &pic);            // bytes COMP-3 listos para disco
let same  = unpack(&bytes, &pic)?;         // roundtrip exacto
assert_eq!(total, same);
```

## Deps

- `serde` for serializing `Decimal` and `Picture`.
- `thiserror` for `BcdError`.
- No I/O deps.
