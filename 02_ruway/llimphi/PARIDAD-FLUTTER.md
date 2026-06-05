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

## Tiers por retorno de inversión

### 🟢 Tier 1 — exponer lo que vello ya hace (alto impacto, bajo costo)
Hoy cada widget las simula a mano (badge inventa el gloss; context-menu/text-input
fingen el borde con un rect-padre inset).

| Falta | Función a desarrollar | Dónde | Notas |
|---|---|---|---|
| Sombras / elevación | `.shadow(ShadowSpec{ blur, offset, color, spread })` | compositor + raster | vello: `draw_blurred_rounded_rect` nativo. Brecha #1. |
| Gradientes como fill | `.fill_gradient(Gradient)` (linear/radial/sweep) | compositor | `peniko::Gradient` ya está. |
| Bordes reales | `.border(width, color)` / `.border_sides(...)` | compositor + raster | stroke de rounded-rect; mata el truco del rect-padre. |
| Radio por esquina | `.radius_corners(tl,tr,br,bl)` | compositor | hoy `radius` es `f64` uniforme. |
| Backdrop blur (glass) | `.backdrop_blur(sigma)` | raster | caro (samplea el framebuffer detrás); el look "moderno". |

### 🟢 Tier 2 — texto rico (parley lo soporta, falta exponerlo)
`TextSpec` hoy sólo expone `italic` + color por rango. **No hay peso de fuente.**

- Peso/bold → `weight: FontWeight` en `TextSpec` + `.text_weight(...)`. Casi gratis, altísimo impacto.
- Spans inline mixtos (tamaño/peso/familia/link por rango, no sólo color) → `RichText` real.
- Decoración: subrayado / tachado.
- Overflow/ellipsis (`maxLines` + `…`) → crítico para listas/labels. Hoy no existe.
- Texto seleccionable fuera del editor (selección + copiar).

### 🟡 Tier 3 — animación declarativa (brecha de arquitectura)
Hay `Tween` + `animate()`, pero cada animación se cablea a mano (tween en Model +
`spawn_periodic` + guard de generación).

- Animaciones **implícitas** (`AnimatedContainer`/`AnimatedOpacity`): el nodo
  interpola solo entre valor anterior y nuevo. Requiere identidad/keys de nodo en
  el compositor. La mejora de DX más grande.
- Ticker central (vsync) en vez de N `spawn_periodic`.
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

1. **Bloque 1 = Tier 1 (sombra+gradiente+borde+peso)** — builders de `View` sobre
   primitivas existentes. Máximo retorno visual; limpia deuda de widgets que las
   fingen. ← arrancamos por acá.
2. Overflow/ellipsis + animaciones implícitas.
3. Pinch-zoom + scroll physics.
4. AccessKit + slivers.

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
