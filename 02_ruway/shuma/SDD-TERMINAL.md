# SDD — superficie de terminal infinita y supereficiente

> Estado: **diseño, sin implementar** · 2026-06-05 · autor de la tarea futura: quien forje esto.
> Idioma del repo: español. Reemplaza, por fases, el `output_pane` actual del shell Llimphi.

## Tesis

El shell de gioser existe para **desplanar la terminal**: el output no es un volcado
plano, es contenido vivo que se despliega en la ventana a medida que se genera. Hoy
eso se logra con cards IDE (numeración, color, selección, badges) — pero el control
**no escala**: capa a ~500 líneas y pinta *todo lo que hay*, no *lo que se ve*.

La apuesta: una **superficie de terminal** virtualizada que (a) sostiene scrollback
**ilimitado** a costo de render **constante**, (b) sirve tres modos sobre la misma
tela — **línea** (IDE: numerada, selectable), **grilla** (alt-screen TUI), **híbrido**
(PTY en modo líneas) — y (c) usa **GPU directo** exactamente donde paga (la grilla y
los floods), vello donde alcanza (chrome + línea virtualizada). Sin render plano,
jamás.

## Principios irrenunciables (el norte, no se negocia)

Lo que define este control y lo separa de una terminal cualquiera (pedido explícito
del usuario, 2026-06-05):

1. **Nunca plano.** El output JAMÁS es un volcado de texto crudo. Siempre es contenido
   estructurado y vivo (bloques, numeración, color, chrome). Si algo cae a render
   plano, es un bug, no un fallback aceptable.
2. **Interactivo y dinámico.** Se despliega a medida que se genera (streaming), se
   colapsa/expande, se scrollea fluido, responde al mouse y al teclado. No es estático.
3. **Menú contextual + clipboard de primera clase.** Selección moderna (arrastre,
   doble/triple-click), copiar/pegar **nuestro** (no el del terminal crudo), menú de
   botón-derecho con acciones — en TODOS los modos (línea, grilla, híbrido), no sólo en
   líneas.
4. **Emula TUIs.** Las apps de pantalla completa (vim/htop/less/…) corren de verdad,
   con su grilla de celdas, dentro de la misma superficie — no como un terminal opaco
   "por un vidrio", sino integradas y (donde aplique) con nuestra selección/copia.

Todo lo de abajo está al servicio de estos cuatro puntos.

## La limitación actual (el porqué de este SDD)

- `MAX_OUTPUT_LINES = 500` (`shuma-module-shell/src/lib.rs`): el buffer se capa. Si no,
  el render explota.
- `output_pane` (`view.rs`) arma **un text-editor por comando**, cada uno pintando
  **todas** sus líneas (subimos el cap embebido a `EMBEDDED_LINE_CAP = 512`), y el
  panel **traslada** todo con un `transform` de scroll.
- Resultado: ~500 Views pintadas por frame es el techo (pared de `wgpu`
  `max_*_buffer_binding_size` + costo de layout). Y el modelo "editor por comando +
  panel que traslada" fue la fuente de bugs reales (negro al anclar al fondo,
  desalineación gutter/contenido por el transform multicolor del compositor —
  arreglado en commit `caf37079`).

**Conclusión:** el techo no es un número a subir, es la arquitectura. Para infinito hay
que **virtualizar** (pintar sólo la ventana visible) y dejar el scroll a UN control,
no a editores anidados que el panel traslada.

## Arquitectura — capas estrictas

```
┌─ Capa 4 · Interacción ────────────────────────────────────────────┐
│  selección sobre el stream · numeración · find · menú · copy/paste │
├─ Capa 3 · Render ─────────────────────────────────────────────────┤
│  GPU-directo (atlas glifos + celdas instanciadas) → grilla/flood   │
│  vello → chrome (cards/badges/colapsables) + modo línea virtualiz. │
├─ Capa 2 · Virtualización ─────────────────────────────────────────┤
│  ventana visible (fila inicial..fila final) sobre el viewport;     │
│  sólo esas filas/bloques se materializan en Views/draws            │
├─ Capa 1 · Modelo de bloques ──────────────────────────────────────┤
│  stream de bloques (comando = header+cuerpo+badge+stages+colapso)  │
│  cada bloque indexa su rango de filas en el store                  │
├─ Capa 0 · Store de scrollback ────────────────────────────────────┤
│  append-only, compacto: bytes + índice de offsets de línea;        │
│  cap por MEMORIA (MB), no por líneas; spill a disco opcional       │
└────────────────────────────────────────────────────────────────────┘
```

Regla dura del repo: **núcleo agnóstico, frontend lo pinta** (Regla 2). Por eso:

- **`llimphi-widget-terminal`** (crate nuevo, reusable — logs, consolas, no sólo shuma):
  Capas 0–4 agnósticas de shuma. No sabe de comandos; sabe de *bloques de filas* con
  un `BlockKind` (líneas numeradas / grilla / chrome opaco que el caller pinta).
- **shuma** maneja el modelo de comando (header/badge/stages/reprocess) como
  *decoración de bloque* que inyecta al widget; el widget virtualiza y pinta.

## Capa 0 — Store de scrollback

- **Append-only.** Cada línea (o chunk de bytes del PTY) se appendea. Nunca se
  reescribe lo viejo.
- **Compacto.** Texto en un `Vec<u8>`/rope; un índice `Vec<u32>` (o `Vec<u64>` si supera
  4 GB) de offsets de inicio de línea. Acceso a la línea N = O(1).
- **Cap por MEMORIA, no por líneas.** `scrollback_limit_mb` (default generoso, p. ej.
  64 MB ≈ cientos de miles de líneas). Al excederlo, se descarta el principio
  (drop-front del rope + reindex). El usuario pidió "infinito"; en la práctica es
  "limitado por una memoria que elegís", con **spill a disco** opcional (ya hay
  precedente: `:limit`/`:spill` de captura por MB en el shell).
- **Estable bajo append durante scroll** (deuda B del PLAN-OUTPUT): si el usuario
  scrolleó arriba y llega output, la posición de lectura se preserva (anclar a un
  *line id*, no a px desde el fondo).

## Capa 1 — Modelo de bloques

- El stream es una secuencia de **bloques**. Para shuma: un bloque = un comando
  (header `$ …` + cuerpo + badge de estado + filas de etapa + estado colapsado).
- Cada bloque conoce su **rango de filas** `[fila_inicio, fila_fin)` en el store y su
  `BlockKind`:
  - `Lines` — filas de texto numeradas/coloreadas (modo línea, lo común).
  - `Grid { rows, cols }` — una grilla de celdas (alt-screen TUI), su contenido vive
    en el emulador vt100, no en el store de líneas.
  - `Chrome` — un nodo opaco que el caller pinta (header de card, fila de etapas) y
    que ocupa un alto fijo conocido.
- **Colapso = el bloque reporta alto 0 para su cuerpo** (sólo su header). La
  virtualización lo respeta gratis.

## Capa 2 — Virtualización (el corazón)

Dado `scroll_y` y `viewport_h`, el widget calcula la **ventana visible** de filas
globales `[v0, v1)` y materializa **sólo** esas:

1. Mapa fila-global → (bloque, fila-local) por búsqueda binaria sobre los rangos de
   bloque (los bloques son monótonos en filas).
2. Sólo los bloques que intersectan `[v0, v1)` emiten Views/draws. Un `ls -alR` de 1 M
   de líneas: si 40 filas caben en pantalla, se materializan ~40 + el chrome de los
   bloques visibles. **Costo de render constante**, independiente del scrollback.
3. El scroll es **del widget** (un `scroll_y` interno, no un `transform` del panel
   sobre editores altos). Esto evita de raíz el bug clip+transform que ya nos costó.

Anclaje al fondo (estilo terminal) = `scroll_y` clamp al máximo salvo que el usuario
scrollee arriba; append mantiene el fondo pegado.

## Capa 3 — Render (dónde entra GPU-directo, con precisión)

Regla del repo (validada, ver [[project_gpu_directo_bench_pending]]): *datos fijos →
buffer persistente GPU; datos dinámicos → vello*.

- **Modo línea (lo común): vello alcanza.** 40 filas × layout de texto por frame es
  trivial. Numeración, color por runs, selección como rects. **No** necesita GPU
  directo. Reusa la maquinaria del `text-editor` (selección/clipboard/find) extraída a
  un núcleo compartido, NO duplicada (Regla 2 + un-término-un-artefacto).
- **Modo grilla (TUI) + floods: GPU directo paga.** Una grilla de celdas (htop, vim,
  un juego-TUI) redibuja toda la pantalla a alta frecuencia. Patrón: **atlas de glifos
  persistente** (cada glifo rasterizado una vez a una textura) + **quads de celda
  instanciados** (un draw instanced de `rows*cols` celdas, cada una = índice de glifo +
  fg/bg). Es el patrón `GpuPipelines.*` ya validado (141 fps @ 1M instancias en Iris
  Xe). Throughput de terminal real, sin generar miles de Views.
- **Chrome (cards/badges/colapsables): vello.** Bordes, gradientes de recencia,
  iconos vectoriales — exactamente como hoy.
- **Híbrido (PTY en modo líneas, p. ej. `claude`/`watch`):** modo línea sobre el
  screen vt100, virtualizado igual.

La superficie compone: una pasada GPU-directo para las celdas de grilla visibles + una
pasada vello para texto-línea visible y chrome. Una sola escena.

## Modos sobre la misma tela

| Modo | Disparador | Render | Selección |
|---|---|---|---|
| **Línea** | output normal | vello text virtualizado + numeración | rangos de líneas globales |
| **Grilla** | `ESC[?1049h` (alt-screen, señal dura ya detectada) | GPU-directo celdas instanciadas | rectangular por celdas |
| **Híbrido** | PTY sin alt-screen | modo línea sobre el screen vt100 | como línea |

La detección de modo ya existe en el shell (`is_tui_fullscreen` / alt-screen del parser
vt100); se mueve a la superficie como `BlockKind`.

## Capa 4 — Interacción

- **Selección sobre el stream completo** (no por-card): un ancla y una cabeza en
  coords de *fila global, columna*. Copia une las líneas del rango desde el store.
- **Numeración** continua o por-bloque (configurable; hoy es por-bloque).
- **Find** (Ctrl+F) sobre el store (búsqueda en bytes, salta scroll a los hits) — deuda
  D del PLAN-OUTPUT, acá nace natural.
- **Menú contextual** (ya hecho, commit `09cd0429`) se reusa.
- **Gancho IA** sobre una selección (depende de [[project_shuma_ctls_ia_busqueda]]).

## Fases de forja (incremental, cada una verificable headless)

> **Gotcha de verificación obligatorio** (lección 2026-06-05, costó confianza): todo
> dump de prueba con output alto DEBE simular el **viewport medido y el scroll al
> fondo** (`out_viewport_h` real), o el bug se esconde y se commitea algo roto.

- **Fase 0 — Store + índice. ✅ (2026-06-05)** Crate `llimphi-widget-terminal`
  (`02_ruway/llimphi/widgets/terminal`), módulo `store`: `Scrollback` append-only,
  índice de offsets de línea (sentinela), acceso O(1), cap por memoria con recorte
  de frente en un `drain`+reindex, ids globales estables (`line_id`/`index_of_id`)
  que sobreviven al recorte, numeración 1-based, `slice_text` para copiar, `clear`.
  Puro, sin deps de UI. 11 tests (incl. 100k líneas acotadas e indexadas).
- **Fase 1 — Virtualización modo línea. ✅ (2026-06-05)** Capas 1–2 en
  `llimphi-widget-terminal::view`: `line_surface` materializa **sólo** la ventana
  visible (`visible_window`, pura y testeada) bajo un `scroll_y` **propio del
  widget** (no transform de contenido alto — la anti-feature del SDD), con
  numeración global 1-based del store, color base + runs + tinte de fondo por
  renglón (inyectados por el caller vía `LineStyle`, Regla 2), scrollbar via
  `thumb_geometry` dimensionada al alto TOTAL virtual, scroll sub-renglón
  (`partial_px`) y painter de medición del viewport. 19 tests (store + ventana).
  **Verificado headless** (`examples/dump_terminal.rs`): 1 M de líneas, anclado al
  fondo → **38 filas materializadas** (999963..1000000), sin negro, alineado,
  costo constante (independiente del scrollback). Falta: enganchar al shell
  (Fase 2 trae bloques/chrome y el flag `SHUMA_TERMINAL_SURFACE`).
- **Fase 2 — Bloques + chrome. ✅ (2026-06-05)** Capa 1 en
  `llimphi-widget-terminal::blocks`: el stream es una secuencia de `Item`s —
  `Chrome{height, view}` (header/badge/etapa de alto fijo que el caller pinta) o
  `Lines{start, end}` (rango del store en modo línea). `block_surface` virtualiza
  sobre **alturas mixtas**: `item_tops` + `visible_items` (búsqueda binaria,
  O(log n) en bloques) localizan los items que tocan el viewport, y dentro de un
  `Lines` enorme `visible_rows_in_item` materializa sólo las sub-filas visibles —
  costo constante aunque un body tenga 500 k líneas. **Colapsar** = no emitir el
  `Lines`. El modo línea de la Fase 1 quedó **unificado** como el caso de un solo
  `Item::Lines(0, len)` (delega en `block_surface`, sin duplicar render). 26 tests.
  **Verificado headless** (`examples/dump_blocks.rs`): 6 comandos, un flood de
  500 k líneas, un bloque colapsado, stderr tintado, anclado al fondo → ~40 filas
  materializadas. Falta: enganchar al `output_pane` del shell detrás del flag
  `SHUMA_TERMINAL_SURFACE` (mapear `OutputLine`/`block_command`/`expanded_stages`
  → `Item`s; A/B con el viejo) — la integración, no más capacidades del widget.
- **Fase 3 — Selección + find sobre el stream.** Extraer el núcleo de selección del
  `text-editor` a compartido; selección global; copy; Ctrl+F.
- **Fase 4 — GPU directo grilla.** Atlas de glifos + celdas instanciadas para el modo
  grilla (TUI). Bench vs el grid vt100 actual. Híbrido.
- **Fase 5 — Pulido + migración.** Anclaje estable bajo append, scroll inertial,
  borrar el `output_pane`/per-command-editor viejo. Spill a disco.

Cada fase es un commit (o pocos) verificado con render headless + viewport medido, y
deja el shell funcionando (flag de migración hasta la Fase 5).

## Cómo reemplaza al `output_pane` (sin romper)

- Fases 1–4 conviven con el `output_pane` actual detrás de un flag
  (`SHUMA_TERMINAL_SURFACE=1`), para A/B y rollback inmediato.
- El modelo de datos no cambia de raíz: las `OutputLine` + `block_command` +
  `expanded_stages` se mapean a bloques de la superficie. El emulador vt100 y la
  detección de alt-screen se reusan.
- La Fase 5 borra el camino viejo sólo cuando el nuevo tenga **paridad verificada**
  (no antes — Regla: no afirmar paridad sin evidencia).

## Anti-features (rechazadas con motivo)

- **Subir `MAX_OUTPUT_LINES` y ya.** Mueve la pared, no la rompe; sigue siendo
  "render todo".
- **Un text-editor gigante para todo el scrollback.** El widget de archivo virtualiza
  pero no modela bloques (header/badge/grilla); forzarlo es lo que ya rompió.
- **GPU directo para TODO.** El modo línea no lo necesita; meterlo ahí es complejidad
  sin payoff y pelea con vello (texto rico).
- **Scroll por `transform` del panel sobre contenido alto.** Es la fuente del bug
  clip+transform. El scroll vive en la superficie, que sólo materializa lo visible.

## Pila exacta (sin negociación)

- Crate `llimphi-widget-terminal` (Capas 0–4 agnósticas), consumido por
  `shuma-module-shell`.
- Texto: `llimphi-text` (vello) para modo línea + chrome.
- Grilla: `llimphi-raster` GPU directo (`GpuPipelines`, patrón persistente) +
  `fontdue`/atlas para el glyph cache (precedente: `atlas` de wawa, Fontdue).
- vt100: el parser ya en uso (`vt100` crate) para grilla/híbrido.
- Núcleo de selección/find: extraído de `text-editor` a compartido, NO duplicado.

## Referencias

- Código actual: `shuma-module-shell/src/view.rs` (`output_pane`, `command_card`,
  `body_editor_*`), `lib.rs` (`MAX_OUTPUT_LINES`, `block_command`).
- Bugs que motivaron esto: negro al anclar al fondo + desalineación gutter/contenido →
  fix de raíz del transform multicolor (commit `caf37079`); cap embebido
  (`2038492c`); ruteo a card IDE (`01befe89`).
- GPU directo validado: [[project_gpu_directo_bench_pending]] (141 fps @ 1M, Iris Xe).
- Plan de UX del output: `PLAN-OUTPUT.md` (deudas D/find, anclaje estable).
- Memoria viva: [[project_shuma_output_ux]], [[project_shuma_rescate]].

## Estado

Diseño. Sin implementar. Pendiente de arranque por fases (Fase 0 primero). Decisión de
construir tomada con el usuario 2026-06-05; el control nuevo se justifica por el techo
arquitectónico de ~500 líneas y por la eficiencia GPU-directo en grilla/TUI.
