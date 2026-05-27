# pluma-notebook-kernel-llm

> Kernel LLM para el notebook de [pluma](../README.md).

Celda LLM: el `source` es un prompt, el `output` es la respuesta del modelo. Usa la fachada [`pluma-llm`](../pluma-llm/README.md) — el backend se elige por config global del notebook (Anthropic, Gemini, Cohere, OpenAI-compatible, Mock). Cacheo BLAKE3 por (prompt + model + params) para que rerun-sin-cambios sea gratis.

## API

```rust
use pluma_notebook_kernel_llm::LlmKernel;

let k = LlmKernel::con_config(LlmConfig::from_env()?);
let outputs = k.correr(&celda).await?;
```

## Deps

- [`pluma-notebook-core`](../pluma-notebook-core/README.md), [`pluma-llm`](../pluma-llm/README.md)
