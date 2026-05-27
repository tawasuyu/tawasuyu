# pluma-notebook-kernel-llm

> LLM kernel for the [pluma](../README.md) notebook.

LLM cell: `source` is a prompt, `output` is the model's response. Uses the [`pluma-llm`](../pluma-llm/README.md) facade — backend is chosen via the notebook's global config (Anthropic, Gemini, Cohere, OpenAI-compatible, Mock). BLAKE3-caching by (prompt + model + params) so unchanged re-runs are free.

## API

```rust
use pluma_notebook_kernel_llm::LlmKernel;

let k = LlmKernel::con_config(LlmConfig::from_env()?);
let outputs = k.correr(&celda).await?;
```

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md), [`pluma-llm`](../pluma-llm/README.md)
