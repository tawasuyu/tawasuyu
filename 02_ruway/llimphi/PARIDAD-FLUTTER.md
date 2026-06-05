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
   Hoy `scroll_y` no contempla "hijos que reaccionan al offset". El 80/20 real es
   (a) lista virtualizada [ya está], (b) header colapsable, (c) sticky sections;
   las tres son incrementales **si** la firma del viewport admite extent-por-offset.
2. **Arena de gestos (desambiguación)** — Tier 4. long-press/double-tap/pinch/
   rotate/fling necesitan un árbitro, no se cuelgan ad-hoc del hit-test.
3. **Árbol de semántica (AccessKit)** — Tier 7. Árbol paralelo al `View`.
4. **Build sensible al tamaño (`LayoutBuilder` / `MediaQuery` breakpoints)** —
   **no estaba en los tiers; verificado ausente 2026-06-05.** `view()` construye
   antes del layout, así que "construir distinto según el espacio disponible"
   exige un builder diferido (o un nodo que reciba sus constraints medidas).
   Habilita paneles responsive/adaptativos. Reservar la forma del builder ahora.

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
| Backdrop blur (glass) | `.backdrop_blur(sigma)` | raster | caro (samplea el framebuffer detrás); el look "moderno". Único pendiente del Tier 1. |

### 🟢 Tier 2 — texto rico (parley lo soporta, falta exponerlo)
`TextSpec` hoy expone `italic`, color por rango y **peso de fuente**.

- ✅ Peso/bold → `weight: f32` en `TextSpec` + `.text_weight(...)`/`.bold()`. Fluye por medida y pintado (camino directo a `Typesetter::layout`, no `TextBlock`).
- ✅ Overflow/ellipsis (`maxLines` + `…`) → `.ellipsis(n)`/`.max_lines(n)` + `Typesetter::layout_clamped`. Clampa medida y pintado; recorta graphemes hasta caber. Cubre single-line y N líneas.
- Spans inline mixtos (tamaño/peso/familia/link por rango, no sólo color) → `RichText` real.
- Decoración: subrayado / tachado.
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

### 🟡 Tier 4 — gestos
Hoy: tap, drag(delta), scroll. Falta el set de `GestureDetector`: long-press,
double-tap, pinch/scale (zoom), rotate, velocity/fling, y arena de
desambiguación. Pinch-zoom es lo más pedido por los canvases (pineal/cosmos/nakui).

### 🟡 Tier 5 — scroll avanzado
Sólo `scroll_y` vertical con inercia manual. Falta: scroll horizontal y 2D,
physics momentum/bounce, scroll anidado, slivers (app bars colapsables, sticky
headers), scrollbar persistente, pull-to-refresh.

### 🟠 Tier 6 — assets / media
- SVG: hoy sólo `llimphi-icons` a mano. Falta parser (existe `vello_svg`).
- Imágenes: `.image()` sólo centra preservando aspecto. Falta `fit:{cover,contain,fill}`,
  clip redondeado sobre imagen, decode pipeline, imágenes de red con caché.

### 🔴 Tier 7 — accesibilidad (la brecha categórica más grande)
**Cero hoy.** No hay árbol de semántica ni lectores de pantalla. A desarrollar:
un árbol de semántica paralelo al `View` (rol, label, estado, acciones por nodo)
+ integración **AccessKit** (estándar Rust, se enchufa a winit) que lo traduce a
UIA/AT-SPI/VoiceOver. Imprescindible para "competente" en serio; se difiere, no se
omite. Ver explicación extendida abajo.

### 🔴 Tier 8 — arquitectura de render / performance
Cada frame reconstruye todo el árbol `View` y vello rerasteriza la escena
completa. Falta: `RepaintBoundary` (sub-escenas cacheadas / capas retenidas) +
damage/dirty-region en el present. No urge (vello es rápido), pero separa "fluido
a 5k nodos" de "a 50k".

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
5. **Bloque 5 = quick wins de la cosecha** — forma de cursor (`.cursor(...)`,
   completa el hover que ya existe) + animación de contenido (cross-fade/enter-exit
   extendiendo `AnimRegistry`) + scrollbar arrastrable. Composición barata, alto
   retorno visual.
6. Pinch-zoom + scroll physics.
7. AccessKit + slivers + `LayoutBuilder` (los seams a reservar, ya con forma de API).

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
| **Forma de cursor** (`.cursor(CursorIcon)`) | `MouseRegion.cursor` / `SystemMouseCursors` · Compose `pointerHoverIcon` | **Ausente** (no hay `set_cursor`/`CursorIcon` en runtime) | Pulido desktop crítico: beam sobre input, resize sobre splitter, pointer sobre botón. El plumbing de hover (`on_pointer_enter/leave`) **ya está** — falta que el runtime aplique el cursor del nodo hovered más alto vía `window.set_cursor()`. Es un mini-contrato (prop en `View` + resolución topmost), pero chico. |
| **Animación de contenido** (cross-fade al swap + enter/exit) | `AnimatedSwitcher` · `AnimatedList` · `AnimatedVisibility` | **Parcial**: sólo animación de props (fill/radius) del Bloque 4 | Es el Bloque 5 natural: `AnimRegistry` ya keya por `key` estable, así que "apareció/desapareció una key" = enter/exit, y "cambió la identidad bajo la misma key" = cross-fade. Altísimo valor visual sobre el reconciliador que ya existe. |
| **Scrollbar interactiva** (drag del thumb) | `Scrollbar` arrastrable | Falta (Tier 5 lo lista como "persistente") | Table-stakes desktop. `thumb_geometry` ya calcula la geometría; falta el hit-test + drag del thumb. |

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

### Mirado y descartado (no encaja hoy)

- Widgets adaptativos plataforma (Cupertino vs Material): N/A, gioser tiene su
  theme semántico propio.
- `RefreshIndicator` pull-to-refresh, `Dismissible` swipe-to-action: patrones
  móviles; gioser es desktop-first (relevante sólo para `android`/`wawa`, diferir).
- `InheritedWidget`/`PreferenceKey` (contexto que baja/sube por el árbol): el
  bucle Elm pasa todo explícito a propósito; sólo haría falta si algún valor
  *derivado del layout* (tamaño medido) tuviera que burbujear hacia un ancestro.
  Anotado como tensión latente, no como deuda.
