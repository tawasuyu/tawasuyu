# chaka-bcd

> Aritmética decimal con semántica COBOL y codec BCD packed-decimal para [chaka](../LEEME.md).

Dos piezas del corazón numérico:

- **Aritmética de punto fijo exacta** (`Decimal`, `Picture`, `Rounding`): COBOL no calcula en flotante binario — opera sobre campos decimales de precisión fija declarados con una cláusula `PICTURE`. Reproducir un programa COBOL fielmente exige reproducir esa aritmética dígito a dígito. Determinista, sin deps de plataforma.
- **Codec packed-decimal** (`pack`, `unpack`, `packed_size`): el formato `COMP-3` que usan mainframes y ficheros de datos COBOL. Un dígito por nibble, signo en el último nibble (`C` positivo, `D` negativo, `F` sin signo).

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

- `serde` para serializar `Decimal` y `Picture`.
- `thiserror` para `BcdError`.
- Sin deps de I/O.
