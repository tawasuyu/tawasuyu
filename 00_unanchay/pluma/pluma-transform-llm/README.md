# pluma-transform-llm

> LLM transforms for [pluma](../README.md): summarize, translate, rewrite, annotate.

Each transform is a struct with a prompt template + the `Arc<dyn ChatClient>` provided by [`pluma-llm`](../pluma-llm/README.md). Executors: `Resumir`, `Traducir`, `Reescribir`, `Anotar`. Recent refactor: no longer carry a `<C>` generic — they use uniform `Arc<dyn ChatClient>`, exposing `new<C>(...)` for concrete clients + `from_arc(arc, ...)` for the transparent factory.

## API

```rust
use pluma_transform_llm::{Resumir, Traducir};

let r = Resumir::from_arc(chat, /* params */);
let salida = r.aplicar(&entrada)?;
```

## Deps

- [`pluma-transform`](../pluma-transform/README.md), [`pluma-llm-core`](../pluma-llm-core/README.md)
- `serde`, `uuid`
