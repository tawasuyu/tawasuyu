# Llimphi · Paridad gráfica con Flutter — roadmap

> Diagnóstico al 2026-06-05, verificado contra el código (`llimphi-compositor`,
> `llimphi-raster`, `llimphi-text`, `llimphi-motion`). Mide qué falta **a nivel
> gráfico** para que Llimphi compita con un Flutter. Fuente autoritativa ante
> divergencia: el `lib.rs`/`view.rs` de cada crate.

## Piso actual (ya a paridad)

Layout Flexbox+Grid (taffy) · shaping bidi/CJK/emoji/ligaduras (parley) ·
transforms afines con hit-test correcto · drag&drop · IME · foco/Tab ·
overlays · virtualización (list/grid/tree) · camino GPU directo · ~45 widgets ·
theming semántico · tweens. El render vectorial AA con Bézier y gradientes
**existe** vía `paint_with` — buena parte del plan es *exponerlo* como propiedad
de `View`, no inventarlo.

## Regla de decisión: contrato vs composición

Antes de pre-analizar un control de otro framework, clasificarlo:

- **Composición o medida sobre primitivas que ya existen → sale solo.** No lo
  pre-analices; emerge cuando un caller lo pida y cuesta poco retrofitear.
  Ejemplos: `autotextsize` (es el inverso de `layout_clamped` — binary-search del
  tamaño de fuente sobre la misma medida), la "decoración rica de input" de
  Flutter (composición en `field` sobre bordes reales del Tier 1 + floating-label
  del Bloque 4; el `text-input` se queda desnudo), acordeones, steppers.
- **Contrato/protocolo entre capas → reservá la forma de la API ahora**, aunque no
  lo implementes. Esto es lo caro de retrofitear porque toca el *seam*
  compositor/runtime y rompe callers si cerrás la firma sin contemplarlo.

### Los cuatro seams a reservar (todo lo demás es composición)

1. **Viewport de scroll (slivers / collapsing app bar / sticky headers)** — Tier 5.
   **Resuelto (Bloque 7):** como Llimphi es Elm y el offset vive en el Model,
   `view()` ya puede construir hijos que reaccionan al offset — no hizo falta un
   seam de runtime nuevo. (a) lista virtualizada [ya estaba], (b) header
   colapsable = `sliver_app_bar`, (c) sticky = `sticky_y`. Todo composición.
2. **Arena de gestos (desambiguación)** — Tier 4. **Parcial (Bloque 6):**
   long-press/double-tap/pinch ya tienen árbitro (hit-test por gesto + árbitro
   temporal en `about_to_wait`). Falta rotate/fling y, si aparecen gestos que
   compitan por el mismo press, un grafo de desambiguación competitivo.
3. **Árbol de semántica (AccessKit)** — Tier 7. Árbol paralelo al `View`.
4. **Build sensible al tamaño (`LayoutBuilder` / `MediaQuery` breakpoints)** —
   **Resuelto (Bloque 9).** El `MediaQuery` a nivel ventana ya era posible
   (`on_resize` → el Model → `view()` ramifica por breakpoint). El gap real era
   el **`LayoutBuilder` por-nodo** (construir según el slot local, que depende del
   flex/hermanos, no de la ventana): `View::layout_builder(|Constraints| -> View)`,
   resuelto en **dos pasadas** por el runtime (`resolve_layout_builders`): monta
   con los builders como hojas, computa para conocer sus slots
   (`collect_builder_constraints`), y reconstruye un `view()` fresco expandiendo
   cada builder con sus constraints (`expand_layout_builders`). Coste cero sin
   builders (`has_layout_builder`). Límite v1: sin anidamiento. Demo
   `--example layout_builder_demo` (1 vs 2 columnas según el slot).

## Tiers por retorno de inversión

### 🟢 Tier 1 — exponer lo que vello ya hace (alto impacto, bajo costo)
Hoy cada widget las simula a mano (badge inventa el gloss; context-menu/text-input
fingen el borde con un rect-padre inset).

| Falta | Función a desarrollar | Dónde | Notas |
|---|---|---|---|
| ✅ Sombras / elevación | `.shadow(Shadow{ blur, offset, color, spread })` | compositor | vello: `draw_blurred_rounded_rect` nativo. Brecha #1. |
| ✅ Gradientes como fill | `.fill_gradient(Gradient)` (linear/radial/sweep) | compositor | `peniko::Gradient` ya está. |
| ✅ Bordes reales | `.border(width, color)` | compositor | stroke de rounded-rect; mata el truco del rect-padre. Respeta radio por esquina. |
| ✅ Radio por esquina | `.radius_corners(tl,tr,br,bl)` | compositor | override de `radius` uniforme; sombra sigue usando el escalar. |
| ✅ Backdrop blur (glass) | `.backdrop_blur(sigma)` | hal + ui | Bloque 11 — Gauss separable (H+V) sobre la intermediate, post-pasada wgpu. Limitación v1: el rect del nodo se borronea **completo** (incluyendo su propio fill/border/text) — para "vidrio + texto nítido" se compone como nodo hermano posterior con el mismo rect (el text se pinta sobre el blur ya aplicado). Tier 1 cerrado. |

### 🟢 Tier 2 — texto rico (parley lo soporta, falta exponerlo)
`TextSpec` hoy expone `italic`, color por rango y **peso de fuente**.

- ✅ Peso/bold → `weight: f32` en `TextSpec` + `.text_weight(...)`/`.bold()`. Fluye por medida y pintado (camino directo a `Typesetter::layout`, no `TextBlock`).
- ✅ Overflow/ellipsis (`maxLines` + `…`) → `.ellipsis(n)`/`.max_lines(n)` + `Typesetter::layout_clamped`. Clampa medida y pintado; recorta graphemes hasta caber. Cubre single-line y N líneas.
- ✅ Decoración: subrayado / tachado → `.underline()` / `.strikethrough()` en `View` + `underline`/`strikethrough` en `TextSpec`/`TextMeasure`. parley emite `StyleProperty::{Underline,Strikethrough}` por bloque; el pintado (`draw_layout_*`) recorre los runs y emite el rect en `baseline - offset` con `underline_size`/`strikethrough_size` del font metric. Brush = mismo color que el texto (`Layout<()>` toma el `color` externo; `Layout<RunBrush>` el brush del run). `ShapeKey` separa las claves del caché de shaping. Funciona junto con `weight`, `italic`, `ellipsis` y multicolor.
- Spans inline mixtos (tamaño/peso/familia/link por rango, no sólo color) → `RichText` real.
- Texto seleccionable fuera del editor (selección + copiar).

### 🟡 Tier 3 — animación declarativa (brecha de arquitectura)
Hay `Tween` + `animate()`, pero cada animación se cablea a mano (tween en Model +
`spawn_periodic` + guard de generación).

- ✅ Animaciones **implícitas** (`AnimatedContainer`): `View::animated(key, dur)`
  + `AnimRegistry` (estado retenido en el runtime, keyado por `key` estable). El
  runtime reconcilia el árbol entre layout y paint, interpola `fill`/`radius` y
  pide otro frame mientras alguna anima (ticker autodetenido, sin `spawn_periodic`).
  Falta extender a más props (alpha, border, size→requiere re-layout) y `AnimatedOpacity`.
- ✅ Ticker central: el redraw se reencola solo mientras haya animación viva; se
  detiene al asentarse. Reemplaza N `spawn_periodic` para las implícitas.
- Curvas: hoy 3 easings (`linear`, `ease_out_cubic`, `ease_in_out_cubic`) + spring physics.
- Transiciones de página + Hero (shared-element). Hoy no hay routing.

### 🟡 Tier 4 — gestos · **parcial (Bloque 6)**
Hoy: tap, drag(delta), scroll **+ pinch/scale (zoom), double-tap, long-press**.
- ✅ **Pinch-to-zoom** (`View::on_scale` + `GesturePhase` + `hit_test_scale`):
  factor multiplicativo incremental + focal local. **Ctrl+rueda** lo sintetiza
  en cualquier desktop (Wayland/Windows no emiten el pinch del trackpad);
  `PinchGesture` lo cubre en macOS. Desbloquea el zoom de canvases
  (pineal/cosmos/nakui).
- ✅ **Double-tap** (`on_double_tap[_at]`) y **long-press** (`on_long_press[_at]`):
  eventos **aditivos** (hit-test propio, no tocan click/drag). El árbitro es el
  tiempo — double-tap = dos presses dentro de 400 ms y <16px; long-press = press
  que sobrevive 500 ms quieto (lo vence `about_to_wait`, lo cancelan movimiento
  >8px o release). Caso canónico limpio: canvas con pan + gesto sin `on_click`.
- Falta: **rotate** (trackpad, sólo macOS — plumbing igual al scale, sin
  fallback de teclado), **velocity/fling** (vive con scroll-physics, Tier 5), y
  pinch **multi-touch real** desde eventos `Touch` (touchscreen wawa/android:
  trackear dos dedos y derivar distancia → factor). La arena hoy es por-gesto
  (cada uno su hit-test + su árbitro temporal), no un grafo de desambiguación
  competitivo entre gestos rivales — alcanza para el set actual; ampliable si
  aparecen gestos que compitan por el mismo press.

### 🟡 Tier 5 — scroll avanzado · **parcial (Bloque 7)**
Antes: sólo `scroll_y` vertical. Ahora (todo stateless, offset en el Model):
- ✅ **Scroll 2D / horizontal** — `scroll_xy(offset:(x,y), content_size,
  viewport_size, …)`, una barra por eje con overflow (reusa `thumb_geometry`).
- ✅ **Física momentum/bounce** — `fling_step`/`fling_settled` (decaimiento
  exponencial, integral exacta indep. del frame-rate) + `rubber_band` overscroll
  estilo iOS. Pure helpers; el caller los driverea por frame (patrón `approach`).
- ✅ **Slivers** — `sliver_app_bar(offset, header_max, header_min, header(frac),
  …)`: un offset colapsa el header y luego scrollea el cuerpo (el seam #1
  "extent-por-offset", resuelto como composición sobre offset-en-Model).
  Helpers: `collapsed_height`/`collapse_fraction`/`sliver_max_offset` +
  `sticky_y` (encabezados de sección pegados). Demo: `--example scroll_avanzado`.
- Falta: **scrollbar persistente con auto-hide**, **scroll anidado** (hoy el
  `on_scroll` del nodo más al frente consume; falta el "pasar el sobrante al
  padre" al llegar al tope), **pull-to-refresh** (patrón móvil, diferido), y un
  **builder de lista sticky** llave-en-mano (hoy `sticky_y` es el helper; el
  caller posiciona los encabezados). El **fling desde arrastre** necesita
  capturar velocidad del drag (timestamp por evento) — hoy el helper existe pero
  el caller estima la velocidad; un seam de "drag con velocidad" lo haría directo.

### 🟠 Tier 6 — assets / media
- ✅ SVG (`llimphi-svg` vía `vello_svg`, Bloque 3 del backlog post-elegance).
- ✅ Imágenes: `fit:{Contain,Cover,Fill,None}` + clip redondeado nativo (Bloque 12).
  `View::image_fit(ImageFit::...)`; `Contain` sigue siendo el default. El paint
  envuelve la imagen en `push_layer(Mix::Clip, node_rrect)` (antes era un
  `KurboRect` plano), así avatares y cards con `radius`/`corner_radii` ya no
  necesitan un padre con `clip(true)`. Demo `--example image_fit_demo`
  (Contain · Cover · Fill · None + un cuadrado con radius=100 que queda como
  avatar circular).
- Falta: decode pipeline (hoy el caller decodifica con el crate `image` antes
  de pasar el `peniko::Image`) e imágenes de red con caché.

### 🔴 Tier 7 — accesibilidad (la brecha categórica más grande)
**Cero hoy.** No hay árbol de semántica ni lectores de pantalla. A desarrollar:
un árbol de semántica paralelo al `View` (rol, label, estado, acciones por nodo)
+ integración **AccessKit** (estándar Rust, se enchufa a winit) que lo traduce a
UIA/AT-SPI/VoiceOver. Imprescindible para "competente" en serio; se difiere, no se
omite. Ver explicación extendida abajo.

### 🟡 Tier 8 — arquitectura de render / performance
Cada frame reconstruye todo el árbol `View` y vello rerasteriza la escena
completa. No urge (vello es rápido), pero separa "fluido a 5k nodos" de "a 50k".

- **Caché de shaping de texto ✅** (`llimphi-text`, 2026-06-05). `Typesetter::layout`
  cachea el `parley::Layout` por sus parámetros (caché generacional LRU-aprox); el
  texto que no cambió no se re-shapea. Medición y pintado comparten el chokepoint,
  así que cubre el reuso intra-frame (medir→pintar) y entre frames (scroll/tipeo).
  Evidencia (`examples/bench_cache`): 89 µs/frame → 6 µs/frame en UI típica (14×),
  63× con texto estable. API sin cambios; transparente para todas las apps.
- **Retención de frame entero (conservadora) — próximo, diferido.** Plan elegido:
  el `view()` Elm es puro sobre el modelo y el modelo sólo muta en `update()`.
  Contador de generación en el `State` del eventloop, bumpeado en cada `A::update`
  (~25 call-sites). En `RedrawRequested`, si la generación, la posición del cursor
  (hover), el tamaño no cambiaron y no hay anim/ripple/ghost/drag activos → saltar
  rebuild+mount+layout+paint y re-presentar la `Scene` retenida. Cero riesgo de
  stale (no cachea por-subárbol). Mata redraws redundantes/espurios.
- **`RepaintBoundary` (sub-escenas locales retenidas) — más adelante.** El win
  marginal baja una vez que el caché de shaping ya elimina el costo caro del paint;
  reservado para subárboles grandes y estáticos (página de pluma, chart de
  tullpu/cosmos). Requiere gateo estricto (sólo subárboles paint-puro: sin
  hover/anim/ripple/gpu ni ancestros transformados, porque ese estado lo maneja el
  runtime y no la `version` de la app → frame stale) + verificación con PNG-diff
  headless (cache vs paint fresco = pixel-idénticos). Captura en coords locales
  (`paint_range` con `base_xf = translate(-origin)`) + replay con `scene.append`,
  calcando el mecanismo ya probado de los ghosts de animación de salida.

## Orden de ejecución sugerido

1. ✅ **Bloque 1 = Tier 1 (sombra+gradiente+borde)** — builders de `View` sobre
   primitivas existentes. Máximo retorno visual; limpia deuda de widgets que las
   fingen.
2. ✅ **Bloque 2 = radio por esquina + peso de fuente** — cierra Tier 1 (salvo
   backdrop-blur) y abre Tier 2. `.radius_corners(...)`, `.text_weight(...)`/`.bold()`.
3. ✅ **Bloque 3 = overflow/ellipsis** — `.ellipsis(n)`/`.max_lines(n)` +
   `Typesetter::layout_clamped`. Crítico para listas/labels/celdas.
4. ✅ **Bloque 4 = animaciones implícitas** — `View::animated(key, dur)` +
   `AnimRegistry` + ticker autodetenido. Interpola fill/radius; ampliable.
5. **Bloque 5 = quick wins de la cosecha** —
   - ✅ **forma de cursor** (`View::cursor(Cursor)`, enum llimphi-native mapeado
     a winit en el runtime; herencia CSS gratis vía `hit_test_cursor`;
     consumidores: text-input=Text, splitter=Col/RowResize, button=Pointer).
   - ✅ **scrollbar arrastrable** — ya estaba en `llimphi-widget-scroll`: el
     thumb es `.draggable(...)` y convierte el delta de px a delta de offset vía
     `thumb_geometry`. (El roadmap lo listaba pendiente por error.)
   - ✅ **animación de contenido completa (opacidad + entrada + salida)** —
     `alpha` entra en `AnimSnapshot` (regla `None ≡ opaco`), así
     `.alpha(x).animated()` interpola opacidad; `View::animated_enter` hace
     fade-in en la primera aparición; `View::animated_exit` hace fade-out al
     desmontarse (el runtime captura la subescena vello del nodo mientras vive y
     la reproduce como fantasma con `replay_ghosts` cuando la key desaparece);
     `View::animated_inout` para ambas. El exit tiene coste por frame
     (captura el subárbol) → usar en pocos nodos.
6. ✅ **Bloque 6 = Tier 4 gestos (parcial)** — `on_scale` (pinch-zoom vía
   Ctrl+rueda + `PinchGesture`), `on_double_tap`, `on_long_press` + arena por
   tiempo (`about_to_wait` vence el long-press). Falta rotate + fling + pinch
   multi-touch. Demo: `--example gestos`.
7. ✅ **Bloque 7 = Tier 5 scroll avanzado (parcial)** — `scroll_xy` (2D),
   `fling_step`/`rubber_band` (física), `sliver_app_bar`/`sticky_y` (slivers).
   Falta scroll anidado, scrollbar auto-hide, pull-to-refresh, fling-desde-drag.
   Demo: `--example scroll_avanzado`.
8. ✅ **Bloque 8 = ripple/InkWell** — `View::ripple(key, color)` + `RippleRegistry`
   (retenido en el runtime). El press dispara la salpicadura (`hit_test_ripple`,
   aditivo); el runtime la pinta tras el contenido como círculo expansivo
   recortado al contorno, atenuado por el fade, con ticker autodetenido.
   Consumidor: `button_ripple`. Demo: `--example ripple_demo`.
9. ✅ **Bloque 9 = LayoutBuilder (4º seam)** — `View::layout_builder(|Constraints|
   -> View)` + resolución en dos pasadas en el runtime
   (`resolve_layout_builders`): mount-builders-como-hojas → compute →
   `collect_builder_constraints` → `expand_layout_builders` sobre un `view()`
   fresco. Coste cero sin builders. Funciones puras testables en
   `llimphi-compositor/src/layout_builder.rs`. Demo `--example layout_builder_demo`.
   Límite v1: sin anidamiento.
10. ✅ **Bloque 10 = decoración inline de texto (Tier 2)** —
    `.underline()` / `.strikethrough()` en `View` (campos `underline`/`strikethrough`
    en `TextSpec` y `TextMeasure`). `Typesetter::{layout,layout_clamped,layout_runs}`
    aceptan los flags y empujan `StyleProperty::{Underline,Strikethrough}`. El
    pintado (`paint_decoration` en `draw_layout_brush_xf` y `draw_layout_runs_xf`)
    recorre los runs y, si parley registró la decoración, emite un rect en
    `baseline - offset` con `underline_size`/`strikethrough_size` del font
    metric — el grosor es proporcional al `size_px`. `ShapeKey` extiende su
    clave con los dos bools para no mezclar layouts con vs sin decoración en
    el caché. Test: `underline_y_strikethrough_se_propagan_al_layout` verifica
    que el `Style` del run las marca cuando los flags están y no las marca
    cuando no están. Tier 2 queda sólo con **spans inline mixtos** (RichText
    real) y **texto seleccionable fuera del editor**. **AccessKit (Tier 7)**
    completo desde iter 3/3 (2026-06-07).
11. ✅ **Bloque 11 = backdrop blur (cierra Tier 1)** — `View::backdrop_blur(sigma)`
    + `MountedNode::backdrop_blur: Option<f32>` + `collect_backdrop_blurs(mounted,
    computed)` en el compositor. En `llimphi-hal` un nuevo `BlurCompositor` aplica
    una **Gauss separable** (dos pases H+V) sobre la intermediate vía fragment
    shader con scissor restringido al rect del nodo y una scratch texture
    interna (full-viewport, recreada en resize). El bind group lleva sampler
    bilinear clamp-to-edge + UBO con `(direction, pixel_size, sigma, radius)`;
    `radius = ceil(sigma*3)` cap en 32px (sigmas > ~10 empiezan a clipear cola).
    El runtime (`llimphi-ui::eventloop`) recolecta los blurs DESPUÉS de la
    pasada vello y ANTES de los `gpu_painter`, así un painter GPU cuya rect se
    solape ve el backdrop ya borroso y se pinta encima nítido. Demo `--example
    backdrop_blur_demo` (4 paneles σ ∈ {0,4,8,16} sobre franjas R/G/B/Y): σ=0
    conserva fill+border+radio (el compositor no-op'ea); σ>0 pierde
    fill+border+radio propios porque la post-pasada los promedia con el fondo
    — la **limitación honesta v1**. Para "glass + chrome nítido" la
    composición canónica es: panel `.backdrop_blur(σ)` sin fill propio + nodo
    hermano POSTERIOR con el mismo rect aportando borde/texto/iconos (el
    blur ya pasó cuando llega el chrome del hermano… mentira: la post-pasada
    es un único pase que afecta a TODO lo de la intermediate dentro del rect.
    Para chrome nítido sobre el blur, el chrome real va por `gpu_painter` o
    por el **overlay** — que se compone DESPUÉS del blur). La paridad estricta
    con CSS `backdrop-filter` (chrome propio nítido en el mismo nodo sin
    overlay) requiere scene-split (Bloque 11.B futuro): pintar el árbol sin
    el subárbol del blur, borronear el rect, y luego pintar el subárbol sobre
    una textura secundaria que se compone con alpha-over (reusa el camino
    existente del `OverlayCompositor`).
12. ✅ **Bloque 12 = `ImageFit` (avanza Tier 6)** — `View::image_fit(ImageFit)`
    + `MountedNode::image_fit: Option<ImageFit>` (default `Contain`,
    preservando el comportamiento histórico de `View::image()`). Cuatro
    políticas: `Contain` (preserva aspect, cabe), `Cover` (preserva
    aspect, cubre + recorta), `Fill` (estira, no preserva), `None` (1:1
    centrada, recorta lo que sobra). El paint del nodo con imagen ahora
    envuelve el `draw_image` en `push_layer(Mix::Clip, node_rrect)` (antes
    `KurboRect` plano), así avatares y cards con `radius`/`corner_radii`
    se recortan a la silueta sin necesidad de un padre `clip(true)`.
    Demo `--example image_fit_demo` (cinco fichas: las cuatro políticas
    sobre la misma imagen sintética 4:3 + un cuadrado con `radius=100`
    que queda como avatar circular).

## Tier 7 — detalle (accesibilidad)

**Qué es.** Una app gráfica pinta píxeles; un lector de pantalla (NVDA, VoiceOver,
Orca, TalkBack) no ve píxeles: lee un **árbol de semántica** que la app publica al
SO. Cada nodo dice *qué es* (rol: botón/checkbox/heading/textfield), *cómo se
llama* (label/value), *en qué estado* (checked/disabled/selected/expanded) y *qué
acciones acepta* (activar, incrementar, enfocar). El SO lo expone por su API de
accesibilidad: UIAutomation (Windows), AT-SPI (Linux), NSAccessibility (macOS).

**Por qué Llimphi hoy da cero.** Llimphi pinta `View`s sobre la GPU sin árbol
nativo del SO. Para el lector de pantalla la ventana es un rectángulo opaco: no
hay "botón Guardar", no hay foco anunciable, no hay navegación por elementos.
Tampoco hay teclado-only completo a nivel semántico (Tab mueve foco visual, pero
nadie *anuncia* a dónde fue). Es exactamente el mismo problema que tuvo Flutter
(que renderiza su propio árbol con Skia) y que resolvió con una **capa de
semántica** sintetizada aparte del árbol de render.

**La pieza a desarrollar.**
1. Un **árbol de semántica** paralelo al `View`: cada `View` puede llevar
   `.semantics(SemanticsSpec{ role, label, value, flags, actions })` y el
   compositor, al montar, produce un `SemanticsTree` (igual que produce el árbol
   de layout). Los widgets ya saben su rol — `button_view` setea `role=Button`,
   `switch` `role=Switch + checked`, etc.
2. Integrar **AccessKit** (`accesskit` + `accesskit_winit`): es el estándar Rust
   que traduce un árbol genérico a UIA/AT-SPI/macOS y ya tiene adaptador winit. El
   runtime (`llimphi-ui`) empuja el `SemanticsTree` a AccessKit cada vez que
   cambia, y rutea de vuelta las acciones del lector (p. ej. "activar botón X")
   como `Msg` al `update`.
3. Conectar **foco** (ya existe `focusable(id)` + `on_focus`) al nodo semántico, y
   exponer las **acciones** (activar = el `on_click` del nodo).

**Costo/forma.** Es un subsistema nuevo pero acotado y bien precedido: el patrón
"árbol paralelo sintetizado + AccessKit" es justo lo que hace Flutter y lo que
AccessKit fue diseñado para soportar. Encaja limpio en el split compositor/runtime:
el árbol se sintetiza en `llimphi-compositor` (winit-free) y `llimphi-ui` lo
empuja a AccessKit. Se difiere por prioridad, no por dificultad arquitectónica.
Cuando se haga: empezar por roles básicos (button/text/heading/checkbox/textfield)
+ foco + acción activar; el resto incrementa.

## Cosecha de otros frameworks (2026-06-05)

Ojeada a Flutter / SwiftUI / Jetpack Compose buscando piezas valiosas que **no**
estén ya en los tiers. Cada fila verificada contra el código, no contra la memoria.
Clasificadas por la regla contrato-vs-composición de arriba.

### Hacer ya — barato y alto retorno (completan lo que existe)

| Pieza | Análogo | Estado verificado | Por qué |
|---|---|---|---|
| ✅ **Forma de cursor** (`View::cursor(Cursor)`) | `MouseRegion.cursor` / `SystemMouseCursors` · Compose `pointerHoverIcon` | **Hecho 2026-06-05** | Enum `Cursor` llimphi-native (19 formas) en el compositor; `hit_test_cursor` da herencia CSS (hijo sin cursor cae al ancestro); `llimphi-ui` lo mapea a `winit::CursorIcon` y lo aplica en la transición de hover. Consumidores: text-input=Text, splitter=Col/RowResize, button=Pointer. |
| ✅ **Animación de contenido** (enter/exit + opacidad) | `AnimatedSwitcher` · `AnimatedList` · `AnimatedVisibility` | **Hecho** (Bloques 4–5): props (fill/radius) + opacidad animable + `animated_enter` (fade-in) + `animated_exit` (fade-out) + `animated_inout`. | El exit se resolvió capturando la subescena vello del nodo mientras vive (`paint_range`) y reproduciéndola como fantasma (`replay_ghosts`) cuando la key desaparece — sin resucitar el árbol. Falta sólo el **cross-fade real** entre dos identidades bajo la misma key (hoy se logra combinando enter+exit de dos keys). |
| ✅ **Scrollbar interactiva** (drag del thumb) | `Scrollbar` arrastrable | **Hecho** en `llimphi-widget-scroll` | El thumb es `.draggable(...)` y convierte delta-px del arrastre a delta-offset vía `thumb_geometry`; sólo aparece si hay overflow. (Antes figuraba pendiente por error.) |

### Reservar el seam — ya cubierto arriba

`LayoutBuilder`/breakpoints (4º seam), slivers (Tier 5), arena de gestos (Tier 4),
semántica (Tier 7). No repetir; ver "Los cuatro seams a reservar".

### Backlog con forma de ERP (composición sobre `field`/`grid`, construir cuando el dominio lo pida)

La suite tiene dominium/ERP y formularios — estas son composición pura, no protocolo:

- **Framework de validación** (`Form`/`FormField`/validators, validar-al-submit,
  agregación de errores) sobre el `field` que ya tiene `error`/`required`.
- **Pickers concretos**: fecha/hora y color. Faltan del catálogo; los formularios
  los piden. Composición sobre overlay + grid.
- **DataTable read-only ordenable/paginable** — distinto de `nakui-sheet` (que es
  motor de cálculo): una tabla liviana para listar registros. allichay ya marcó
  "tablas/listas = v2".
- **Accordion / expansion panel**, **stepper / wizard** — triviales, composición.

### Saber que es "gratis" con el camino GPU (diferenciador, no urgente)

- **Shaders de fragmento / efectos de material** (`FragmentShader` Flutter ·
  `RenderEffect` Compose) — `wgpu` lo habilita. **backdrop-blur (glass)** es el
  primer caso concreto y es justo el único pendiente del Tier 1.
- **Lottie / vector animado** cae bajo SVG (Tier 6, existe `vello_svg`).

### Wishlist ampliada de controles (anotados, sin prioridad — para retomar después)

Barrido del catálogo de Flutter/SwiftUI/Compose/web por controles que **resaltan**
y faltan (verificado contra la lista de widgets: ninguno existe hoy). Columna clave:
*¿influye en la arquitectura?* — **No** = composición pura, se hace cuando un caller
lo pida; **Anim** = sólo extiende el `AnimRegistry`; **Seam** = ya cubierto por los
cuatro seams de arriba, no abre uno nuevo.

| Control | Análogo | ¿Influye arq.? | Nota |
|---|---|---|---|
| ~~**Charts**~~ | Swift Charts · `fl_chart` | — | **YA EXISTE: es `pineal`** (`00_unanchay/pineal`, dominio cerrado): cartesian/polar/financial/heatmap/treemap/hexbin/contour/bars/flow/stream/mesh/phosphor sobre el trait `Canvas`, agnóstico de backend, **ya pinta a vello/llimphi**. No es un gap. Único faltante posible: un `llimphi-widget` fino que embeba un canvas pineal en un `View` vía `paint_with` — **verificar si el bridge ya existe** (la migración a Llimphi trajo backend SceneCanvas + widgets) antes de anotar nada. |
| ✅ **Ripple / InkWell** | Material `InkWell` ripple | Anim | **Hecho (Bloque 8).** `View::ripple(key, color)`/`ripple_styled` + `RippleRegistry` (retenido en el runtime como `AnimRegistry`): el press dispara una salpicadura (`hit_test_ripple`, aditivo — no toca click/drag), el runtime la pinta tras el contenido como círculo expansivo (radio ease-out hasta cubrir el nodo) recortado al `node_rrect` y atenuado por el fade; ticker autodetenido. Consumidor: `button_ripple`/`button_styled_ripple`. Demo `--example ripple_demo`. Limitación v1: ignora `transform` de ancestros y no "sostiene" mientras se mantiene el press. |
| **Carousel / Pager** | Compose `HorizontalPager` · iOS page control | Seam (gestos+scroll) | Páginas full-width con snap. Cae bajo scroll-physics + gestos (Tier 4/5), no abre seam nuevo. |
| **Chips** (filter/choice/input, removibles) | `FilterChip`/`InputChip` · `AssistChip` | No | Selección múltiple compacta, tag-input. Composición sobre button+badge. |
| **Range slider** (dos thumbs) | `RangeSlider` | No | Variante del `slider` que ya existe (filtros de rango, ecualizador en media). |
| **Shimmer** (skeleton animado) | `Shimmer` · `redacted` animado | Anim | El `skeleton` existe estático; falta el barrido de gradiente. Tween de offset sobre gradiente (Tier 1 ya da gradientes). |
| **Reorderable list** (drag de ítems) | `ReorderableListView` | No | Drag&drop ya existe (`on_drop`); `tiled` reordena paneles pero no ítems de lista. Útil para playlists/kanban/prioridades. |
| **Wrap / flow layout** | `Wrap` · Compose `FlowRow` | No (verificar flex-wrap de taffy) | Ítems que saltan de línea (chips, tags, galería fluida). Probablemente sale de `flex-wrap` de taffy — verificar antes de widget nuevo. |
| **FittedBox / scale-to-fit** | `FittedBox` | Seam (LayoutBuilder) | Escala un subárbol arbitrario para caber. Pariente de autotextsize pero para cualquier `View`; depende del seam de size-aware. |
| **Calendar / agenda view** | `CalendarView` · agenda | No | Grilla de mes + vista de agenda. Scheduling de ERP; compone date-picker + grid. |
| **Rating** (estrellas) · **Gauge** (radial) | `RatingBar` · SwiftUI `Gauge` | No | Pequeños, sobre `paint_with`. Gauge útil para dashboards/cosmos/nakui. |
| **animateContentSize** | Compose `animateContentSize()` | Anim (re-layout) | Animar el alto/ancho al cambiar contenido. Ya anotado como extensión del Bloque 4 que requiere re-layout en el frame. |
| **Markdown render** | `flutter_markdown` | No (depende de RichText) | Render de markdown a `View`; espera spans inline mixtos del Tier 2. Probable territorio de pluma, no widget genérico. |

### Mirado y descartado (no encaja hoy)

- Widgets adaptativos plataforma (Cupertino vs Material): N/A, gioser tiene su
  theme semántico propio.
- `RefreshIndicator` pull-to-refresh, `Dismissible` swipe-to-action: patrones
  móviles; gioser es desktop-first (relevante sólo para `android`/`wawa`, diferir).
- `InheritedWidget`/`PreferenceKey` (contexto que baja/sube por el árbol): el
  bucle Elm pasa todo explícito a propósito; sólo haría falta si algún valor
  *derivado del layout* (tamaño medido) tuviera que burbujear hacia un ancestro.
  Anotado como tensión latente, no como deuda.
