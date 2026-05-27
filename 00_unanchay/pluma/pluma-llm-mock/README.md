# pluma-llm-mock

> Deterministic mock backend for [pluma](../README.md). Tests + fallback.

`ChatClient` impl without network. Returns answers based on a preconfigured script or echoes the input. Useful for:

- **Tests** green without credentials.
- **Fallback** when no env keys are present (the [`pluma-llm`](../pluma-llm/README.md) facade falls back to it).
- **CI** offline.

## API

```rust
use pluma_llm_mock::MockClient;

let chat = MockClient::echo();
let chat = MockClient::scripted(vec!["resp1", "resp2"]);
```

## Deps

- [`pluma-llm-core`](../pluma-llm-core/README.md)
- No network deps
