# Llimphi — motor gráfico soberano

> Llimphi (quechua: *color / brillo / pigmento*, en el sentido de "pintar la pantalla"). Tipo: **NATIVE GPU rendering suite**.

> **Regla dura para apps:** nada de cómputo pesado síncrono en `App::update`/`init`/handlers — congela la UI ("Not Responding"). Ver [COMPUTO-FUERA-DEL-HILO-UI.md](COMPUTO-FUERA-DEL-HILO-UI.md) (patrón worker + checklist por app, prioridad urgente).

## Tesis

Soberanía total sobre el píxel. Renderizar las geometrías exactas del simulador cósmico (`cosmos`), el compositor (`mirada`), las apps de escritorio (`nahual`) y el visor (`pluma`) sin cajas negras de Apple/Google/navegadores. Reemplazo total de **GPUI** en la pila gioser.

## Anatomía — 4 capas estrictas (S₀ → S₂)

Cada capa hace **una sola cosa** con precisión matemática.

```
[ CUADRANTE III · 0x02 RUWAY ]

4. llimphi-ui      — Lógica de Interfaz (Árbol Monádico / DAG UI)
   │                 (manejo de estado, eventos de teclado/ratón)
   ▼
3. llimphi-layout  — Motor de Layout (Cálculo Espacial)
   │                 (cajas, dimensiones, restricciones flex/grid)
   ▼
2. llimphi-raster  — Rasterizador Vectorial (La Brocha Fina)
   │                 (primitivas matemáticas → píxeles via Compute Shaders)
   ▼
1. llimphi-hal     — Abstracción de Hardware (Puente al Silicio)
   │                 (GPU o Framebuffer, sin importar el OS)
   ▼
[ HARDWARE · GPU / Pantalla ]
```

## Fases de forja

### Fase 1 — Puente al Silicio (`llimphi-hal`)

Aislar el motor del sistema operativo. Llimphi debe pintar tanto en una ventana Wayland controlada por `mirada` como en el framebuffer directo al arrancar `wawa`.

- **Abstractor:** `wgpu` (impl Rust de WebGPU sobre Vulkan nativo). Control de memoria seguro, bajísima sobrecarga.
- **Ventana:** `winit` para desarrollo en Linux. La arquitectura define un **trait `Surface`** abstracto: el día de mañana se desenchufa `winit` y se le pasa el puntero de memoria bruto del kernel `wawa`.
- **Hito:** Compilar, iniciar Vulkan por debajo, limpiar la pantalla pintándola de un solo color gris plomo a 144 Hz.

### Fase 2 — Brocha Matemática (`llimphi-raster`)

Pintar curvas y grafos orbitales con precisión Δ < 10⁻⁹ rad sin destrozar la CPU. En lugar de rasterizar píxel por píxel, **delegar todo el cálculo vectorial a los Compute Shaders de la GPU**.

- **Motor:** `vello`.
- **Integración:** Conectar la textura de salida de `wgpu` como lienzo destino de `vello`.
- **Ejecución:** Construir una `Scene` en `vello`. Pasarle primitivas geométricas puras (líneas, curvas de Bézier, texto).
- **Hito:** Renderizar en pantalla el grafo de un nodo estático con anti-aliasing perfecto calculado íntegramente por la GPU.

### Fase 3 — Física del Espacio (`llimphi-layout`)

Posicionar dinámicamente paneles, texto y ventanas requiere resolver ecuaciones de restricciones espaciales. No escribir un sistema propio de márgenes/padding: es un sumidero infinito.

- **Motor:** `taffy` (de la gente de Dioxus). Algoritmos Flexbox + CSS Grid en Rust puro.
- **Flujo:** Antes de decirle a `llimphi-raster` dónde pintar, pasar el árbol de nodos a `taffy` para calcular las coordenadas `(x, y, width, height)` absolutas de toda la interfaz.
- **Hito:** Paneles laterales y cajas que se redimensionan automáticamente, calculados en < 1 ms por frame.

### Fase 4 — Árbol de Estado Monádico (`llimphi-ui`)

El mayor problema de las interfaces (y por qué falló el paradigma OOP en esto) es el manejo del estado. Aquí se inyecta la cosmovisión estructural.

- **Arquitectura:** Nada de mutabilidad compartida (`Rc<RefCell<...>>` disperso). Unidireccional estilo Elm o **DAG (Grafo Acíclico Dirigido)**: el estado de la aplicación es **inmutable** y cada evento (click, tecla) genera una **nueva versión** del estado.
- **Bucle:**
  1. El usuario hace click (Input).
  2. El evento actualiza el Estado Global.
  3. El Estado Global reconstruye el Árbol UI.
  4. El Árbol pasa por `llimphi-layout` (Layout).
  5. Las coordenadas resultantes generan primitivas para `llimphi-raster` (Scene).
  6. `llimphi-hal` renderiza y hace el swap de la pantalla.

## Veredicto arquitectónico

No es una biblioteca genérica. Es un **motor de combate**. `wgpu + vello + taffy + DAG monádico` da un frontend capaz de competir en rendimiento con los mejores editores del mundo, diseñado como **traje a medida** para las topologías de gioser. Sin abstracciones de navegadores, sin cajas negras de Apple/Google.

## Pila exacta (sin negociación)

| Capa | Crate raíz | Deps externas |
|---|---|---|
| HAL | `llimphi-hal` | `wgpu`, `winit`, `raw-window-handle` |
| Raster | `llimphi-raster` | `vello`, `vello_encoding`, `peniko` |
| Text | `llimphi-text` | `parley` (shaping + fontique + swash, hereda vello via raster) |
| Layout | `llimphi-layout` | `taffy` |
| UI | `llimphi-ui` | `llimphi-{hal,raster,layout,text}` |

## Migración GPUI → Llimphi

Apps actualmente en GPUI que deben portarse:

- `02_ruway/nahual/*` (todas las apps GPUI: shell, file-explorer, database-explorer, image-viewer, text-viewer + 8 libs + 12 widgets)
- `02_ruway/mirada/mirada-launcher`, `mirada-portal`, `mirada-greeter`
- `00_unanchay/pluma/pluma-editor-gpui`
- `01_yachay/dominium/dominium-canvas-gpui`
- `01_yachay/cosmos/cosmos-app` (canvas + panels GPUI)

**Estrategia:** Las apps mantienen su lógica de dominio en sus `*-core` agnósticos. Solo se reemplaza la capa de presentación: en lugar de `use gpui::*`, pasan a usar `use llimphi_ui::*`.

## Estado (2026-05-31)

### Hecho
- Las 5 capas del framework en producción: `llimphi-hal` (wgpu+winit), `llimphi-raster` (vello), `llimphi-text` (parley, ahora con vello directo y texto multicolor en una pasada), `llimphi-layout` (taffy, con `LayoutTree::clear()` para reuso entre frames), `llimphi-ui` (bucle Elm + runtime winit).
- Split compositor/runtime: `llimphi-compositor` (winit-free: View tree, mount, paint/paint_gpu, hit-test) separado de `llimphi-ui` (runtime winit) → habilita un futuro runtime sobre el framebuffer de `wawa` sin winit.
- GPUI extinto (2026-05-26): toda app gráfica de la suite corre sobre Llimphi.
- Backend GPU directo (sin vello) completo y validado en hardware real (Iris Xe): `GpuPipelines` + `GpuBatch` + `View::gpu_paint_with`; ~11× vs vello a 1M puntos persistente, >140 fps.
- Catálogo de ~44 widgets: incluye text-editor (split en `-core` agnóstico + `-lsp`), nodegraph, tiled/panes/splitter, tree, list, grid (virtualizada 2D), gallery, timeline (scrub clickeable), menubar/edit-menu/context-menu, clipboard del sistema, tabs, modal, toast, y la familia de controles (button/field/slider/switch/segmented/...).
- 10 módulos compuestos: command-palette, diff-viewer, fif (find-in-files), file-picker, bookmarks, mini-map, shuma-term, symbol-outline, selector, plugin-host.
- `llimphi-workspace` (chasis tipo tmux) + `llimphi-gallery` (showcase) + `llimphi-motion`/`llimphi-icons`/`llimphi-surface` auxiliares.

### Pendiente
- Runtime sobre framebuffer de `wawa` (`WawaFramebufferSurface`) reusando el compositor winit-free — habilitado por el split pero aún no escrito.
- Backend GPU directo: sin MSAA/AA fino, sin texto, una sola `line_width` por flush; falta primer caller real denso (cosmos starfield) que mida una falla concreta antes de extender shaders.
- Widgets `llimphi-widget-{transport, waveform}` aún por extraer (la nota de media los deja como futuro no bloqueante).
- Investigación abierta: cuelgue/deadlock de apps Llimphi tras click/scroll (hipótesis `get_current_texture` Wayland FIFO) — pendiente reproducir+backtrace.

## Estado — bitácora histórica

- **2026-05-25:** SDD escrito. Esqueletos de los 4 crates creados.
- **2026-05-25 (tarde):** Las 4 fases en código y compilando. Examples:
  - `cargo run -p llimphi-hal --example clear_screen --release` — ventana gris plomo a refresh del display ✅ (verificado en hardware).
  - `cargo run -p llimphi-raster --example render_node --release` — nodo con AA perfecto vía vello/wgpu.
  - `cargo run -p llimphi-layout --example layout_panels --release` — sidebar + header/body/footer flex que se reorganiza al resize.
  - `cargo run -p llimphi-ui --example counter --release` — bucle Elm completo: click hit-test → update → view → layout → raster → present.
- **2026-05-25 (noche):** quinto crate `llimphi-text` (skrifa + vello). Bug de `max_storage_buffers_per_shader_stage` corregido (`Limits::default()` en vez de `downlevel`). `View::text()` permite poner texto centrado en cualquier nodo. Examples:
  - `cargo run -p llimphi-text --example hello_text --release` — "Llimphi" + tagline sobre fondo negro.
  - `counter` ahora muestra el número real (no barras) y los botones llevan label.
- **2026-05-25 (cierre):** dos fixes de hardware + parley.
  - **Storage write fix:** swapchain de muchos adapters Linux/Vulkan no acepta storage writes en Rgba8Unorm. Patrón nuevo: textura intermedia con `STORAGE_BINDING | TEXTURE_BINDING` donde pinta vello + `TextureBlitter` que la copia al swapchain en `Surface::present(frame, &hal)`. Cambio de API: `frame.present()` → `surface.present(frame, &hal)`.
  - **Paint-order fix:** `mount_recursive` registraba en post-orden y el background del root tapaba a los hijos. Ahora pre-orden depth-first.
  - **Parley:** llimphi-text reescrito sobre parley. API nueva: `Typesetter` (cachea FontContext + LayoutContext), `TextBlock { text, size_px, color, origin, max_width, alignment, line_height }`, `Alignment { Start, Center, End, Justify }`, `measure(&mut ts, &block)`. Bidi + ligatures + fallback CJK/emoji vía fontique. `hello_text` muestra título + párrafo justificado con script mixto Latin/Arabic/CJK.
- **2026-05-25 (cierre+1):** teclado en `llimphi-ui`. `App` gana `fn on_key(model, &KeyEvent) -> Option<Msg>` con default `None`. Re-export `Key` y `NamedKey` de winit. Runtime mantiene `Modifiers` state vía `ModifiersChanged`. `TextSpec` gana `alignment` (default `Center`, los labels de botón siguen igual) + `View::text_aligned(...)`. Example nuevo `editor`: text field con char insertion, backspace, enter, tab→4-spaces, ctrl+L limpia.
- **2026-05-26:** migración GPUI → Llimphi **completada**. GPUI queda extinto: toda app gráfica de la suite (pluma, mirada, cosmos, dominium, nahual, iniy, khipu, chasqui…) corre sobre Llimphi. No se agrega código nuevo sobre GPUI (ver regla dura §3 de `CLAUDE.md`).
- **2026-05-31:** split de `llimphi-widget-text-editor` (4328 LOC) → núcleo agnóstico `llimphi-widget-text-editor-core` (buffer/cursor/ops/undo/bracket/find/diagnostics/clipboard/highlight, sin render: sólo `peniko::Color`) + widget Llimphi (state + view) que lo re-exporta. Núcleo reutilizable en TUI/web/headless. `LayoutTree::clear()` para reusar el árbol taffy entre frames (`llimphi-layout`).
- **2026-05-31 (texto multicolor):** syntax highlighting en una sola pasada de shaping. `llimphi-text` gana `RunBrush` + `Typesetter::layout_runs` (color por rango de bytes vía `parley::RangedBuilder`/`StyleProperty::Brush`) + `draw_layout_runs`; `View::text_runs` lo expone. El editor pasó de un nodo (+ layout parley) por token a uno por línea.
- **2026-05-31 (split compositor/runtime):** `llimphi-ui` (1943 LOC) partido para separar la composición declarativa del runtime winit:
  - **`llimphi-compositor`** (nuevo, **winit-free**): el árbol `View<Msg>`, `mount` sobre taffy, `paint`/`paint_gpu` a `vello::Scene` y el hit-test. Depende sólo de `llimphi-layout` + `llimphi-text` + `vello` + `wgpu` (este último sólo por la firma de `GpuPaintFn`; `wgpu` no es windowing). **No depende de `llimphi-hal`.**
  - **`llimphi-ui`**: queda como el runtime winit (`App`/`Handle`/`run`/event loop/`KeyEvent`) y re-exporta el compositor entero → los consumidores siguen usando `llimphi_ui::View` etc. sin cambios.
  - Prerrequisito habilitado: `llimphi-text` ahora depende de `vello` directo (no de `llimphi-raster`), así que la pila de render (`compositor`→`text`/`vello`) es winit-free. Eso abre la puerta a un runtime sobre el framebuffer del kernel `wawa` (`WawaFramebufferSurface`) que reuse el mismo compositor sin arrastrar winit. `Renderer` (lo único que necesita `llimphi-hal`) se queda en `llimphi-raster`, consumido por `llimphi-ui`.

## Roadmap — GPU directo wgpu (sin vello)

### Por qué

`llimphi-raster` traduce hoy todo a `vello::Scene` (BezPath / kurbo /
peniko) y vello rasteriza vía compute shaders. Para 99 % de la suite
sobra: pluma editor, shuma shell, mirada compositor, nahual, iniy, khipu,
chasqui explorer, etc. pintan decenas a centenas de primitivos por frame.

El techo aparece cuando una app necesita rendir **>1 M primitivos por
frame**. En ese régimen el overhead de construir `BezPath`, ensamblar
buffers para los shaders internos de vello y hacer una pasada compute
por cada batch domina sobre el tiempo de raster real. Casos concretos
en gioser:

| App | Carga potencial | Trigger probable |
|---|---|---|
| **cosmos** | Catálogo Gaia DR3, mapas de cielo enteros | Starfield denso o sky-survey overlay |
| **tinkuy** | Particle engine N→∞ por diseño | Sim con > 10⁵ partículas |
| **nakui** | 100 K filas × 26 cols = 2.6 M celdas potencialmente visibles | Viewport con dataset grande |
| **dominium** | Mean-field con N agentes | Cuando se pase de 10³ a 10⁵ |
| **pineal** | Sus painters ya producen `Vec<f32>` interleaved (principio P1) — son los primeros listos para consumir el backend | Cualquiera de los anteriores que use pineal-* |

El techo es **horizontal**. Resolverlo en cualquier app individual sería
duplicación; el lugar es el motor.

### Qué es

Un backend alternativo en `llimphi-raster` que **salta vello** y sube
los slices de coordenadas directamente a vertex buffers `wgpu`, dispara
shaders WGSL chiquitos y emite una draw call por batch.

```
hoy:      painter → vello::Scene → BezPath → vello → wgpu → GPU
con esto: painter → GpuBatch     → vertex buffer    → wgpu → GPU
```

El trait que ven las apps (`Canvas` para pineal, `View::paint_with` para
llimphi-ui) **no cambia**. Cambia el implementador por debajo cuando se
elige "modo GPU directo".

### Trade-offs vs vello

| | Vello (hoy) | GPU directo |
|---|---|---|
| AA | Analítico, perfecto | MSAA hardware o supersample en shader |
| Curvas suaves | Bezier nativo | Hay que teselar primero |
| Texto | Sí, vello + parley | No — usar vello para text aunque coexista |
| Throughput primitivos | Bueno hasta ~100 K | Apto para 1–10 M |
| Costo de mantener | Cero (vello lo mantiene Linebender) | Shaders WGSL + pipelines propias |

Decisión: los dos backends **coexisten**. La app elige por hint
(`View::gpu_paint_with` para denso, `paint_with` para todo lo demás).

### Plan de tareas

**Fase 0 — Spike de medición (½ día). ✓ HECHO (2026-05-28).**
Benchmark sintético: pintar 100 K, 500 K y 1 M puntos con `SceneCanvas`
actual vs un mock GPU-directo (vertex buffer + shader trivial). Si el
factor no es ≥ 5× en el rango de 500 K, abortar — vello ya es
suficiente y no vale el costo de mantenimiento. Métrica de éxito: 60 fps
con 1 M puntos en GPU mid (Radeon 5500M, Intel Iris Xe).

Implementado en `llimphi-raster/examples/spike_gpu_directo.rs`. Cubre
ambos backends contra una textura `Rgba8Unorm` 1024×1024 headless,
warmup 5 + 15 frames medidos, bloquea hasta GPU idle (`Maintain::Wait`)
para que los `ms` reportados sean tiempo real CPU+GPU.

El binario `llimphi-gpu-bench` (en su propio crate) reporta info del
adapter wgpu + corre dos escenarios distintos: **rebuild por frame**
(LCG + `write_buffer` de 12-160 MB por frame, peor caso) y
**persistente** (buffer/Scene preparados UNA vez, bucle medido sólo
emite la draw call — caso real de cosmos/tinkuy/nakui).

**Resultados — Intel Iris Xe (TGL GT2), Mesa 26.1.1, Vulkan, 2026-05-28:**

Rebuild por frame:

| N | vello ms | directo ms | factor |
|---:|---:|---:|---:|
| 25K  | 7.3  | 1.2  | **6.05×** |
| 50K  | 12.9 | 1.4  | **8.94×** |
| 100K | 21.7 | 3.2  | **6.67×** |
| 200K | 26.1 | 6.1  | 4.30× |
| 500K | 94.4 | 18.0 | **5.25×** |
| 1M   | 202.4 | 49.0 | 4.13× |

Persistente (datos fijos, sólo redraw):

| N | vello ms | directo ms | factor | fps directo |
|---:|---:|---:|---:|---:|
| 100K | 18.6  | 0.8  | **22.55×** | 1210 |
| 500K | 34.1  | 3.4  | **9.97×**  | 293 |
| 1M   | 83.1  | 7.1  | **11.76×** | 141 |
| 2M   | 101.7 | 16.0 | **6.37×**  | 63 |
| 5M   | crash | 41.8 | —          | 24 |
| 10M  | crash | 79.7 | —          | 13 |

Veredictos contra el criterio del SDD:

- **Factor ≥5× a 500K**: ✓ PASA. Rebuild 5.25×, persistente 9.97×.
- **≥60 fps @ 1M**: ✓ PASA en persistente (141 fps); falla en rebuild
  (22 fps) — pero rebuild no es el use case real.
- **Techo de vello**: ~2 M paths en GPU mid. Más alto que mi hipótesis
  inicial (que era 200–300 K, contaminada por llvmpipe), pero existe.
  El path directo escala lineal a >10 M sin crashes.

Conclusión: el GPU directo cumple su propósito. La diferencia entre
rebuild y persistente (5–20×) confirma que el patrón correcto es
"datos cambian → vello, datos estáticos → GPU directo persistente".

**Fase 1 — Hook en `llimphi-ui` (1–2 días).**
Hoy `View::paint_with(F)` da
`F: Fn(&mut vello::Scene, &mut Typesetter, PaintRect)`. Agregar:

```rust
View::gpu_paint_with(F)
  where F: Fn(&wgpu::Device, &wgpu::Queue,
              &mut wgpu::CommandEncoder,
              &wgpu::TextureView, PaintRect)
```

El runtime de llimphi-ui ya tiene `Device`/`Queue` para vello; sólo hay
que exponer el `CommandEncoder` y `TextureView` del frame durante el
mount/paint. Compatibilidad: ambos hooks coexisten en el mismo View
tree; el orden de pintura sigue siendo pre-orden DFS.

**Fase 2 — Pipelines y shaders en `llimphi-raster` (3–5 días).**
Tres pipelines WGSL precompiladas y cacheadas:

- `lines_pipeline` — line list, anchura uniforme (expandida a tris en
  vertex shader como hace pineal-export::png).
- `tris_pipeline` — triangle list con per-vertex color.
- `rects_pipeline` — instanced quad con per-instance `[x, y, w, h, color]`.

Vertex format común: `[x: f32, y: f32, rgba: u32]`. Sin texturas; eso
queda para una fase posterior si aparece demanda.

**Fase 3 — `GpuBatch` accumulator (2–3 días).**
Estructura que las apps usan dentro del callback:

```rust
let mut batch = GpuBatch::new(device);
batch.add_lines(&coords, color);
batch.add_tris(&coords, &colors);
batch.add_rect(rect, color);
batch.flush(encoder, view);  // 1 draw call por pipeline usada
```

Grow strategy: vertex buffer dobla capacidad cada vez que se queda
chico. Sin copy back — vive del frame, se reusa el siguiente.

**Fase 4 — `GpuSceneCanvas` en pineal-render (1 día).**
Wrapper que implementa el trait `Canvas` de pineal usando `GpuBatch`
por debajo. Cero cambios en los painters. Permite usar el catálogo
entero de pineal en modo denso simplemente eligiendo el otro
constructor de Canvas dentro del `gpu_paint_with`.

**Fase 5 — Primer caller real (cosmos starfield, 2–3 días).**
Adaptar `cosmos-canvas-llimphi` para subir todas las estrellas del
viewport en una draw call usando `gpu_paint_with`. Métrica: dataset
HYG (~120 K estrellas brillantes) renderizadas a 144 fps en GPU mid.

**Fase 6 — Tests + demo + SDD (1 día). ✓ HECHO (2026-05-28).**
- `llimphi-raster/examples/gpu_million_points.rs`: usa `GpuPipelines` +
  `GpuBatch` puros (sin app, sin runtime Elm) para pintar N rects
  sintéticos. Validación headless del HAL + bench de referencia
  post-implementación. Smoke en `tests/gpu_batch_smoke.rs`.
- Tabla "cuándo elegir" → abajo.
- Pineal SDD §4 actualizado con `GpuSceneCanvas` en producción.

### ¿Cuándo elegir vello vs GPU directo?

| Pregunta | Vello (`paint_with`) | GPU directo (`gpu_paint_with`) |
|---|---|---|
| ¿Cuántos primitivos por frame? | < ~500 K (rebuild) o < ~2 M (Scene reusada) | 100 K – 10 M+ |
| ¿Los datos cambian cada frame? | Sí — vello rebuild es barato hasta 500 K | Posible pero con coste de `write_buffer`; ideal estático |
| ¿Curvas Bezier nativas? | Sí | No (teselar antes) |
| ¿Texto? | Sí | No — usar vello hermano u overlay |
| ¿AA fino requerido? | Sí (analítico) | No (sin MSAA todavía) |
| ¿Múltiples grosores de stroke? | Sí | Una sola `line_width` por flush |
| ¿Anti-fluctuación de pixel? | Sí | Subpixel jitter visible |
| Ejemplos de uso | pluma editor, shuma shell, mirada, nahual, iniy, khipu, chasqui explorer, dominium UI | cosmos starfield denso, tinkuy particles, nakui viewport, pineal denso |

Default razonable: **`paint_with`** salvo que el caller ya midió que el
volumen lo justifica. El costo de mantener un pipeline + WGSL propios
es alto comparado con seguir usando vello.

Patrón "buffer persistente": para el use case denso real (catálogo
fijo, particles iniciales, dataset estático), construir el
`wgpu::Buffer` y `BindGroup` UNA vez con `GpuPipelines::{rects, tris,
lines, bind_layout}` expuestos y emitir el draw call manualmente
desde el `gpu_paint_with` reusando esos recursos. Eso da factores
~11× vs vello a 1M en GPU mid (medido Iris Xe), y >140 fps.
`GpuBatch` queda para datos transitorios (UI dinámica densa).

Convivencia: una misma `View` puede registrar AMBOS hooks. El runtime
pinta vello primero (toda la Scene), luego ejecuta los GPU painters
en orden DFS. Para texto encima de un render GPU denso, se usa
`App::view_overlay` (segunda Scene vello sobre el main).

**Estimado total: 10–15 días de trabajo concentrado.**
**Trabajo real (1 día, 2026-05-28):** todas las fases completas, sólo
falta validar el criterio formal (≥5× a 500K, 60 fps @ 1M) en GPU mid
real — el bench corrió en llvmpipe.

### Trigger

No empezar hasta tener un caller real que mida una falla concreta.
El candidato natural es cosmos (starfield Gaia o sky-survey overlay).
Hasta entonces, el item queda acá en este SDD como decisión arquitectónica
tomada — todas las apps saben que el techo existe y que la salida
está diseñada.

### No-objetivos explícitos

- **No** reemplazar vello. Coexisten — vello para vector/text/AA fino,
  GPU directo para volumen.
- **No** hacer un layer de abstracción tipo Skia. El trait `Canvas` de
  pineal y el `paint_with` de llimphi son la abstracción; no se agrega
  más arriba.
- **No** soportar texto en el backend GPU directo. Texto siempre por
  vello+parley; si una vista mezcla millones de puntos + labels, hace
  `gpu_paint_with` para los puntos y un `paint_with` superpuesto para
  los labels.
