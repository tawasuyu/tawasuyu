# pluma-llm-cohere

> Cohere backend for [pluma](../README.md).

`ChatClient` impl against Cohere's Chat API (Command R, Command R+). Streaming + RAG citations when documents are included in the request. Reads `COHERE_API_KEY`.

## API

```rust
use pluma_llm_cohere::CohereClient;

let chat = CohereClient::new("command-r-plus", api_key);
```

## Deps

- [`pluma-llm-core`](../pluma-llm-core/README.md)
- `reqwest`, `serde_json`
