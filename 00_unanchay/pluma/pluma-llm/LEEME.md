# pluma-llm

> Fachada `Arc<dyn ChatClient>` con autodetect para [pluma](../README.md).

`LlmConfig { kind, model?, api_key?, endpoint? }` + `build_client(&cfg) -> Arc<dyn ChatClient>`. Cinco backends: Anthropic, Gemini, Cohere, OpenAI-compatible (DeepSeek/Ollama/proxies), Mock. `from_env()` autodetecta por `PLUMA_LLM_BACKEND` o por la primera env key presente; fallback final `Mock` para que el proceso jamás falle por credenciales ausentes.

`LlmConfig` (de)serializable JSON/TOML — apto para config files de apps.

## API

```rust
use pluma_llm::{build_client, LlmConfig};

let chat = build_client(&LlmConfig::from_env()?);
let resp = chat.send(messages).await?;
```

## Deps

- [`pluma-llm-core`](../pluma-llm-core/README.md) + backends opcionales
