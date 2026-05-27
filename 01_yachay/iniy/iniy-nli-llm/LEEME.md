# iniy-nli-llm

> Inferencia delegada a LLM para [iniy](../README.md).

Backend NLI vía LLM ([`pluma-llm`](../../../00_unanchay/pluma/pluma-llm/README.md)). Útil cuando la inferencia local de [`iniy-nli`](../iniy-nli/README.md) no resuelve (afirmaciones largas, contexto pragmático, paráfrasis lejanas). **Opcional y flag-by-flag**: nunca se invoca sin que el usuario lo habilite.

## API

```rust
use iniy_nli_llm::infer;

let ent = infer(a, b, &chat).await?;
```

## Deps

- [`iniy-nli`](../iniy-nli/README.md) (fallback)
- [`pluma-llm-core`](../../../00_unanchay/pluma/pluma-llm-core/README.md)
