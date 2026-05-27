# pluma-llm-cohere

> Backend Cohere para [pluma](../README.md).

Implementa `ChatClient` contra la API Chat de Cohere (Command R, Command R+). Streaming + RAG citations cuando se incluyen documents en la request. Lee `COHERE_API_KEY`.

## API

```rust
use pluma_llm_cohere::CohereClient;

let chat = CohereClient::new("command-r-plus", api_key);
```

## Deps

- [`pluma-llm-core`](../pluma-llm-core/README.md)
- `reqwest`, `serde_json`
