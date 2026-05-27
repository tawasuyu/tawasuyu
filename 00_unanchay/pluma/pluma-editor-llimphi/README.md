# pluma-editor-llimphi

> Visual Llimphi editor for [pluma](../README.md).

UI: left panel with document list (from [`pluma-store`](../pluma-store/README.md)), center with [`text-editor`](../../../02_ruway/llimphi/widgets/text-editor/README.md) over the concatenated body, right panel with LLM (backend cycler via [`pluma-llm`](../pluma-llm/README.md)), history, diff. Each save triggers [`pluma-editor-cuerpo::diff`](../pluma-editor-cuerpo/README.md).

Runtime model selector: cyclic button Mock → Gemini → Anthropic → DeepSeek → Cohere → Ollama → Mock. Click → `build_client(...)` rebuilds the `Arc<dyn ChatClient>` live; if a backend isn't configured, keeps the previous one with an error message.

## Deps

- All `pluma-*` crates
- [`llimphi-ui`](../../../02_ruway/llimphi/) + widgets `text-editor`, `tree`, `tabs`, `splitter`
- [`wawa-config-llimphi`](../../../shared/wawa-config-llimphi/)
