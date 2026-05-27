# pluma-llm-anthropic

> Anthropic (Claude) backend for [pluma](../README.md).

`ChatClient` impl against the Anthropic API. Supports the current Claude model catalog + SSE streaming + tool use + prompt caching when the message justifies it. Reads `ANTHROPIC_API_KEY` from env.

## API

```rust
use pluma_llm_anthropic::AnthropicClient;
use pluma_llm_core::ChatClient;

let chat = AnthropicClient::new("claude-opus-4-7", api_key);
```

## Deps

- [`pluma-llm-core`](../pluma-llm-core/README.md)
- `reqwest`, `eventsource-stream`, `serde_json`
