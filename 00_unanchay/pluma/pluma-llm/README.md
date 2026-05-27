# pluma-llm

> `Arc<dyn ChatClient>` facade with autodetect for [pluma](../README.md).

`LlmConfig { kind, model?, api_key?, endpoint? }` + `build_client(&cfg) -> Arc<dyn ChatClient>`. Five backends: Anthropic, Gemini, Cohere, OpenAI-compatible (DeepSeek/Ollama/proxies), Mock. `from_env()` autodetects via `PLUMA_LLM_BACKEND` or the first present env key; final `Mock` fallback so the process never fails over missing credentials.

`LlmConfig` is (de)serializable as JSON/TOML — apt for app config files.

## API

```rust
use pluma_llm::{build_client, LlmConfig};

let chat = build_client(&LlmConfig::from_env()?);
let resp = chat.send(messages).await?;
```

## Deps

- [`pluma-llm-core`](../pluma-llm-core/README.md) + optional backends
