# pluma-llm-anthropic

> Backend Anthropic (Claude) para [pluma](../README.md).

Implementa `ChatClient` contra la API de Anthropic. Soporta el catálogo de modelos Claude actual + streaming SSE + tool use + prompt caching cuando el mensaje lo justifica. Lee `ANTHROPIC_API_KEY` del entorno.

## API

```rust
use pluma_llm_anthropic::AnthropicClient;
use pluma_llm_core::ChatClient;

let chat = AnthropicClient::new("claude-opus-4-7", api_key);
```

## Deps

- [`pluma-llm-core`](../pluma-llm-core/README.md)
- `reqwest`, `eventsource-stream`, `serde_json`
