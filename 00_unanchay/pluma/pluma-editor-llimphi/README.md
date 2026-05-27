# pluma-editor-llimphi

> Editor visual Llimphi de [pluma](../README.md).

UI: panel izquierdo con lista de documentos (de [`pluma-store`](../pluma-store/README.md)), centro con [`text-editor`](../../../02_ruway/llimphi/widgets/text-editor/README.md) sobre el cuerpo concatenado, panel derecho con LLM (cycler de backends via [`pluma-llm`](../pluma-llm/README.md)), historial, diff. Cada save dispara [`pluma-editor-cuerpo::diff`](../pluma-editor-cuerpo/README.md).

Selector de modelo en runtime: botón cíclico Mock → Gemini → Anthropic → DeepSeek → Cohere → Ollama → Mock. Click → `build_client(...)` reconstruye `Arc<dyn ChatClient>` en vivo; si el backend no está configurado, conserva el anterior con mensaje de error.

## Deps

- Todos los crates `pluma-*`
- [`llimphi-ui`](../../../02_ruway/llimphi/) + widgets `text-editor`, `tree`, `tabs`, `splitter`
- [`wawa-config-llimphi`](../../../shared/wawa-config-llimphi/)
