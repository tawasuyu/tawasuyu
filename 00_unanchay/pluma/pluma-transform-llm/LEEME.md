# pluma-transform-llm

> Transforms LLM para [pluma](../README.md): resumir, traducir, reescribir, anotar.

Cada transform es un struct con un prompt template + el `Arc<dyn ChatClient>` que provee [`pluma-llm`](../pluma-llm/README.md). Ejecutores: `Resumir`, `Traducir`, `Reescribir`, `Anotar`. Refactor reciente: ya no llevan genérico `<C>` — usan `Arc<dyn ChatClient>` uniforme, y exponen `new<C>(...)` para clients concretos + `from_arc(arc, ...)` para el factory transparente.

## API

```rust
use pluma_transform_llm::{Resumir, Traducir};

let r = Resumir::from_arc(chat, /* params */);
let salida = r.aplicar(&entrada)?;
```

## Deps

- [`pluma-transform`](../pluma-transform/README.md), [`pluma-llm-core`](../pluma-llm-core/README.md)
- `serde`, `uuid`
