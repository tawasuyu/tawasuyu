# pluma-llm-core

> `ChatClient` trait + shared LLM types for [pluma](../README.md).

Defines the abstraction every backend implements. Types: `Message`, `Role`, `Tool`, `ChatRequest`, `ChatResponse`, `ChatStream`. Designed so switching backend is changing ONE config enum variant.

## API

```rust
pub trait ChatClient: Send + Sync {
    async fn send(&self, req: ChatRequest) -> Result<ChatResponse>;
    async fn stream(&self, req: ChatRequest) -> Result<ChatStream>;
}
```

## Deps

- `serde`, `async-trait`, `futures-core`
- Zero HTTP / provider-specific deps
