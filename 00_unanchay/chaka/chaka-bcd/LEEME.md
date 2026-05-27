# chaka-bcd

> Reader/writer BCD (Binary-Coded Decimal) para [chaka](../README.md).

Formato legacy específico común en mainframes y sistemas COBOL: dígitos decimales empacados 2-por-byte. Este crate ofrece API tipada para leer registros BCD y producirlos cuando un legacy ouput los exige.

## API

```rust
use chaka_bcd::{read, write, Number};

let n: Number = read(&bytes)?;
let bytes = write(&n);
```

## Deps

- `byteorder` para lectura endian-aware
- Cero deps de runtime
