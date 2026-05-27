# iniy-nli-llm

> LLM-delegated inference for [iniy](../README.md).

NLI backend via LLM ([`pluma-llm`](../../../00_unanchay/pluma/pluma-llm/README.md)). Useful when local inference of [`iniy-nli`](../iniy-nli/README.md) doesn't resolve (long assertions, pragmatic context, distant paraphrases). **Optional and flag-by-flag**: never invoked unless the user enables it.

## API

```rust
use iniy_nli_llm::infer;

let ent = infer(a, b, &chat).await?;
```

## Deps

- [`iniy-nli`](../iniy-nli/README.md) (fallback)
- [`pluma-llm-core`](../../../00_unanchay/pluma/pluma-llm-core/README.md)
