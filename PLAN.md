# Plan maestro gioser

> Estado al **2026-05-26**: monorepo nacido, 4 cuadrantes consolidados, ~210 crates compilando, **GPUI extinto** — todas las apps pasaron a Llimphi.

## 0. Cartografía

```
gioser/
├── 00_unanchay/   PERCIBIR  — pluma · khipu · rimay · chaka · pineal · puriy
├── 01_yachay/     CONOCER   — cosmos · dominium · nakui
├── 02_ruway/      HACER     — mirada · shuma · nahual · chasqui · takiy · llimphi
├── 03_ukupacha/   RAÍZ      — arje · wawa · agora · minga
├── shared/                  — sandokan · auth · card · ssh · format
└── web/                     — landing sobria (no producto)
```

## 1. Lo hecho (2026-05-25 / 2026-05-26)

1. **Migración estructural**: brahman (188 crates) + eternal (12) + dominium (1) → gioser, 214 crates en workspace + 13 en wawa excluido. Historia git preservada (336 commits + 478 brahman + 56 eternal).
2. **Rename semántico**: 344 cambios en Cargo.tomls + 1668 en .rs. Nombres antiguos (`fana-*`, `charka-*`, `cosmobiologia-*`, `eternal-*`, `brahman-*`, `agorapura-*`, `barra-*`, `revista-*`, `yachay-core`, `verbo-*`, `badu-*`, `formato`) reemplazados por los canónicos.
3. **Landing sobria**: plano cartesiano SVG estático + visor pluma (`web/gioser-web`, 38 LOC).
4. **Llimphi**: 5 crates (`hal/raster/layout/text/ui`) verdes en hardware. Texto vía parley (shaping completo, fallback CJK/emoji vía fontique). Bucle Elm con hit-test funcional.
5. `cargo check --workspace` pasa.
6. **Canal de release wawa** (2026-05-26): `format::Canal` + `RaizFirmada` + `mensaje_a_firmar`, `akasha::MensajeAkasha::AnunciarCanal` (cuarta variante). Kernel ingesta el DAG y traza; verificación de firma + re-anclaje quedan para userspace (app `mudanza`, pendiente). 8/8 tests format, 7/7 tests akasha. Distribución/actualización en wawa: paquete = objeto, versión = hash, repo = canal firmado por agora, update = re-anclar superbloque (atómico), rollback = re-anclar raíz vieja del log.
7. **Pluma multilienzo — andamiaje** (2026-05-26): cuatro de las siete fases del §11.7 ya en `main`.
   - `pluma-cuerpo` (9/9 tests): Cuerpo + MetaCuerpo + Intencion (Original/Traduccion/Tono/Resumen/Reescritura/Anotacion/Custom), `valida_consistencia`, `es_stale(modificado_madre_en)`, postcard roundtrip.
   - `pluma-align` (7/7 tests): Alineamiento(atom_a, atom_b, fuerza∈[0,1], origen, fresco), OrigenAlineamiento (Manual/Embeddings/Derivado, todos con timestamp), CartaHebras + `marcar_stale_anteriores_a`, alineadores `alinear_uno_a_uno` y `alinear_explicito`.
   - `pluma-transform` (5/5 tests): TipoTransformacion (Identidad/Traducir/Tono/Resumir/Reescribir/Custom{Rhai}), trait Ejecutor (no toca reloj — recibe `ahora` para tests deterministicos), `EjecutorIdentidad` end-to-end sin LLM (hija comparte UUIDs con madre, carta 1↔1 Derivada).
   - `pluma-editor-llimphi::multilienzo` (5 tests nuevos + demo ejecutable): vista HStack de columnas + carriles de hebras. Hebras pintadas via `View::paint_with` con vello strokes; color por origen (Derivada verde / Embeddings azul-modulado-por-fuerza / Manual ámbar / Stale gris), trazo punteado para stale. `precomputar_hebras` robusto a la convención atom_a/atom_b. Demo `multilienzo_demo` con 3 cuerpos (es / qu runa simi / en resumen) muestra los 4 estados y un caso 5→2 (resumen condensando varios párrafos).
   - **Pendiente** (§11.7 pasos 5-7): conectar `Traducir` a `rimay-localize` (cuerpo derivado real es→qu), conectar `alinear_por_embeddings` a `iniy` (saturación + stale detection automáticos), transformaciones LLM completas (Tono/Resumir/Reescribir) y UI completa (focus mode, búsqueda transversal, vista matriz, inline previews, federación minga).
8. **Pluma multilienzo — backends real** (2026-05-26): dos crates nuevos cierran el flujo end-to-end con datos auténticos.
   - `pluma-align-embeddings` (5/5 tests): `alinear_por_embeddings(cuerpo_a, cuerpo_b, atoms, &dyn Provider, params, ahora)` async sobre `rimay_verbo_core::Provider`. Matriz NxM de coseno, dos políticas (`MejorParaCadaA` permite N→1, `MutuoMejor` solo 1↔1 y descarta ambigüedades). Trabaja con cualquier backend que implemente `Provider`: `MockProvider` determinista para tests/demo, BGE local o Cohere remoto cuando se enchufe. `OptimoGlobal` (Hungarian) declarado, no implementado todavía.
   - `pluma-transform-tabla` (6/6 tests): `EjecutorTraducirTabla { tabla: HashMap<Uuid_madre, String>, lengua_destino, branch_suffix }`. Honesto sobre la procedencia: no genera texto — la traducción la mete quien sea (humano, LLM externo, traductor automático). El ejecutor solo cuece la tabla en `Cuerpo` hija con atoms nuevos + `CartaHebras` Derivada 1↔1. Huecos en la tabla → párrafo de la hija ausente (queda huérfano). Valida que `lengua_destino` del `TipoTransformacion::Traducir` coincida con la del ejecutor.
   - **Demo end-to-end** (`cargo run -p pluma-editor-llimphi --example multilienzo_demo --release`): cuerpo es → `EjecutorTraducirTabla` → cuerpo qu con atoms reales (Uuids propios, branch `es-qu`, Intencion::Traduccion) → carta es↔qu Derivada del ejecutor. Cuerpo en (resumen, manual). Carta qu↔en calculada por `alinear_por_embeddings` con `MockProvider`. Una hebra marcada stale a mano para ver el trazo punteado.
   - **Pendiente** restante (§11.7 paso 7): backend LLM para `EjecutorTraducirLLM` que GENERE la tabla, ejecutores Tono/Resumir/Reescribir, y UI completa (focus mode, búsqueda transversal, vista matriz, inline previews, federación minga).
9. **Stack LLM agnóstico de proveedor — completo** (2026-05-26). Cuatro crates nuevos para abrir transformaciones generadas por LLM dentro de pluma sin acoplar a ningún proveedor concreto.
   - `pluma-llm-core` (3/3 tests): trait `ChatClient` async + `ChatRequest` (system opcional, messages user/assistant, max_tokens, temperature) + `ChatResponse` + `ChatUsage` con cache_read/creation_input_tokens explícitos (visibilidad del ahorro real de prompt caching) + `ChatError` (AuthMissing/Invalid, RateLimited, Network, Backend, Cancelled). Cero dependencias de proveedor.
   - `pluma-llm-mock` (5/5): `MockChatClient` con tabla `(substring → respuesta)` + fallback al eco con prefijo configurable. Determinista, sin red, ChatUsage cero. Para tests rápidos de `pluma-transform-llm`.
   - `pluma-llm-anthropic` (4/4 + doctest): cliente Messages API con prompt caching del system ENCENDIDO por defecto — bloque `{type:text, text, cache_control:{type:ephemeral}}`. N requests con el mismo system pagan input cacheado en las sucesivas. Modelo default `claude-sonnet-4-6`, override por `with_model`. API key via `ANTHROPIC_API_KEY` o `with_api_key`. Mapea 401/403→AuthInvalid, 429→RateLimited.
   - `pluma-transform-llm` (7/7): cuatro ejecutores (`EjecutorTraducirLlm`, `EjecutorTonoLlm`, `EjecutorResumirLlm`, `EjecutorReescribirLlm`) sobre cualquier `ChatClient`. Patrón compartido en `ejecutar_lote()`: una request por átomo de la madre con system fijo cacheado + user con el texto, respuesta limpiada de comillas envolventes, materialización en `Cuerpo` hija + `CartaHebras` Derivada en una pasada. Por restricción del trait `Ejecutor::aplicar` (que da solo Uuids, no texto), exponen método inherente `aplicar_con_atoms(t, madre, atoms_index, ahora)`; el `impl Ejecutor::aplicar` falla con guía clara.
   - **Refactor base**: `pluma-transform::Ejecutor` pasó a async (decisión adelantada para no migrar API cuando llegue otro adapter remoto). `EjecutorIdentidad` y `EjecutorTraducirTabla` ahora son `async fn` sin awaits internos. 11/11 tests pasan tras la migración.
   - **Stack LLM transparente — completo** (2026-05-26, mismo día). Tres backends adicionales + fachada que despacha por config: el caller habla con `Arc<dyn ChatClient>` y cambia de IA con UNA variante del enum.
     - `pluma-llm-openai-compatible` (5/5 + doctest): adapter genérico shape OpenAI. Cubre **DeepSeek** (preset `deepseek_from_env`, env `DEEPSEEK_API_KEY`, default `deepseek-chat`) y **Ollama local** (preset `ollama_local`, sin auth, endpoint `http://localhost:11434/v1/chat/completions`) de un solo golpe. Funciona con cualquier proxy compatible (Groq, Together, vLLM self-hosted). Lee `prompt_cache_hit_tokens` y `prompt_tokens_details.cached_tokens` del usage cuando el servidor los reporta (DeepSeek auto-cachea prompts >1k tokens server-side).
     - `pluma-llm-gemini` (5/5 + doctest): adapter Gemini con shape propia (`contents` con `parts`, `systemInstruction` top-level, roles `user`/`model`). API key vía `GEMINI_API_KEY` o `GOOGLE_API_KEY`. Default `gemini-2.5-flash`. Lee `cachedContentTokenCount` para reportar caching.
     - `pluma-llm` (6/6 + doctest): **fachada transparente**. `LlmConfig{kind, model?, api_key?, endpoint?}` + `build_client(&cfg) -> Arc<dyn ChatClient>`. Cinco backends: Anthropic, Gemini, DeepSeek, Ollama, Mock. `from_env()` autodetecta según `PLUMA_LLM_BACKEND` o la primera env key presente; fallback final Mock para que el proceso jamás falle por credenciales ausentes. `LlmConfig` (de)serializable JSON/TOML — apto para config files de apps. Ollama exige `model` explícito (sin default seguro); resto cae a defaults documentados.
   - **Ciclo cerrado LLM ↔ grafo** (2026-05-26, sigue). Refactor + crate de pegamento + demo end-to-end real:
     - **Refactor `pluma-transform-llm`**: los cuatro ejecutores pierden el genérico `<C>` y usan `Arc<dyn ChatClient>`. Cada uno expone `new<C>(chat, ...)` para clients concretos y `from_arc(arc, ...)` para el `Arc<dyn>` que devuelve `pluma_llm::build_client`. Encaja directo con el factory transparente sin gimnasia de tipos. 7/7 tests pasan tras el refactor.
     - `pluma-graph-transform` (3/3): pegamento mínimo. `indice_atoms(&graph)` construye el `HashMap<Uuid, &NarrativeAtom>` para `aplicar_con_atoms`; `persistir_producto(&mut graph, producto) -> (Cuerpo, CartaHebras)` mete los `atoms_nuevos` al grafo y devuelve lo que la app necesita después. Sin acoplar `pluma-graph` ni `pluma-transform` entre sí — el pegamento vive en el intersticio.
     - **Demo `multilienzo_llm_demo`**: orquesta las cuatro piezas (LLM transparente vía `from_env`, ejecutor LLM, persistencia en grafo, embeddings vía daemon o mock). Sin credenciales sustituye el Mock por uno pre-poblado con traducciones predecibles — el demo arranca "como si tradujera" aun sin red. Con `ANTHROPIC_API_KEY`/`GEMINI_API_KEY`/`DEEPSEEK_API_KEY` configurada (o Ollama local pulled) traduce de verdad cada párrafo. Documenta los cinco modos de arranque en cabecera.
   - **Persistencia cerrada** (2026-05-26 sigue). `pluma-store` extendido con módulo `multilienzo`: `PlumaStore` abre un sólo `sled::Db` con cuatro trees nominales (atoms, cuerpos, transformaciones, cartas) que cubren todo el estado del haz. Cartas indexadas por `cuerpo_a||cuerpo_b` (32 bytes) con búsqueda bidireccional opcional. Cartas sin par anotado se rechazan al persistir — el dato del par ES la clave. Backend sled+bincode preexistente; `GraphStore` legacy queda intacto. 10/10 tests (8 nuevos + 2 viejos), incluyendo close/reopen.
   - **Validación Gemini real**: smoke (`pluma-llm-gemini --example smoke`) contra `api.anthropic.com`... contra `generativelanguage.googleapis.com` confirmó que el adapter mapea correctamente la shape `contents/parts/systemInstruction` ↔ `ChatRequest/ChatResponse`. Una request de 28+2 tokens fue suficiente para validar end-to-end.
   - **Tres piezas finales del día**:
     - **`multilienzo_store_demo`** (ejemplo): primer arranque genera con LLM y persiste en `~/.cache/gioser/pluma-multilienzo/pluma.sled`; siguientes arranques cargan instantáneo. `MULTILIENZO_RESET=1` limpia. Cierra el loop "lo que generaste ayer sigue ahí hoy".
     - **`multilienzo_dinamico_demo`** (ejemplo): toolbar con 4 botones (`→qu`, `→en`, `tono formal`, `resumir 30p`). Click → `handle.spawn(...)` thread con runtime tokio efímero → ejecutor LLM transparente → `Msg::LlmListo` repinta con columna nueva. `model.en_curso` deshabilita botones durante el trabajo (un job a la vez). Vista LLM-aware sobre la vista multilienzo.
     - **`pluma-notebook-kernel-llm`** (7/7 tests): `LlmKernel` implementa `pluma_notebook_exec::Kernel`. Lenguajes `llm-prompt | llm-traducir-{lang} | llm-tono-{etiqueta} | llm-resumir[-N] | llm-reescribir`. Conecta el notebook (DAG reactivo de celdas) con el LLM transparente: una celda traduce, su dependiente la resume; al editar la madre, el notebook reactivo re-corre el cono y todo se regenera con el LLM activo.
     - **`multilienzo_completo_demo`** (ejemplo): la "app" del multilienzo. Toolbar LLM dinámica como `multilienzo_dinamico_demo` + persistencia automática como `multilienzo_store_demo`, todo en un solo binario. Cada transformación se persiste (atom + cuerpo + transformación + carta + flush) ANTES de actualizar el modelo, así una caída del proceso entre los dos pasos no deja estado a medias. Status bar en vivo mostrando `N cuerpos · M cartas`.
     - **`notebook_llm_demo`** (ejemplo CLI en `pluma-notebook-kernel-llm`): construye un notebook con 4 celdas (markdown fuente + traducir-qu + tono-formal + resumir-20), corre `run_all`, imprime cada output. Sin GUI — válido en CI o servidor sin display. Para que el mock distinga las tres acciones aun cuando el user message es el mismo texto, `pluma-llm-mock` ganó `con_respuesta_si_system(substring, resp)` (6/6 tests).
   - **Cuatro upgrades del editor + un sexto backend** (2026-05-26, fin del día).
     - **`pluma-md::import`** (9 tests nuevos, 14/14 total): `parse_md(md, branch_id, nombre, ahora) -> DocumentoImportado` produce un cuerpo madre `Original` con un `NarrativeAtom` por bloque markdown (párrafos, listas, encabezados con prefijo `# `/`## `, code blocks, blockquotes). Formato inline (negrita/cursiva/code) se aplana — el LLM no lidia con asteriscos. Desbloquea importar archivos reales como cuerpos: `parse_md(fs::read_to_string("README.md")?, "es", "readme", ahora)` arranca un haz multilienzo sobre cualquier doc.
     - **Scroll horizontal nativo** en `multilienzo_completo_demo`: `App::on_wheel` con eje X del touchpad o `Shift+wheel-Y`. Sin esto, con más de 3-4 cuerpos los del extremo se cortaban; ahora se recorre toda la fila.
     - **Focus mode + búsqueda transversal**: en `multilienzo_completo_demo`, botón "solo madre / todos" oculta derivados; búsqueda transversal sin widget de input (capturada vía `App::on_key` directo: tecla acumula, Backspace borra, Esc limpia). Cada átomo cuyo `content` contenga el query se pinta con fondo destacado en TODAS las columnas visibles — el ojo recorre las correspondencias en una mirada. `pluma-editor-llimphi::multilienzo` ganó `multilienzo_view_resaltado(..., resaltar: &str)` con helper privado `mezclar(a, b, t)` para interpolación de color.
     - **`pluma-llm-cohere`** (4/4 tests): sexto backend del trait. Cohere v2 chat. Request shape OpenAI-like; response shape Anthropic-like (`message.content: [{type:text, text}]`) — por eso crate aparte. API key `COHERE_API_KEY`, default `command-a-03-2025`. Cableado al `pluma-llm` router: `BackendKind::Cohere` autodetectable; `from_env` lo elige si `COHERE_API_KEY` está presente (cuarto en la cadena).
   - **Cinco upgrades finales del editor + parser docx** (2026-05-26, cierre):
     - **Persistencia del estado UI**: `pluma-store::EstadoUi {solo_madre, busqueda, scroll_x, backend_llm}` en un tree dedicado del sled. El `multilienzo_completo_demo` lo restaura al abrir y lo persiste tras cada cambio (scroll, focus toggle, cada tecla de búsqueda). Cierra el demo a media búsqueda y la próxima vez te encuentra en el mismo párrafo resaltado. 11/11 tests del store.
     - **Botones "tocar madre" + "regenerar stale (N)"**: cierran el ciclo edit→stale→regen sin editor de texto. "Tocar madre" actualiza `modificado_en`; todas las hijas Derivadas quedan stale. "Regenerar stale" detecta la primera, busca la `Transformacion` original en el store, traduce el `TipoTransformacion` a `TrabajoLlm` via helper, y la relanza con la madre actualizada. Solo Traducir/Tono/Resumir auto-regenerables.
     - **Selector de modelo en runtime**: botón cíclico `Mock → Gemini → Anthropic → DeepSeek → Cohere → Ollama → Mock`. Click → `build_client(...)` reconstruye `Arc<dyn ChatClient>` en vivo; si el backend no está configurado, conserva el anterior con mensaje de error. Persistido como `backend_llm` en EstadoUi.
     - **`shared/foreign-docx`** (5/5 + doctest): importa `.docx` como cuerpo madre. Parser SAX minimal con `zip` + `quick-xml` (sin `docx-rs`): un `<w:p>` → un `NarrativeAtom`, runs `<w:t>` concatenados, formato descartado. Misma shape de `DocumentoImportado` que `pluma_md::import`. Manejo claro de no-zip y zip-sin-document.xml.
     - **Editor inline MVP**: botón "editar madre" muta el primer átomo añadiendo `⟨edit N⟩`, llama `atom.set_content(...)` (re-hashea + `PendingEvaluation`) y `graph.propagate_mutation(...)` (propaga a todos los descendientes del DAG narrativo de `pluma-core`). El "editor" es degenerado pero demuestra el ciclo reactivo completo edit→propagate→stale→regenerate sin necesitar widget de text-area. Edit con texto libre queda como follow-up.
   - **Sincronía cuerpo ↔ buffer plano para editor único** (2026-05-26, fin de día). La pieza conceptual del editor multiline que entiende "regiones de párrafo": un solo `text-editor` (ropey IDE de Llimphi) sobre todos los átomos concatenados, las hebras+transformaciones siguen funcionando porque cada párrafo mantiene su `Uuid`.
     - `pluma-editor-cuerpo` (9/9 tests): `EditorCuerpo { texto, atom_ids }`. `from_cuerpo(c, atoms)` concatena con `SEPARADOR = "\n\n"`. `parrafos()` recupera el split actual. `diff(atoms_originales) -> Vec<CambioAtom>` con greedy por contenido: párrafos coincidentes se saltan, distintos emiten `Mutar` reusando el `Uuid` (hebras vivas), sobrantes emiten `Crear` o `Eliminar`. `aplicar_cambios(cambios, nuevos_ids)` extiende/remueve el `atom_ids` tras persistir.
     - **Integración visual** (cierre, 2026-05-26): `pluma-editor-llimphi::cuerpo_ide` (6/6 tests) une `EditorCuerpo` con el widget `text-editor` IDE (ropey + `EditorState` + multi-cursor + undo + viewport scroll). `CuerpoIde::from_cuerpo` carga el texto plano en el buffer; `apply_key` reenvía teclas marcando `pendiente_sync`; `diff(&atoms)` vuelca el buffer y devuelve los `CambioAtom`; `aplicar_cambios_locales` alinea `atom_ids` con los Uuids recién creados. `cuerpo_ide_view` envuelve `text_editor_view_highlighted` (Language::Plain por default para prosa). Demo `cuerpo_ide_demo`: cuerpo de 5 átomos, edición libre, `Ctrl+S` corre el diff y persiste mutando `HashMap<Uuid, NarrativeAtom>` + reconstruyendo `cuerpo.orden`. Las hijas Derivadas quedan stale y el botón "regenerar stale" del multilienzo cierra el ciclo.
10. **Embedder global de la suite — completo** (2026-05-26). La arquitectura ya existía como diseño (daemon socket-Unix, cliente que se presenta como `Provider`); en esta iteración se cierran las dos piezas que faltaban.
   - `rimay-verbo-daemon-bin` (binario `verbo-daemon`): levanta un `Provider` en RAM y lo sirve. CLI con clap: `--socket` (default `$XDG_RUNTIME_DIR/verbo.sock`, fallback `/tmp/verbo-{uid}.sock`), `--provider {mock|fastembed}` (extensible vía `ValueEnum`), `--dim N` (solo mock). Una instancia = un modelo; multi-modelo = N daemons en N sockets.
   - `rimay-verbo-fastembed` (`FastembedProvider`): backend real local sin API key, ONNX-Runtime en CPU vía `fastembed-rs`. Default `multilingual-e5-small` (384d, multilingüe — sirve es/qu/en/otros sin cambiar de modelo). Descarga el ONNX a `~/.cache/fastembed` al primer arranque. Catálogo declarado de modelos E5/BGE con `nombre_canonico` + `dimension`; modelo nuevo no declarado falla al primer embed con `BadDimension` — forzando declaración explícita. Adapta sync→async vía `tokio::task::spawn_blocking` + `tokio::sync::Mutex`; tests unit sobre catálogo, integration `#[ignore]` que descarga modelo y verifica coseno≈1 para textos idénticos.
   - **Consumo**: cualquier consumidor de `Provider` en la suite (`pluma-semantic`, `pluma-align-embeddings`, `khipu`, `chasqui`) cambia `MockProvider::default()` por `DaemonClient::connect("$XDG_RUNTIME_DIR/verbo.sock").await?` y ya tiene embeddings semánticos reales sin cargar nada propio.
   - **Pendiente**: `verbo-cohere` (backend remoto opcional), feature flag `bin` o crate `verbo-daemon-systemd` con unit file si se quiere arranque automático con el sesión del usuario.

## 2. Hito #1 — Llimphi (gráfico soberano)

**Objetivo:** Reemplazar GPUI completamente. Motor propio basado en `wgpu + vello + taffy + DAG monádico`.

Ver [`02_ruway/llimphi/SDD.md`](02_ruway/llimphi/SDD.md) para el spec completo.

### Fases secuenciales

| Fase | Crate | Deps | Hito visible |
|---|---|---|---|
| 1. HAL | `llimphi-hal` | `wgpu` + `winit` | Pantalla gris plomo a 144 Hz |
| 2. Raster | `llimphi-raster` | `vello` | Grafo de un nodo con AA perfecto |
| 3. Layout | `llimphi-layout` | `taffy` | Paneles redimensionados < 1 ms/frame |
| 4. UI | `llimphi-ui` | (puro Rust) | Bucle Elm completo: input→update→view→layout→raster |

## 3. Hito #2 — Puriy (navegador soberano Servo+Llimphi)

**Objetivo:** Navegador web propio que corre idéntico en mirada (Wayland) y en wawa (bare-metal) por el mismo trait `Surface` de Llimphi.

Ver [`00_unanchay/puriy/SDD.md`](00_unanchay/puriy/SDD.md).

| Fase | Crate | Hito |
|---|---|---|
| 1. Core | `puriy-core` | Sesiones/tabs/history puros (sin gráficos) |
| 2. Engine | `puriy-engine` | Embed de Servo, parsea DOM, renderiza viewport en textura wgpu |
| 3. Chrome | `puriy-llimphi` | Toolbar+tabs+address bar sobre llimphi-ui |
| 4. App | `puriy-app` | `puriy URL` abre y carga sitio en mirada o framebuffer |

**Bloqueado por:** Hito #1 (Llimphi fases 1-4). `puriy-core` se puede arrancar en paralelo (puro Rust).

## 4. Hito #3 — Migración GPUI → Llimphi

Cuando Llimphi tenga las 4 fases verdes, portar:

| App | Crate(s) actual(es) | Acción |
|---|---|---|
| Nahual shell + viewers (5 apps + 8 libs + 12 widgets) | `02_ruway/nahual/*` | Reemplazar capa GPUI; conservar lógica de dominio |
| Mirada UI (launcher, portal, greeter) | `02_ruway/mirada/mirada-{launcher,portal,greeter}` | Idem · `mirada-greeter` portado a Llimphi (2026-05-25). |
| Pluma editor | ~~`00_unanchay/pluma/pluma-editor-gpui`~~ | ✅ `pluma-editor-llimphi` (2026-05-25). |
| Dominium canvas + app | ~~`01_yachay/dominium/dominium-canvas-gpui`~~ + ~~`dominium-app`~~ (GPUI) | ✅ `dominium-canvas-llimphi` + ✅ `dominium-app-llimphi` (2026-05-25): la app monta la cadena `core→physics→iso→render-plan→canvas-llimphi`, corre un loop de tick ~11 Hz en un `thread::spawn` que reentra al update vía `Handle::dispatch(Msg::Tick)` (`Handle` es `Send + Clone`), y compone status bar + canvas + side panel con botones Play/Pause y Re-sembrar (vía `llimphi-widget-button`). |
| Cosmos canvas + app | ~~`01_yachay/cosmos/cosmos-app`~~ (GPUI) | 🚧 MVP (2026-05-25): `cosmos-canvas-llimphi` traduce `Vec<DrawCommand>` de `cosmos-render::compose_wheel` a primitivas vello (Circle/Line/Polygon) + texto vía llimphi-text con glyphs unicode astrológicos (☉♀♈…). `cosmos-app-llimphi` (binario) arma un RenderModel mock (sin engine real, eternal-sky no compila a WASM) con cuerpos clásicos y lo pinta. `cosmos-{tree,panel,theme}` GPUI borrados (huérfanos tras la caída de `cosmos-app`); cuando se necesite shell completo, los recreamos en Llimphi sobre `llimphi-widget-tree`. Falta integración con cosmos-engine real + módulos overlay. |
| Nakui ERP shell + explorer | ~~`01_yachay/nakui/nakui-ui`~~ + ~~`nakui-explorer`~~ (GPUI) | 🚧 MVP (2026-05-25): `nakui-explorer-llimphi` 1:1 con la versión GPUI (timeline cards + breakdown + banners + polling 2s vía `Handle::spawn_periodic`). `nakui-ui-llimphi` MVP read-only: sidebar de módulos + menú + área principal que listea entities y muestra record counts vía `MetaBackend::list_records`; `NakuiBackend` (WAL + replay + snapshot + auto-compact + executors Rhai) intacto y testeado. Falta el widget Llimphi paralelo a `nahual-widget-meta-form` (2k LOC borradas) para reactivar seed/edit/delete/morphism desde la UI. |

**Regla:** Las apps mantienen su `*-core` agnóstico intacto. Solo cambia el frontend.

## 5. Hitos por dominio (orden no estricto)

### `00_unanchay/`
- **pluma**: cerrar editor (en Llimphi), notebook DAG funcional.
- **khipu**: gravedad semántica usable.
- **rimay**: embeddings via verbo-daemon.
- **chaka**: ampliar subconjunto COBOL (CICS, SQL, dialectos).
- **pineal**: dominio propio, charts vivos.
- **puriy**: ver Hito #2.

### `01_yachay/`
- **cosmos**: cerrar 4 áreas del roadmap Kepler (box graphs → harmonics → AstroCarto → research). Corpus de interpretación pendiente de escritura humana.
- **dominium**: simulador determinista validado.
- **nakui**: ERP usable (módulos inventory/sales/treasury/crm).

### `02_ruway/`
- **mirada**: shell completo + DM en hardware real (Artix laptop con GPU física, no VPS).
- **shuma**: sandbox + baremetal (matilda absorbido) funcional.
- **nahual**: portado a Llimphi.
- **chasqui**: message broker monádico productivo.
- **takiy**: app de composición musical con generador IA de sonidos.
- **llimphi**: ver Hito #1.
- **supay**: modernizar Doom sin tocar su alma — ver `02_ruway/supay/SDD.md`. Fase 0.x (raycaster hardcoded sobre Llimphi con sprites, sector lights, texturas procedurales, disparo, enemies, pickups, game over) entregada 2026-05-25; Fase 1.0 (`supay-core` con FFI + build.rs a doomgeneric, `supay-doom-llimphi` que pinta el framebuffer 320×200 como `View::image`) andamiaje completo entregado, modo stub si vendor no está provisto.

### `03_ukupacha/`
- **arje**: DM end-to-end en hardware real, packaging rootfs+mesa.
- **wawa**: kernel SASOS WASM, expandir hardware soportado.
- **agora**: identidad federada operativa.
- **minga**: P2P VFS productivo.

### `shared/`
- **sandokan**: orquestador hot-swap consumible por shuma y otros.
- **auth, card, ssh, format**: pulir APIs.

## 6. Disciplina técnica permanente

1. **Filesystem = arquitectura**: cada cuadrante es una fase del ciclo de información.
2. **Un dominio = un crate raíz + subcrates plugin**, sin proliferación.
3. **UIs intercambiables** sobre `*-core` agnósticos.
4. **No GPUI** en código nuevo (a partir de hoy). Todo gráfico pasa por Llimphi.
5. **Modularidad horizontal**: splittear crates > 1.500–2.000 LOC.
6. **Commit + push** tras cada bloque, sin pedir permiso (excepto operaciones destructivas).
7. **Smoke test mínimo**: `cargo check --workspace` debe pasar en `main` siempre.

## 6.bis Hito — Distribución y actualización en wawa (Canal de release)

**Estado parcial 2026-05-26.** Lo entregado: ver §1.6. Lo que falta:

| Pieza | Crate / archivo | Estado |
|---|---|---|
| App `mudanza` (daemon userspace) | `03_ukupacha/wawa/apps/mudanza/` | pendiente — suscripción a canales, verificación firma Ed25519 vía agora, descarga DAG delta, syscall `sys_actualizar_raiz` |
| `sys_actualizar_raiz(hash_manifiesto)` | `wawa-kernel/src/wasm/env.rs` + manifiesto.rs | pendiente — validar tipos WASM de apps nuevas antes de re-anclar |
| Ring buffer de últimas N raíces en superbloque | `format::SuperBloque` v3 + `almacen.rs` | pendiente — habilita rollback y menú de boot |
| Menú "anclas recientes" en `wawa-boot` | `wawa-boot/src/main.rs` | pendiente |
| Identidad agora Ed25519 firmable | `01_yachay/agora/agora-core` (o `shared/firma`) | pendiente — primitiva real, hoy `format::Firma` es un transporte sin verificación |
| `mensaje firmable` también en host (constructor de canales) | host-side tool en `wawa-explorer-*` o crate nueva `canalero` | pendiente — emitir AnunciarCanal desde una laptop |

**Decisión clave**: el kernel NO carga criptografía de identidad. Solo ingesta el DAG; toda política vive en userspace.

## 6.ter Hito — Compatibilidad office/PSD y motor de hojas

**Principio**: formatos ajenos entran por puentes (`shared/foreign-*`), nunca al núcleo de las apps. Las apps trabajan siempre en su formato nativo (BLAKE3 + DAG + postcard).

| Pieza | Crate | Propósito | Toca apps existentes |
|---|---|---|---|
| `foreign-docx` | `shared/foreign-docx` | docx ↔ pluma AST (round-trip lossy; lo que no se expresa va a nodo opaco del grafo) | no |
| `foreign-xlsx` | `shared/foreign-xlsx` | xlsx ↔ nakui tabla + AST yupay (fórmulas) | no |
| `foreign-pptx` | `shared/foreign-pptx` | pptx ↔ pluma-deck | no |
| `foreign-psd` | `shared/foreign-psd` | psd ↔ AST de capas tullpu | no |
| `yupay` (motor de fórmulas) | `01_yachay/nakui/yupay-core` + `yupay-fns` | DSL Excel-like (`=SUMA(A1:A10)`, bilingüe es/qu) compilado a Rhai; lambdas y full-Rhai en celdas avanzadas | crate nuevo, **Rhai ya está en el stack** |
| Vista de hoja en `nakui-ui-llimphi` | `01_yachay/nakui/nakui-ui-llimphi` | celdas + headers + freeze panes + pivot views | vista alterna; no toca el ERP view |
| `tullpu` (editor de capas) | `02_ruway/tullpu/tullpu-core` + `tullpu-app-llimphi` + `tullpu-render` | App nueva: lienzo, capas (cada una objeto del grafo BLAKE3 → dedup automático), brush, máscaras, ajustes no destructivos como nodos del DAG | crate nuevo |

**Estimaciones gruesas**: foreign-docx 2-3 sem · foreign-xlsx sin fórmulas 1-2 sem · yupay 6-10 sem · vista spreadsheet 3-4 sem · foreign-pptx 1-2 sem · tullpu base 3-4 meses · foreign-psd 2 sem post-tullpu.

## 6.quater Hito — Pluma: lienzos paralelos (texto multivista)

**Visión** (2026-05-26): un documento pluma es una secuencia de párrafos sobre un *lienzo*; a su lado existen otros lienzos (idioma, tono, audiencia, resumen, versión, comentario crítico) alineados párrafo-a-párrafo. UI: scroll horizontal entre lienzos, barras de color verticales que conectan posiciones correspondientes. Generación automática de lienzos por transformaciones inteligentes (vía rimay/iniy, todo local).

**Base ya existente** en `pluma-core` (138 LOC) y `pluma-graph` (211 LOC): `NarrativeAtom` con `branch_id` + `semantic_vectors` + `coherence: PendingEvaluation` propagado por DAG. La idea de "lienzos" es darle a `branch_id` semántica de variante (idioma/tono/derivado), no solo de rama temporal.

| Pieza | Crate | Propósito |
|---|---|---|
| `pluma-cuerpo` | `pluma-cuerpo` | Modelo de *cuerpo* (lienzo): conjunto ordenado de `NarrativeAtom`s con un `branch_id`, metadatos (idioma, autor, intención: traducción/resumen/tono…) |
| `pluma-align` | `pluma-align` | Alineamientos `(atom_a, atom_b, fuerza, origen)`. Origen ∈ {Manual, Embeddings(rimay/iniy), Derivado(transformación)}. Persistencia incremental |
| `pluma-transform` | `pluma-transform` | Transformaciones declarativas que derivan un cuerpo de otro: `Traducir(qu)`, `Tono(formal)`, `Resumir(palabras)`, `Reescribir(prompt)`. Pueden ser idempotentes/regenerables |
| Vista multilienzo en `pluma-editor-llimphi` | `pluma-editor-llimphi` | Scroll horizontal, *hebras* (barras de color) entre párrafos correspondientes; focus mode 1-2 lienzos |

Ver §11 abajo para la propuesta detallada.

## 7. Repos legacy

`~/legacy/{brahman, eternal, dominium}` — arqueología local. Espejos remotos en gitea siguen como respaldo (no se borran).

## 8. Próxima sesión arranca con

**Migración GPUI → Llimphi**. Llimphi ya tiene: render gráfico (vello), layout flex/grid (taffy), texto con shaping (parley), input mouse+teclado, bucle Elm. Suficiente para portar la primera app.

Candidatos (orden de menor a mayor riesgo):
1. ~~**`mirada-launcher`**~~ — descartado como primera migración: hoy es TUI, no GPUI.
2. **`mirada-greeter`** — ✅ portado (2026-05-25). Extendido `llimphi-ui` con `Handle<Msg>` (quit + spawn de hilos que reentran al `update`) y `app_id()` para Wayland. La lógica de `auth-core` quedó intacta. Refactorizado más tarde para consumir `llimphi-widget-text-input` (extraído del input inline) — análogo Llimphi al `nahual-widget-text-input` GPUI.
3. **`pluma-editor-gpui`** → `pluma-editor-llimphi` — ✅ portado (2026-05-25). Visualizador DAG: bloques absolutamente posicionados (taffy `Position::Absolute`), conectores S-codo como triplas de rectángulos delgados, osciloscopio de coherencia. Llimphi-ui ganó `App::initial_size()` para overridear el default 960×540.
4. **`nahual-shell-llimphi`** — MVP (2026-05-25): file explorer + viewer dual (texto o imagen según extensión PNG/JPG/JPEG) en split **draggable**. Cada pieza extraída a su propio crate Llimphi reusable: `nahual-file-explorer-llimphi` (`FileExplorerState` + `file_explorer_view`), `nahual-text-viewer-llimphi` (`PreviewState` + `load_preview` + `text_viewer_view`), `nahual-image-viewer-llimphi` (`ImagePreviewState` + `load_image` + `image_viewer_view`, decodifica PNG/JPEG con crate `image`). El shell mismo queda fino: header + splitter + switch de viewer por extensión. Navegación con teclado (↑↓ Enter ⌫), rueda del mouse, click; preview de archivos texto ≤256KB. Llimphi-ui ganó: `clip` (push_layer/pop_layer con `Mix::Clip`, recorta paint **y** hit-test), `on_wheel` (delta normalizado a líneas), `hover_fill` (paint distinto cuando el cursor toca el nodo), `draggable(handler)` con `DragPhase::{Move, End}` (handler recibe el delta del eje principal desde el evento anterior, sobrevive a invalidaciones de cache vía `Arc<dyn Fn>`). Widgets reusables ya extraídos en `02_ruway/llimphi/widgets/`: `list`, `text-input`, `button` (con hover), `splitter` (con drag), `tabs`, `tree` (expand/collapse + selección), `app-header` (label + acciones), `card` (container con accent opcional), `stat-card` (label + value + description sobre card), `banner` (Info/Success/Warning/Error), `tiled` (grid auto cols×rows con title bar fija, **drag-to-swap activo** vía `tiled_view_reorderable`). Cada uno con `examples/{widget}_demo.rs` ejecutable. Además: `gallery` (bin) pinta todos en una ventana — referencia visual + smoke test. Paleta compartida `llimphi-theme` con slots semánticos (bg_app, fg_text, accent, etc.); todas las paletas de widget consumen `Palette::from_theme(&theme)`. Llimphi-ui ganó drop-targets globales: `View::drag_payload(u64)` declara payload del drag y `View::on_drop(Fn(u64) -> Option<Msg>)` + `View::drop_hover_fill(color)` los reciben en el destino (runtime hace hit-test sobre drop targets durante drag, invoca el handler al soltar y pinta el target hovereado con override). Llimphi-ui también gana imágenes: `View::image(peniko::Image)` pinta una imagen Rgba8 dentro del rect del nodo en aspect-fit centrado vía `vello::Scene::draw_image`. Sobre eso, `nahual-image-viewer-llimphi` (PNG/JPEG via crate `image`) es el primer consumidor — análogo al `nahual-text-viewer-llimphi`. Y `View::paint_with(Fn(&mut Scene, &mut Typesetter, PaintRect))` para canvas elements custom: la closure recibe scene + typesetter cacheado + rect absoluto del nodo. Consumidores: `dominium-canvas-llimphi` (quads del `RenderPlan`) y `cosmos-canvas-llimphi` (DrawCommand de `cosmos-render` → Circle/Line/Polygon vello + texto vía llimphi-text). `Handle::spawn_periodic(period, Fn() -> Msg)` extrae el patrón thread+loop+sleep+dispatch para ticks de simulación. Sin layout.json/persister/hot-reload/DatabaseExplorer/AppBus todavía.

En paralelo (no bloqueado): **Fase 1 de Puriy** (`puriy-core` puro Rust — Tab/Session/History/Bookmark/Profile testeables).

---

## 11. Propuesta detallada — Pluma: lienzos paralelos

### 11.1 Concepto

Un documento ya no es *una* secuencia lineal de párrafos: es **un haz de cuerpos** que recorren el mismo material desde distintas miradas. Cada cuerpo (lienzo) es una secuencia ordenada de `NarrativeAtom`s. Distintos cuerpos del mismo documento se enlazan por *alineamientos* párrafo-a-párrafo. La UI los presenta como columnas en scroll horizontal con *hebras* (barras de color verticales) que conectan posiciones correspondientes.

### 11.2 Casos de uso primarios

1. **Traducción paralela** es ↔ en ↔ qu (gioser ya tiene rimay-localize y embeddings rimay/iniy locales).
2. **Versiones / borradores** alineados — diff de revisiones párrafo a párrafo, no línea a línea.
3. **Tono / audiencia** — formal, casual, técnico, infantil sobre el mismo contenido.
4. **Resumen ↔ expansión** — abstract alineado con artículo completo.
5. **Anotación crítica** — texto original alineado con comentario (modelo Talmud / glosa medieval).
6. **Multi-modal** — texto alineado con transcripción de audio, descripción de imagen, código.

### 11.3 Modelo de datos

- **`NarrativeAtom`** (ya existe) = párrafo. Conserva id, hash, contenido, vectores semánticos, dependencias, `branch_id`, coherence.
- **`Cuerpo`** (nuevo) = `{ id: Uuid, branch_id: String, orden: Vec<Uuid>, metadatos: MetaCuerpo }`. `MetaCuerpo` incluye `lengua: Option<Lengua>`, `intencion: Intencion`, `derivado_de: Option<Uuid_cuerpo>`, `fresco_hasta: Option<u64>` (timestamp del último hash de cuerpo madre que regeneró este).
- **`Alineamiento`** (nuevo) = `{ atom_a: Uuid, atom_b: Uuid, fuerza: f32 ∈ [0,1], origen: OrigenAlineamiento, fresco: bool }`. Un atom puede alinearse a N atoms (1↔1, 1↔N, N↔1, 0↔1).
- **`OrigenAlineamiento`** = `Manual { autor, ts } | Embeddings { algoritmo, modelo, ts } | DerivadoDe { transformacion: Uuid_transform }`.
- **`Transformacion`** (nuevo) = `{ id, kind, params, madre: Uuid_cuerpo, hija: Uuid_cuerpo }`. `kind ∈ { Traducir(Lengua), Tono(Tono), Resumir{palabras}, Reescribir{prompt}, Identidad, Custom(Rhai) }`. Si la madre cambia, la hija queda *stale*; un comando regenera puntualmente por párrafo.

### 11.4 Innovaciones que añade gioser sobre la idea base

- **Alineación dinámica por embeddings** (rimay/iniy): al crear un cuerpo, no asume 1:1. Mapea por similitud semántica; un párrafo del original puede mapear a 2 párrafos de la traducción, o a ninguno. La **saturación** de la hebra refleja la fuerza de la correspondencia.
- **Hebras con estado**: color sólido = fresca, color desaturado con patrón punteado = stale (la madre cambió desde la última regeneración), gris = manual sin embeddings que la respalden.
- **Lienzos derivados vs divergentes**: hebra continua = derivado regenerable, hebra discontinua = versión humana independiente. El usuario sabe de un vistazo qué le costará "actualizar".
- **Grafo de lienzos, no lista**: cuerpos forman un DAG (`qu` deriva de `es`, `qu-formal` deriva de `qu`). El scroll horizontal recorre un orden topológico, configurable.
- **Identidad estable de párrafo**: cada `NarrativeAtom` mantiene su `id: Uuid` aunque se mueva o se reescriba; los alineamientos no se rompen al insertar/borrar párrafos.
- **Búsqueda transversal**: una búsqueda atraviesa todos los cuerpos visibles; resultados aparecen como puntos brillantes en sus respectivas columnas y se enlazan con hebras temporales.
- **Vista matriz** (alternativa al scroll horizontal): párrafos en filas, cuerpos en columnas — útil para textos cortos o revisión densa.
- **Focus mode 2 cuerpos**: oculta todos menos N, sigue alineados, lectura comparativa.
- **Inline lienzos pequeños**: en lugar de scroll, expansión inline en el lienzo principal (preview transitorio del lienzo hija).
- **Historial de transformaciones por hebra**: click en una hebra muestra la cadena `(es → resumir → en → tono(infantil))` que generó ese párrafo.
- **Lienzos federados (minga)**: un cuerpo puede vivir en otro nodo. Tu `es` alineado con `qu` de un compañero. Cada cuerpo es objeto del grafo, content-addressed, ya federable.
- **Exportación lossy explícita**: a docx eliges UN cuerpo o un par "lado a lado"; al formato nativo pluma conservas todo el haz.

### 11.5 UI — el scroll horizontal

```
┌────────────┬──────────┬────────────┬──────────┬────────────┐
│ es (madre) │ hebras   │ en (deriv) │ hebras   │ qu (deriv) │
│ ▓▓▓▓▓▓▓▓▓ │ ━━━━━━━━ │ ▓▓▓▓▓▓▓▓▓ │ ╴╴╴╴╴╴╴╴ │ ▓▓▓▓▓▓▓▓▓ │   ← párrafo 1: hebra fresca a en, stale a qu
│            │          │            │          │            │
│ ▓▓▓▓▓▓▓▓▓ │ ━━━━━━━━ │ ▓▓▓▓▓▓▓▓▓ │ ━━━━━━━━ │ ▓▓▓▓▓▓▓▓▓ │   ← párrafo 2: todo fresco
│            │   ╲      │            │          │            │
│ ▓▓▓▓▓▓▓▓▓ │    ╲     │ ▓▓▓▓▓▓▓▓▓ │ ━━━━━━━━ │ ▓▓▓▓▓▓▓▓▓ │   ← párrafo 3: 1→2 en en (hebra divergente)
│            │     ╲    │ ▓▓▓▓▓▓▓▓▓ │ ━━━━━━━━ │            │
└────────────┴──────────┴────────────┴──────────┴────────────┘
   ←──── scroll horizontal ────→
```

Color de hebra codifica fuerza de correspondencia (0–1) en saturación; tipo (continua/discontinua/punteada) codifica origen (derivado/divergente/stale).

### 11.6 Crates y fases

1. **`pluma-cuerpo`** (nuevo) — `Cuerpo`, `MetaCuerpo`, persistencia. ~200 LOC. Independiente de UI.
2. **`pluma-align`** (nuevo) — `Alineamiento`, alineadores: `alinear_uno_a_uno`, `alinear_por_embeddings(modelo_iniy)`. ~300 LOC.
3. **`pluma-transform`** (nuevo) — `Transformacion`, ejecutor con backend pluggable (rimay-localize para traducir, iniy para tono/resumen, Rhai para custom). ~400 LOC + adapters.
4. **`pluma-editor-llimphi`** — extender con view multilienzo, hebras (paint_with custom o widget nuevo `pluma-hebras-llimphi`), scroll horizontal sincronizado, focus mode. ~600 LOC nuevas sobre las 318 actuales.
5. **`pluma-core`** — añadir `id` estable + utilidad `paragraf_key(atom)` para alineamientos robustos a edición. Cambio mínimo.

### 11.7 Orden propuesto

1. `pluma-cuerpo` + tests de roundtrip.
2. `pluma-align` con alineador manual y `alinear_uno_a_uno`.
3. Vista multilienzo en `pluma-editor-llimphi`: 2 columnas, hebras simples (sin saturación todavía), scroll horizontal sincronizado.
4. `pluma-transform` con `Identidad` (copia 1:1 de un cuerpo a otro, hebras a tope) — prueba el flujo madre/hija sin LLM.
5. Conectar `pluma-transform::Traducir` a rimay-localize → primer cuerpo derivado real (es → qu).
6. Conectar `pluma-align::alinear_por_embeddings` a iniy → hebras con saturación + stale detection.
7. Resto de transformaciones (`Tono`, `Resumir`, `Reescribir`) y UI completa (búsqueda transversal, vista matriz, focus mode).
