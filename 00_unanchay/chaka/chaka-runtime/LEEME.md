# chaka-runtime

> Soporte de ejecución para el código Rust que emite [`chaka-codegen`](../chaka-codegen/LEEME.md).

Una biblioteca pequeña de tipos que le dan a un programa COBOL transpilado su semántica en tiempo de ejecución: campos numéricos y alfanuméricos de ancho fijo, aritmética decimal exacta, formato de PICTURE de edición, E/S de fichero line-sequential. El código que emite `chaka-codegen` **no** es Rust autónomo: enlaza contra este crate.

## Tipos

- `Num` — un campo numérico (`PIC 9(5)V99`): un `Decimal` con la `Picture` que lo conforma. Toda asignación ajusta el valor a la escala y el ancho declarados — el `MOVE` de COBOL.
- `Text` — un campo alfanumérico de longitud fija (`PIC X(20)`); toda asignación justifica a la izquierda y rellena o trunca.
- `format_edited` — aplica una PICTURE de edición (`ZZ,ZZ9.99`) a un `Decimal`.
- `CobFile` — fichero line-sequential (`OPEN INPUT/OUTPUT`, `READ`, `WRITE`, `CLOSE`).
- Reexports de `chaka-bcd`: `Decimal`, `Picture`, `Rounding`.

## API

```rust
use chaka_runtime::*;

let mut ws_cont = Num::with_value(Picture::new(3, 0, false), "0");
ws_cont.store(ws_cont.value().add(&Decimal::from_integer(1)));
assert_eq!(ws_cont.display(), "001");

let mut ws_msg = Text::new(10);
ws_msg.store("HOLA");
assert_eq!(ws_msg.as_str(), "HOLA      ");
```

## Fuera de alcance (v1)

- Sandbox WASM con `wasmtime`/`wasmi` (la idea original; pospuesta — hoy el transpilado corre como Rust nativo).
- Organizaciones de fichero indexada y relativa (`CobFile` sólo soporta line-sequential).

## Deps

- [`chaka-bcd`](../chaka-bcd/LEEME.md) para `Decimal` / `Picture` / `Rounding`.
