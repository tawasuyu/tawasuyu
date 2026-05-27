# chaka-bcd

> BCD (Binary-Coded Decimal) reader/writer for [chaka](../README.md).

Specific legacy format common in mainframes and COBOL systems: decimal digits packed 2-per-byte. This crate offers typed APIs to read BCD records and produce them when a legacy output demands it.

## API

```rust
use chaka_bcd::{read, write, Number};

let n: Number = read(&bytes)?;
let bytes = write(&n);
```

## Deps

- `byteorder` for endian-aware reads
- Zero runtime deps
