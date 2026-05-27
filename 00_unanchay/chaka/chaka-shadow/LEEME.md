# chaka-shadow

> Modo sombra de [chaka](../README.md): corre legacy + nuevo en paralelo y compara.

El operador despliega el camino nuevo en producción **a la par** del legacy: las dos pipelines reciben el mismo input, el shadow captura ambas salidas, las diffea y reporta divergencia. Cuando la divergencia llega a cero por un período definido por el operador, el legacy se puede apagar con confianza.

## API

```rust
use chaka_shadow::{ShadowRun, Report};

let report: Report = ShadowRun::new(legacy_path, nueva_path)
    .input(input_bytes)
    .timeout(Duration::from_secs(30))
    .compare()?;
```

## Deps

- [`chaka-runtime`](../chaka-runtime/README.md) para correr la versión nueva
- `tokio` o `std::thread` para paralelizar
