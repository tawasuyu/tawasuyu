# SDD — superficie de terminal infinita y supereficiente

> Estado: **implementado — Fases 0-5 ✅** (ver §Estado al final) · diseño 2026-06-05, forjado al 2026-06-07.
> Idioma del repo: español. Reemplaza, por fases, el `output_pane` actual del shell Llimphi.

## Tesis

El shell de tawasuyu existe para **desplanar la terminal**: el output no es un volcado
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

## Principio rector: **un control, los paneles son datos** (no al revés)

La inversión que hace funcionar todo lo de abajo. En el diseño viejo cada panel
(card de comando) **era un control** con su propio scroll/estado (`text_editor` por
comando) y un contenedor los **trasladaba a todos** con un `transform`. Acá es al
revés: hay **un solo control** (la superficie) y los paneles son **items de datos**
(`Item::Chrome` / `Item::Lines`) que el control coloca y virtualiza. Las ventajas
—por las que se eligió esta forma, no por estética—:

1. **Costo de render desacoplado del contenido.** Un único scroller virtualiza: el
   costo es ∝ la ventana visible, **no** ∝ la cantidad de paneles ni de líneas. El
   modelo "un control por panel" pagaba por *cada* panel siempre — la pared de ~500.
2. **Un scroll, un sistema de coordenadas.** Sin transforms anidados → mata de raíz
   la clase de bug clip+transform (negro al anclar, desalineación gutter). Y habilita
   la **selección/find sobre todo el stream** (Capa 4) en un único espacio
   `(fila global, columna)`, no card-por-card.
3. **Estado mínimo.** Los paneles son datos planos rearmados desde el modelo cada
   frame; no hay estado de widget por-panel que sincronizar o que se filtre.
   Colapsar/reordenar/insertar = cambiar la lista de items.
4. **La composición GPU encaja (Capa 3).** Como la superficie es dueña de todo el
   paint, compone **una** pasada GPU-directo (celdas de grilla) + **una** pasada vello
   (líneas/chrome) en una sola escena. Con controles independientes por panel, esa
   pasada única sería imposible.

Precio aceptado (no es gratis): el caller arma el chrome de **todos** los bloques por
frame (O(n_bloques); el control descarta los no visibles) y los paneles pierden estado
local salvo que se modele como dato. Para este dominio —líneas ilimitadas, bloques
acotados a lo que un humano tipea— es claramente conveniente: lo verdaderamente
ilimitado (las líneas) se virtualiza de raíz; los bloques están acotados por
naturaleza. Si algún día importara, el paso es **chrome lazy** (`Fn() -> View` por
item en vez del `View` ya construido).

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
  materializadas.
  - **Integración al shell ✅ (2026-06-05).** `output_pane_surface` en
    `shuma-module-shell/src/view.rs` mapea el modelo del shell
    (`OutputLine`/bloques/`collapsed`/`block_command`) a `Item`s: cada comando =
    un header chrome (`surface_header`: chevron + `$ cmd` + badge, click→colapso)
    + su cuerpo (rango en un `Scrollback`), reusando
    `body_lines_for_block`/`body_color_runs`/`CmdStatus`. Conversión de scroll
    `scroll_px` (desde el fondo) ↔ `scroll_y` (desde arriba); rueda/arrastre del
    widget → `Msg::Scroll(-delta)`. Detrás del flag **`SHUMA_TERMINAL_SURFACE`**
    (env, leído una vez); el `output_pane` viejo queda intacto para A/B y
    rollback. **Verificado** (`examples/dump_surface.rs`, viewport sembrado +
    scroll al fondo): flood de 3 000 líneas virtualizado, bloque colapsado,
    stderr tintado, anclado al fondo, sin negro, en la composición real del
    `view()`. 94 tests del shell pasan; `output_pane` sin cambios.
  - Deuda de paridad (no crítica): filas de etapa (tee) y chip de reprocess del
    header todavía no están en el chrome de la superficie; numeración global
    continua (no por-bloque). Se cierran antes de la migración (Fase 5).
- **Fase 3 — Selección + find sobre el stream.** Extraer el núcleo de selección del
  `text-editor` a compartido; selección global; copy; Ctrl+F.
- **Fase 4 — GPU directo grilla.** Atlas de glifos + celdas instanciadas para el modo
  grilla (TUI). Bench vs el grid vt100 actual. Híbrido.
- **Fase 5 — Pulido + migración. ✅** Anclaje estable bajo append, scroll inertial,
  spill a disco, y **borrado del `output_pane`/per-command-editor viejo**
  (2026-06-14): la superficie es la **única** vía de output (salvo PTY/TUI
  fullscreen). Se eliminaron `view/output_pane.rs`, la fn `command_card` +
  `pipe_stages_row`, `render_output_line` + sus helpers exclusivos
  (`build_span_children`/`kind_icon`/`partition_line`/`LinePiece`), el menú
  legacy (`view/chrome.rs::body_context_menu`), la maquinaria del editor IDE
  per-comando (`body_sel`/`body_menu`/`body_drag_accum`, `apply_body_pointer`,
  `apply_body_double_click`, `body_editor_state`, los `Msg`
  `BodyPointer`/`BodyDoubleClick`/`CopyBody`/`OpenBodyMenu`/`BodyMenu{Pick,Dismiss}`)
  y el flag `terminal_surface_enabled`/`SHUMA_TERMINAL_LEGACY`. Quedan, ahora
  como helpers compartidos por la superficie, `stage_capture_rows`,
  `copy_command_block`+`Msg::CopyCommandBlock`, `word_range_at`, `mix_color`,
  `ROW_H`/`STAGES_H`/`COLLAPSE_ANIM`, `pty_lines_panel` y `body_editor_metrics`/
  `body_editor_palette`. `cargo check --workspace` verde; 181 tests pasan
  (los 2 que fallan ya fallaban en `main`, sin relación con esto).

Cada fase es un commit (o pocos) verificado con render headless + viewport medido, y
deja el shell funcionando (flag de migración hasta la Fase 5).

## Cómo reemplazó al `output_pane` (sin romper)

- Fases 1–4 convivieron con el `output_pane` viejo detrás de un flag
  (`SHUMA_TERMINAL_SURFACE` / opt-out `SHUMA_TERMINAL_LEGACY`), para A/B y
  rollback inmediato.
- El modelo de datos no cambió de raíz: las `OutputLine` + `block_command` +
  `expanded_stages` se mapean a bloques de la superficie. El emulador vt100 y la
  detección de alt-screen se reusan.
- La **Fase 5 borró el camino viejo** (2026-06-14) una vez verificada la
  paridad: la superficie tiene su propia selección/copy/find/menú sobre el
  stream (`surf_*`), así que el editor IDE per-comando y su menú legacy ya no
  aportaban nada. Ya no hay flag: la superficie es el único path (salvo PTY/TUI
  fullscreen).

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

**Implementado al 2026-06-07; migración cerrada el 2026-06-14.** Fases 0-5 ✅ (foundation, virtualización, bloques, selección + copy + find, GPU grid behind `SHUMA_GPU_GRID=1`, pulido y migración). La superficie es **el único path** de output (salvo PTY/TUI fullscreen): el `output_pane` viejo + las cards per-comando IDE + su menú legacy + el flag `SHUMA_TERMINAL_LEGACY` fueron **borrados** (no hay más opt-out).

**Cerrado en Fase 5**:
- Anclaje estable bajo append (no más jiggle al recibir output mientras se lee historia).
- Doble-click select-word + triple-click select-line.
- Scroll inercial (touchpad/wheel decay).
- Menú contextual right-click (Copiar / Copiar todo / Seleccionar todo) sobre el stream (`surf_*`).
- Spill a disco: configurable vía `[scrollback]` en `shumarc.toml`, archive automático al recortar el frente, chip de status en UI, builtin `:scrollback open` para abrirlo con `$EDITOR`.
- **Borrado del `output_pane`/per-command-editor viejo** (2026-06-14): ver el detalle en la lista de Fases (Fase 5). Migración de `view()`/`body_view()` a `output_pane_surface` incondicional; eliminados los módulos/funciones/`Msg`/campos de `State` legacy; helpers compartidos reubicados. `cargo check --workspace` verde, downstream (`shuma-shell-llimphi`, `pata-llimphi`, `shuma-cli`) compila, 181 tests pasan (2 fallos pre-existentes en `main`).

**Pendiente** (post-Fase 5):
- Integrar líneas spilled al view (servirlas cuando el usuario scrollea way up — requiere extender el virtualizador para id < dropped via spill async/cache). Hoy el archive se ve mediante `:scrollback open` (fuera del shell).

Decisión de construir tomada con el usuario 2026-06-05; ejecución completa de Fase 0 a 5.10 entre 2026-06-06 y 2026-06-07. El control nuevo se justifica por el techo arquitectónico de ~500 líneas del path viejo y por la eficiencia GPU-directo en grilla/TUI.
