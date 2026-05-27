# pluma-llm-gemini

> Google Gemini backend for [pluma](../README.md).

`ChatClient` impl against the Gemini API. Pro / Flash models + streaming + native system instructions. Reads `GEMINI_API_KEY` or `GOOGLE_API_KEY`.

## API

```rust
use pluma_llm_gemini::GeminiClient;

let chat = GeminiClient::new("gemini-2.0-pro", api_key);
```

## Deps

- [`pluma-llm-core`](../pluma-llm-core/README.md)
- `reqwest`, `serde_json`
