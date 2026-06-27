# AGENTE.md — la IA conversacional multi-agente de shuma

Documenta el subsistema de **chat multi-agente** de shuma: el panel estilo apps
web de IA (sidebar de conversaciones + selector de agente + hilo con bloques
ricos), construido sobre `pluma-llm`. Es la evolución de la IA *atómica* de shuma
(`:?` / `:hacé` / `:explica`, una sola vuelta sin memoria) a **agentes
configurables + conversaciones multi-turno persistidas + salida en streaming**.

> Fuente autoritativa cuando difiera con comentarios sueltos del código. Verificá
> nombres con `grep` antes de asumir: este doc envejece.

## Mapa de crates

Cuatro capas, agnósticas de UI hacia abajo (Regla 2 del repo):

| Crate | Rol |
|---|---|
| `sandbox/shuma-agente` | **Núcleo** sync/puro: `Agente`, `Conversacion`/`Turno`/`BloqueSalida`, `motor` (arma el `ChatRequest`, interpreta la salida en bloques), `Almacen` sled. Sin red. |
| `sandbox/shuma-agente-host` | **Host**: `responder` / `responder_streaming` — corre `pluma-llm` (resuelve backend, bloqueante en un thread) y devuelve bloques + tokens. |
| `sandbox/shuma-module-agente` | **UI** (módulo shuma): `State`/`Msg`/`update`/`view`. Panel de chat + editor de agentes. |
| `shuma-shell-llimphi` | **Chasis**: monta el panel como diente `Tool::Agente`, abre el `Almacen`, corre el host en threads, rutea teclado y persiste. |
| `00_unanchay/pluma/pluma-llm-claude-cli` | **Backend** que usa el binario `claude` (suscripción) — ver §Auth. |

## Modelo de datos

- **`Agente`** — identidad + `backend` propio (`wawa_config::LlmSettings`: proveedor,
  modelo, API key, endpoint) + `system_prompt` (persona) + `Capacidades` (si puede
  proponer acciones de control atipay, y de qué superficies) + temperatura/max_tokens.
  `backend` vacío = hereda el `[ai.llm]` global del SO.
- **`Conversacion`** — hilo multi-turno: `Vec<Turno>`, título auto-derivado del primer
  mensaje, timestamps. Ordenadas por `actualizada` (recientes primero).
- **`Turno`** — `rol` (Usuario/Asistente), `bloques`, `uso: Option<Uso>` (tokens).
- **`BloqueSalida`** — la *gama de outputs*: `Texto` · `Codigo{lenguaje,codigo}` ·
  `Accion(AccionPropuesta)` (acción de control validada por atipay) · `Error`.

El texto crudo del modelo se interpreta a bloques en `motor::interpretar_respuesta`:
cercos ```` ```accion ```` → acción atipay (validada, **nunca auto-ejecutada**), otros
cercos → código, el resto → texto.

## Persistencia

`Almacen` (sled) en `<perfil>/agente.sled` (`persist::agente_db_path`). Dos árboles:
`agentes` y `conversaciones`, JSON por clave=id. `sembrar_defaults` crea «Asistente»
y «Control» la primera vez (idempotente). El chasis persiste tras cada cambio.

## Patrón intent (trabajo async sin colgar el bucle Elm)

El módulo **no toca la red**. Deja intents que el chasis cumple en threads:

- `take_request()` → el chasis corre `shuma_agente_host::responder_streaming` y
  devuelve `Msg::Token` (por fragmento) + `Msg::Respuesta` (final).
- `take_ejecucion()` → acción aprobada; el chasis la pone en el input del shell
  activo (`InsertAtCursor` — revisar y Enter, nunca auto-corre).
- `take_persist_agente()` / `take_borrar_agente()` → alta/edición/borrado de agente
  → el chasis escribe al `Almacen` y re-provee con `set_agentes`.

El reloj y el alto del viewport los inyecta el chasis (`fijar_reloj`, `fijar_vista_alto`):
el `update` es puro y no lee el reloj.

## Streaming

`ChatClient::stream(req, on_delta)` (en `pluma-llm-core`) tiene un **default no
incremental** (corre `complete` y emite todo al final) — así los backends sin
streaming no cambian. `pluma-llm-claude-cli` lo sobreescribe: corre el CLI con
`--output-format stream-json --verbose --include-partial-messages`, lee el NDJSON y
emite cada `content_block_delta.text`. El chasis despacha `Msg::Token` por delta vía
`Handle::dispatch`; el módulo acumula en `parcial` y pinta una burbuja viva con
cursor `▌`, reemplazada por los bloques al llegar `Respuesta`.

## Autenticación — usar la suscripción sin API key

Tres caminos, de menos a más acoplado a Anthropic:

1. **API key por agente** (`anthropic`/`gemini`/`deepseek`/`cohere`/`ollama`): cada
   agente lleva su clave. NO requiere ser app oficial. Pago por token.
2. **Backend `claude-cli`** (default de los agentes sembrados): maneja el binario
   `claude` (Claude Code) como subproceso. **Claude Code hace el OAuth** (incluida la
   suscripción Pro/Max); la app no toca ni reusa el token. Es el camino **legítimo**
   para usar una suscripción desde software propio, sin pagar por token aparte.
   Requiere `claude` instalado y `claude login`. Override del binario por
   `$CLAUDE_CLI_BIN` o el campo `endpoint` del agente.
3. **OAuth crudo de suscripción** (`sk-ant-oat01-…`): **PROHIBIDO** reusarlo en apps
   de terceros (viola los ToS de Anthropic, enforcement desde feb-2026). No se usa.

## Cómo se usa

Abrir shuma → diente «Agente» (globo de diálogo) en el rail derecho. Elegir agente,
escribir, Enter envía. «+ agente» / «editar» abren el formulario (nombre, modelo,
persona, backend ciclable, toggle control; Tab cicla campos, Escape cancela). Las
acciones de control salen como tarjetas con aprobar/rechazar.

## Pendientes / futuro

- El panel entra en el slot angosto del rail (sidebar 150px, redimensionable) — quizá
  convenga un slot más ancho o una vista compacta sin sidebar.
- Ejecutar acciones aprobadas directo (hoy van al input del shell, por la doctrina
  «nunca auto-ejecutar»).
- Visión (imágenes): `pluma-llm-core` ya soporta `ChatImage`; falta cablearlo en la UI.
