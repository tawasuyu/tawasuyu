# pluma-llm-core

> Trait `ChatClient` + tipos compartidos de LLM para [pluma](../README.md).

Define la abstracción que cualquier backend implementa. Tipos: `Message`, `Role`, `Tool`, `ChatRequest`, `ChatResponse`, `ChatStream`. Pensado para que el switch de backend sea cambiar UNA variante del enum de config.

## API

```rust
pub trait ChatClient: Send + Sync {
    async fn send(&self, req: ChatRequest) -> Result<ChatResponse>;
    async fn stream(&self, req: ChatRequest) -> Result<ChatStream>;
}
```

## Deps

- `serde`, `async-trait`, `futures-core`
- Cero deps de HTTP o providers específicos
