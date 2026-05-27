# pluma-llm-openai-compatible

> Backend genérico OpenAI-compatible para [pluma](../README.md).

Cualquier endpoint con la API de OpenAI funciona: **OpenAI**, **DeepSeek**, **Ollama**, **vLLM**, **Together**, proxies y self-hosted. Config explícita: `endpoint` + `api_key` (opcional, según el proveedor) + `model` (default depende del backend).

## API

```rust
use pluma_llm_openai_compatible::OpenAiClient;

let chat = OpenAiClient::new("https://api.deepseek.com", api_key, "deepseek-chat");
```

## Deps

- [`pluma-llm-core`](../pluma-llm-core/README.md)
- `reqwest`, `serde_json`, `eventsource-stream`
