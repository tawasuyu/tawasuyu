# pluma-llm-mock

> Backend mock determinista para [pluma](../README.md). Tests + fallback.

Implementa `ChatClient` sin red. Devuelve respuestas según un script preconfigurado o ecos del input. Útil para:

- **Tests** verdes sin credenciales.
- **Fallback** automático cuando no hay env keys (la fachada de [`pluma-llm`](../pluma-llm/README.md) cae acá).
- **CI** offline.

## API

```rust
use pluma_llm_mock::MockClient;

let chat = MockClient::echo();
let chat = MockClient::scripted(vec!["resp1", "resp2"]);
```

## Deps

- [`pluma-llm-core`](../pluma-llm-core/README.md)
- Sin deps de red
