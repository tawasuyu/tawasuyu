# pluma-llm-openai-compatible

> Generic OpenAI-compatible backend for [pluma](../README.md).

Any OpenAI-API endpoint works: **OpenAI**, **DeepSeek**, **Ollama**, **vLLM**, **Together**, proxies and self-hosted. Explicit config: `endpoint` + `api_key` (optional, depending on provider) + `model` (default depends on backend).

## API

```rust
use pluma_llm_openai_compatible::OpenAiClient;

let chat = OpenAiClient::new("https://api.deepseek.com", api_key, "deepseek-chat");
```

## Deps

- [`pluma-llm-core`](../pluma-llm-core/README.md)
- `reqwest`, `serde_json`, `eventsource-stream`
