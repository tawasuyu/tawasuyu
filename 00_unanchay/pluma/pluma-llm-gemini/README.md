# pluma-llm-gemini

> Backend Google Gemini para [pluma](../README.md).

Implementa `ChatClient` contra la API de Gemini. Modelos Pro / Flash + streaming + system instruction nativa. Lee `GEMINI_API_KEY` o `GOOGLE_API_KEY`.

## API

```rust
use pluma_llm_gemini::GeminiClient;

let chat = GeminiClient::new("gemini-2.0-pro", api_key);
```

## Deps

- [`pluma-llm-core`](../pluma-llm-core/README.md)
- `reqwest`, `serde_json`
