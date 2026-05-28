# Llimphi — motor gráfico soberano

> Llimphi (quechua: *color / brillo / pigmento*, en el sentido de "pintar la pantalla"). Tipo: **NATIVE GPU rendering suite**.

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

## Estado

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
- **Próximo:** migración GPUI → Llimphi (Pluma editor primero, valida text editing real).

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

**Fase 0 — Spike de medición (½ día).**
Benchmark sintético: pintar 100 K, 500 K y 1 M puntos con `SceneCanvas`
actual vs un mock GPU-directo (vertex buffer + shader trivial). Si el
factor no es ≥ 5× en el rango de 500 K, abortar — vello ya es
suficiente y no vale el costo de mantenimiento. Métrica de éxito: 60 fps
con 1 M puntos en GPU mid (Radeon 5500M, Intel Iris Xe).

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

**Fase 6 — Tests + demo + SDD (1 día).**
- `llimphi-raster` example: `gpu_million_points` (LCG + shader, sin
  ninguna app, valida el HAL).
- Update SDD: tabla "cuándo elegir vello vs GPU directo".
- Pineal SDD: anotar que `GpuSceneCanvas` ya está disponible.

**Estimado total: 10–15 días de trabajo concentrado.**

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
