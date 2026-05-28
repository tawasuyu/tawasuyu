# chaka-shadow

> Validación en sombra del pipeline de [chaka](../LEEME.md).

Dos rutas de ejecución independientes para el mismo fuente COBOL:

- **Intérprete en proceso** (`interpret(&Ir)` / `run_source(&str)`): recorre el IR directamente sobre los tipos de `chaka-runtime` sin compilar nada. Es la ruta rápida que usan `chaka run` y los tests del corpus.
- **Harness GnuCOBOL** (`cobc::compare_with_cobc(source)`): compila el fuente con `cobc -x -free`, ejecuta el binario con timeout y devuelve las dos stdouts lado a lado para que quien llama las diffee. Opt-in: necesita `cobc` en el `PATH`; los tests que dependen de eso van `#[ignore]` por defecto.

Si el intérprete y el código transpilado divergen, hay un bug en `chaka-codegen`; si el intérprete y `cobc` divergen, el bug está en el **intérprete**. Hacen falta las dos mitades.

## API

```rust
use chaka_shadow::{interpret, run_source, Outcome};

let outcome: Outcome = run_source(cobol)?;
for linea in &outcome.lines {
    println!("{linea}");
}

// Validación opt-in contra GnuCOBOL:
use chaka_shadow::cobc;
if cobc::is_available() {
    let reporte = cobc::compare_with_cobc(cobol)?;
    assert!(reporte.matches());
}
```

## Fuera de alcance (v1)

- Despliegue «sombra» en producción con timeouts, presupuestos de reintento y tableros de divergencia — la idea original. Hoy es un validador de tiempo de desarrollo, no un harness de producción.

## Deps

- [`chaka-ir`](../chaka-ir/LEEME.md), [`chaka-lexer`](../chaka-lexer/LEEME.md), [`chaka-parser`](../chaka-parser/LEEME.md), [`chaka-runtime`](../chaka-runtime/LEEME.md).
- `thiserror` para errores. Sin deps async — el harness `cobc` usa `std::process::Command` con timeout por polling.
