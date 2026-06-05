# Manual de Llimphi

> Motor gráfico soberano de gioser. `wgpu` + `vello` + `taffy` + `parley`,
> bucle Elm `input → update → view → layout → raster → present`.
> Reemplazo total de GPUI (extinto 2026-05-26): toda app gráfica de la suite
> corre sobre Llimphi.

Este documento es la **referencia de uso** orientada a humanos y a IA.
Está organizado para salto directo: cada capa, widget y módulo trae su API
real (firmas copiadas del código). Para el **porqué** arquitectónico ver
[`SDD.md`](SDD.md); para la regla de concurrencia ver
[`COMPUTO-FUERA-DEL-HILO-UI.md`](COMPUTO-FUERA-DEL-HILO-UI.md).

---

## Índice

1. [Modelo mental en 60 segundos](#1-modelo-mental-en-60-segundos)
2. [Arquitectura — las capas](#2-arquitectura--las-capas)
3. [Quickstart — la app mínima](#3-quickstart--la-app-mínima)
4. [El trait `App` (bucle Elm)](#4-el-trait-app-bucle-elm)
5. [`Handle` — efectos y concurrencia](#5-handle--efectos-y-concurrencia)
6. [`View<Msg>` — el DSL declarativo](#6-viewmsg--el-dsl-declarativo)
7. [Layout (`taffy` / `Style`)](#7-layout-taffy--style)
8. [Eventos e interacción](#8-eventos-e-interacción)
9. [Texto](#9-texto)
10. [Canvas custom y GPU directo](#10-canvas-custom-y-gpu-directo)
11. [Theme y paletas](#11-theme-y-paletas)
12. [Capas base (hal · raster · text · motion · icons · surface)](#12-capas-base)
13. [Catálogo de widgets](#13-catálogo-de-widgets)
14. [Catálogo de módulos](#14-catálogo-de-módulos)
15. [`llimphi-workspace` — chasis tipo tmux](#15-llimphi-workspace--chasis-tipo-tmux)
16. [Reglas duras y gotchas](#16-reglas-duras-y-gotchas)
17. [Comandos y demos](#17-comandos-y-demos)
18. [Cheat-sheet](#18-cheat-sheet)
19. [Índice de crates](#19-índice-de-crates)

---

## 1. Modelo mental en 60 segundos

Llimphi es **Elm sobre la GPU**. Una app es un tipo que implementa el trait
`App` con cuatro piezas:

- `Model` — estado **inmutable** de la app.
- `Msg` — todo lo que puede pasar (`Clone + Send`).
- `update(model, msg, handle) -> model` — transición **pura** que devuelve un
  modelo nuevo.
- `view(&model) -> View<Msg>` — función **pura** que describe la pantalla como
  un árbol de `View`.

El runtime hace el bucle: un evento (click/tecla/rueda) produce un `Msg`,
`update` deriva el nuevo `Model`, `view` reconstruye el árbol, `taffy` calcula
las cajas, `vello` rasteriza, y se hace swap del frame. **No hay mutabilidad
compartida, no hay vDOM ajeno, no hay callbacks imperativos**: declarás qué se
ve y qué `Msg` emite cada nodo.

```
   evento ─▶ Msg ─▶ update(model,msg) ─▶ model' ─▶ view(model') ─▶ View<Msg>
                                                                      │
   present ◀─ raster(vello) ◀─ layout(taffy) ◀──────────────────────┘
```

Tres reglas de oro:
1. **`view` es pura** — no muta nada, sólo lee el modelo y arma el árbol.
2. **Cómputo pesado va a un worker** vía `Handle::spawn`, nunca síncrono en
   `update`/`init`/handlers (congela la ventana → "Not Responding").
3. **Widgets son visuales y stateless**; el estado vive en tu `Model`.
   **Módulos** sí encapsulan estado + comportamiento.

---

## 2. Arquitectura — las capas

```
4. llimphi-ui ........... runtime winit del bucle Elm (App, Handle, run, KeyEvent)
   └ llimphi-compositor . árbol View<Msg>, mount sobre taffy, paint, hit-test (winit-free)
3. llimphi-layout ....... motor de layout (taffy: flexbox + grid)
2. llimphi-raster ....... rasterizador vectorial (vello) + backend GPU directo
1. llimphi-text ......... shaping + fuentes (parley): bidi, ligaduras, CJK/emoji
0. llimphi-hal .......... abstracción de superficie (wgpu + winit / framebuffer)
```

El **split compositor/runtime** (2026-05-31) es importante: `llimphi-compositor`
es *winit-free* (sólo `View`, `mount`, `paint`, hit-test). `llimphi-ui` lo corre
sobre winit y **re-exporta todo el compositor**, así escribís `llimphi_ui::View`
sin enterarte del split. Esto habilita un futuro runtime sobre el framebuffer
del kernel `wawa` reusando el mismo compositor.

Auxiliares: `llimphi-theme` (paletas), `llimphi-motion` (tweens),
`llimphi-icons` (iconos vectoriales), `llimphi-surface` (texturas externas),
`llimphi-workspace` (chasis tmux), `llimphi-gallery` (showcase).

Catálogo: **~45 widgets** (visuales) + **10 módulos** (features con estado).

---

## 3. Quickstart — la app mínima

```rust
use llimphi_ui::llimphi_layout::taffy::prelude::*;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, View};

#[derive(Clone)]
enum Msg { Increment, Reset }

struct Counter;

impl App for Counter {
    type Model = u32;
    type Msg = Msg;

    fn title() -> &'static str { "llimphi · counter" }

    fn init(_: &Handle<Self::Msg>) -> Self::Model { 0 }

    fn update(model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Increment => model.saturating_add(1),
            Msg::Reset => 0,
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let boton = View::new(Style {
            size: Size { width: length(160.0), height: length(56.0) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(Color::from_rgba8(60, 200, 130, 255))
        .radius(12.0)
        .text("+1", 28.0, Color::from_rgba8(10, 30, 20, 255))
        .on_click(Msg::Increment);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0), height: percent(1.0) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            gap: Size { width: length(0.0), height: length(24.0) },
            ..Default::default()
        })
        .fill(Color::from_rgba8(20, 24, 32, 255))
        .children(vec![
            View::new(Style::default()).text(model.to_string(), 160.0, Color::WHITE),
            boton,
        ])
    }
}

fn main() { llimphi_ui::run::<Counter>(); }
```

`Cargo.toml`:
```toml
[dependencies]
llimphi-ui    = { workspace = true }
llimphi-theme = { workspace = true }
# + los widgets/modules que uses:
# llimphi-widget-button = { workspace = true }
```

Corre con `cargo run -p <tu-crate> --release`. El ejemplo vivo está en
`llimphi-ui/examples/counter.rs`.

---

## 4. El trait `App` (bucle Elm)

Definido en `llimphi-ui/src/lib.rs`. El estado es inmutable; cada evento
produce un `Model` nuevo.

```rust
pub trait App: 'static {
    type Model: 'static;
    type Msg: Clone + Send + 'static;

    fn init(handle: &Handle<Self::Msg>) -> Self::Model;
    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model;
    fn view(model: &Self::Model) -> View<Self::Msg>;

    // --- Todo lo de abajo tiene default; sobreescribí lo que necesites ---

    fn on_key(_model: &Self::Model, _event: &KeyEvent) -> Option<Self::Msg> { None }

    fn on_wheel(_model: &Self::Model, _delta: WheelDelta,
                _cursor: (f32, f32), _modifiers: Modifiers) -> Option<Self::Msg> { None }

    /// Capa de overlay (menús, modales, popovers). Si devuelve `Some`, se pinta
    /// encima y clicks/hover van EXCLUSIVAMENTE a ella (el fondo queda "bajo
    /// vidrio"). La transición la maneja tu Model.
    fn view_overlay(_model: &Self::Model) -> Option<View<Self::Msg>> { None }

    /// Drag&drop de archivos desde el file manager. Un evento por archivo.
    fn on_file_drop(_model: &Self::Model, _path: std::path::PathBuf) -> Option<Self::Msg> { None }

    /// El foco cambió (Tab/Shift+Tab o click sobre un nodo `focusable`). El
    /// runtime administra el foco; guardás `id` en tu Model para pintar el ring
    /// y rutear el teclado. Ver §8 (Foco y teclado).
    fn on_focus(_model: &Self::Model, _id: Option<u64>) -> Option<Self::Msg> { None }

    /// IME (composición de texto: CJK, acentos muertos, emoji). Opt-in vía
    /// `ime_allowed()` para no robarle el texto a las apps que sólo leen
    /// `on_key`. Flujo: Enabled → Preedit* → Commit/Disabled. Ver §8 (IME).
    fn ime_allowed() -> bool { false }
    fn on_ime(_model: &Self::Model, _event: &ImeEvent) -> Option<Self::Msg> { None }
    /// Área del caret en px físicos para ubicar la ventana de candidatos.
    fn ime_cursor_area(_model: &Self::Model) -> Option<(f32, f32, f32, f32)> { None }

    fn title() -> &'static str { "llimphi" }
    fn app_id() -> Option<&'static str> { None }   // app_id del xdg-toplevel en Wayland
    fn initial_size() -> (u32, u32) { (960, 540) }
}
```

Punto de entrada: `pub fn run<A: App>()` — corre hasta que el usuario cierre la
ventana o la app llame `Handle::quit`.

**Eventos de teclado** (`KeyEvent`):
```rust
pub struct KeyEvent {
    pub key: Key,                 // re-export de winit; usar NamedKey para teclas especiales
    pub state: KeyState,          // Pressed | Released
    pub text: Option<String>,     // texto resultante con IME/modifiers; None para flechas etc.
    pub modifiers: Modifiers,     // { shift, ctrl, alt, meta }
    pub repeat: bool,
}
```
`Key` y `NamedKey` se re-exportan desde `llimphi_ui`.

**Rueda** (`WheelDelta { x, y }`): normalizado a "líneas". Convención CSS:
`y` positivo = scroll hacia abajo.

---

## 5. `Handle` — efectos y concurrencia

`Handle<Msg>` es `Send + Clone`. Llega a `init` y `update`. Es el único modo
legítimo de producir efectos sin romper la pureza de la transición.

```rust
impl<Msg: Send + 'static> Handle<Msg> {
    pub fn quit(&self);                 // cierra la ventana / termina el bucle
    pub fn dispatch(&self, msg: Msg);   // encola un Msg para el próximo turno
    pub fn spawn<F: FnOnce() -> Msg + Send + 'static>(&self, f: F);   // worker; su Msg reentra al update
    pub fn spawn_periodic<F: Fn() -> Msg + Send + 'static>(&self, period: Duration, f: F);  // tick periódico
    pub fn for_test() -> Self;          // handle "muerto" para tests sin event loop
}
```

- **`spawn`** — trabajo bloqueante (IO, PAM, parse, efemérides). El `Msg` que
  devuelve la closure se entrega al `update` en el hilo de UI. **Este es el
  patrón obligatorio para todo cómputo pesado** (§16).
- **`spawn_periodic`** — feeds a intervalos: ticks de simulación (~11 Hz en
  dominium), polling, animaciones por reloj. El thread muere cuando se cierra
  el event loop.

---

## 6. `View<Msg>` — el DSL declarativo

Un `View` = `Style` de taffy + relleno + texto/imagen/painter + handlers +
hijos. Todo se arma con builders encadenables (`self -> Self`). Definido en
`llimphi-compositor/src/view.rs`.

```rust
View::new(style: Style) -> View<Msg>
```

### Apariencia
| Método | Efecto |
|---|---|
| `.fill(Color)` | color de fondo |
| `.fill_gradient(Gradient)` | relleno con gradiente (autoreado en el cuadrado unidad `[0,1]²`, mapeado al rect). Gana sobre `fill`; `hover_fill` lo overridea en hover |
| `.hover_fill(Color)` | color al pasar el cursor (habilita hit-test de hover) |
| `.radius(f64)` | esquinas redondeadas (radio uniforme) |
| `.radius_corners(tl,tr,br,bl)` | radio **por esquina** (CSS `border-radius` 4 valores); override de `.radius`. El borde sigue las 4 esquinas; la sombra usa el radio escalar |
| `.shadow(Shadow)` | drop shadow (vello `draw_blurred_rounded_rect`). `Shadow::soft(alpha,blur)` + `.offset(dx,dy)`/`.spread(s)` |
| `.border(width, Color)` | stroke sobre el contorno redondeado, inset hacia adentro (border-box) |
| `.alpha(f32)` | opacidad de todo el subtree `[0,1]` (capa intermedia — no gratis) |
| `.transform(Affine)` | afín 2D alrededor del centro del rect (estilo CSS `transform-origin:50% 50%`) |
| `.animated(key, Duration)` | animación **implícita** estilo Flutter `AnimatedContainer`: si `fill`/`radius` cambian entre frames, el runtime interpola (ease-out cúbico) en vez de saltar. `key` estable entre rebuilds. `.animated_curve(key,dur,fn)` para otra curva |
| `.clip(bool)` | recorta hijos al rect (paint + hit-test) |
| `.image(Image)` | pinta `peniko::Image` centrada, preservando aspect ratio |
| `.children(Vec<View<Msg>>)` | hijos |

### Texto (ver §9)
```rust
.text(content, size_px, color)                            // centrado
.text_aligned(content, size_px, color, Alignment)
.text_aligned_italic(content, size_px, color, Alignment, italic)
.text_aligned_full(content, size_px, color, Alignment, italic, font_family: Option<String>)
.text_runs(content, size_px, default_color, runs: Vec<(usize,usize,Color)>, Alignment) // multicolor 1-pasada
.line_height(mult)                                        // override interlínea (default 1.2)
.text_weight(f32)                                         // peso CSS: 400 normal, 600 semibold, 700 bold
.bold()                                                   // atajo de text_weight(700.0)
.ellipsis(n)                                              // clampa a n líneas terminando en … (n=1 = single-line)
.max_lines(n)                                             // clampa a n líneas sin glifo (corte seco)
```

### Interacción (ver §8)
```rust
.on_click(Msg)
.on_click_at(|lx, ly, w, h| -> Option<Msg>)     // posición local + tamaño del rect
.on_right_click(Msg) / .on_right_click_at(...)
.on_middle_click(Msg)
.on_pointer_enter(Msg) / .on_pointer_leave(Msg)
.draggable(|phase: DragPhase, dx, dy| -> Option<Msg>)
.draggable_at(|phase, dx, dy, lx0, ly0| -> Option<Msg>)   // + posición inicial del press
.drag_payload(u64)                                        // payload que viaja con el drag
.on_drop(|payload: u64| -> Option<Msg>)                   // este nodo es drop target
.drop_hover_fill(Color)                                   // resaltado mientras un drag lo sobrevuela
.on_scroll(|dx, dy| -> Option<Msg>)                       // rueda local (antes del on_wheel global)
.on_scale(|phase: GesturePhase, factor, fx, fy| -> Option<Msg>)  // pinch-to-zoom (Ctrl+rueda / trackpad)
.on_double_tap(Msg) / .on_double_tap_at(|lx, ly, w, h| ...)      // dos clicks rápidos y cercanos
.on_long_press(Msg) / .on_long_press_at(|lx, ly, w, h| ...)      // mantener ~500 ms quieto
.focusable(u64)                                           // nodo enfocable por Tab/click (id opaco)
```

### Pintura custom (ver §10)
```rust
.paint_with(|scene: &mut vello::Scene, ts: &mut Typesetter, rect: PaintRect| { ... })
.gpu_paint_with(|device, queue, encoder, view, rect: PaintRect, (vp_w, vp_h)| { ... })
```

Notas clave:
- **Un nodo es draggable *o* clickable**, no ambos: `draggable` sobreescribe
  `on_click`.
- Las variantes `*_at` ganan sobre las simples si ambas están.
- `PaintRect { x, y, w, h }` es el rect **absoluto** del nodo en píxeles físicos.
- `DragPhase` = `Move` (un evento por `CursorMoved`, `dx/dy` = delta **desde el
  evento anterior**, no acumulado) | `End` (al soltar).
- **Gestos (`on_scale`/`on_double_tap`/`on_long_press`)** son **aditivos**: se
  resuelven con su propio hit-test y no interfieren con `on_click`/`draggable`
  del mismo nodo. El caso limpio (sin disparos cruzados) es ponerlos en un nodo
  que **no** tenga `on_click` — p. ej. un canvas con `draggable` (pan) +
  `on_scale` (zoom) + `on_long_press` (marca). `GesturePhase` = `Begin`/`Update`/
  `End`; en `on_scale`, `factor` es **multiplicativo incremental** (`>1` agranda)
  y `(fx, fy)` el focal local — `Ctrl+rueda` lo sintetiza en cualquier desktop
  (Wayland/Windows no emiten el pinch del trackpad; macOS sí, vía `PinchGesture`).
  El long-press lo arbitra el tiempo (~500 ms quieto); moverse (>8px) o soltar lo
  cancela. Demo completo: `cargo run -p llimphi-ui --example gestos --release`.

---

## 7. Layout (`taffy` / `Style`)

`Style` es el `taffy::Style` directo, re-exportado vía
`llimphi_ui::llimphi_layout::taffy::prelude::*`. Es Flexbox + CSS Grid puro.

Campos más usados:
```rust
Style {
    flex_direction: FlexDirection::Row | Column,
    size:    Size { width, height },          // length(px) | percent(0..1) | Dimension::auto()
    min_size, max_size,
    flex_grow: f32, flex_shrink: f32,
    align_items:     Some(AlignItems::{Start,Center,End,Stretch}),
    justify_content: Some(JustifyContent::{Start,Center,End,SpaceBetween,...}),
    gap:     Size { width, height },
    padding: Rect { left, right, top, bottom },   // con length(px)
    margin:  Rect { ... },
    ..Default::default()
}
```

Helpers de `prelude`: `length(px)`, `percent(frac)`, `auto()`, `Dimension`,
`Size`, `Rect`, `FlexDirection`, `AlignItems`, `JustifyContent`.

`llimphi-layout` además expone:
- `LayoutTree::new()` / `.clear()` (reuso entre frames), `.leaf(style)`,
  `.node(style, &children)`, `.compute(...)`, `.compute_with_measure(F)`.
- `Rect { x, y, w, h }` y `ComputedLayout { rects: HashMap<NodeId, Rect> }`.

En el 99% de los casos no tocás `LayoutTree` a mano: lo maneja el runtime al
montar tu `View`. Sólo armás `Style`s.

---

## 8. Eventos e interacción

| Quiero… | Cómo |
|---|---|
| Botón / fila clickable | `.on_click(Msg)` (+ `.hover_fill` para feedback) |
| Saber dónde se clickeó (canvas) | `.on_click_at(\|lx,ly,w,h\| ...)` → convertir a coords de mundo |
| Menú contextual | `.on_right_click(Msg::OpenMenu{..})`, guardar pos en Model, abrir en `view_overlay` |
| Abrir en pestaña nueva | `.on_middle_click(Msg)` |
| Preview al pasar el mouse | `.on_pointer_enter(Msg)` / `.on_pointer_leave(Msg)` |
| Resize de panel | `.draggable(\|phase,dx,dy\| ...)` acumulando delta en el Model |
| Arrastrar entidad de un canvas | `.draggable_at(\|phase,dx,dy,lx0,ly0\| ...)` |
| Drag&drop entre zonas | origen: `.drag_payload(id)`; destino: `.on_drop(\|id\| ...)` + `.drop_hover_fill` |
| Scroll global | `App::on_wheel(model, delta, cursor, mods)` |
| Área de scroll | widget `scroll_y(...)` (autocontenido) o `.on_scroll(\|dx,dy\| ...)` por nodo |
| Zoom de canvas (pinch) | `.on_scale(\|phase,factor,fx,fy\| ...)` → `zoom *= factor`, reajustar pan al focal |
| Doble-click | `.on_double_tap(Msg)` / `.on_double_tap_at(\|lx,ly,w,h\| ...)` |
| Long-press (mantener) | `.on_long_press(Msg)` / `.on_long_press_at(\|lx,ly,w,h\| ...)` |
| Teclado | `App::on_key(model, &KeyEvent) -> Option<Msg>` |
| Foco / Tab | `.focusable(id)` en los nodos + `App::on_focus(model, id)` (ver abajo) |
| IME (CJK, acentos) | `App::ime_allowed() -> true` + `App::on_ime(model, &ImeEvent)` (ver abajo) |
| Drop de archivos del SO | `App::on_file_drop(model, path)` |

**Patrón overlay** (menús/modales): el modelo guarda "menú abierto sí/no".
Mientras esté abierto, `view_overlay` devuelve `Some(view)`; clicks fuera se
cierran envolviendo los items en un scrim a pantalla completa con
`on_click = DismissOverlay`. Cuando el modelo dice cerrado, `view_overlay`
devuelve `None`.

**Scroll** (widget `llimphi-widget-scroll`). `scroll_y(offset, content_len,
viewport_len, content, on_scroll, &palette)` arma un viewport clipeado +
contenido desplazado `-offset` + barra arrastrable. Es **stateless**: el offset
vive en tu Model. `on_scroll(delta_px)` (rueda y arrastre) emite un delta a
sumar; clampealo con `scroll::clamp_offset` en tu `update`. Helpers:
`ensure_visible(offset, vp, item_top, item_h)` para llevar la selección a la
vista (teclado); `approach(cur, target, factor)` para scroll suave hacia un
objetivo (driveado por `Handle::spawn_periodic`).

**Scroll 2D / física / slivers** (Tier 5, mismo widget, todo stateless):
- `scroll_xy(offset:(x,y), content_size:(w,h), viewport_size:(w,h), content,
  on_scroll:(dx,dy)→Msg, &palette)` — dos ejes, una barra por eje con overflow.
- **Inercia (fling)**: `fling_step(velocity, dt, friction) → (v', delta)` +
  `fling_settled(v)` — soltá con velocidad y avanzá el offset por frame con el
  ticker (`spawn_periodic`); `FLING_FRICTION`/`FLING_STOP` defaults. **Bounce**:
  `rubber_band(overscroll, dim)` amortigua el desplazamiento más allá del tope.
- **Sliver app-bar colapsable**: `sliver_app_bar(offset, header_max, header_min,
  header(frac)→View, content, content_len, viewport_len, on_scroll, &palette)` —
  un único offset colapsa el header (de max a min) y luego scrollea el cuerpo
  bajo él; `header(frac)` recibe `frac∈[0,1]` para fundir título/subtítulo.
  Clampeá con `sliver_max_offset(...)`. Helpers puros: `collapsed_height`,
  `collapse_fraction`, `sticky_y(offset, section_top, section_h, header_h)` para
  encabezados de sección pegados al tope. Demo: `cargo run -p
  llimphi-widget-scroll --example scroll_avanzado`.

**Foco y teclado.** Marcá los nodos navegables con `.focusable(id)` (id `u64`
que vos elegís). El runtime es la **única fuente de verdad** del foco: lo mueve
con Tab/Shift+Tab en orden de árbol (envolviendo) y al clickear un nodo
enfocable, y te avisa con `App::on_focus(model, Option<u64>)`. Guardás el id en
tu Model para (a) pintar el ring (`if model.focus == Some(id) { .fill(accent) }`
en `view`) y (b) rutear el teclado al campo activo desde `on_key`. No setees el
foco por tu cuenta vía Msg: quedaría desincronizado del runtime.

**IME** (composición de texto). Opt-in: `ime_allowed() -> true`. Con IME activo
el texto compuesto **no** llega por `KeyEvent.text` sino por `on_ime`:
`ImeEvent::Enabled` → uno o más `Preedit{text, cursor}` (texto en composición, a
pintar subrayado en el caret) → `Commit(text)` (insertá como tecleado) o
`Disabled`. Reportá el área del caret con `ime_cursor_area(model)` para ubicar
la ventana de candidatos (CJK) junto al cursor.

---

## 9. Texto

`TextSpec` (en compositor) describe el texto de un nodo:
```rust
pub struct TextSpec {
    pub content: String,
    pub size_px: f32,
    pub color: Color,
    pub alignment: Alignment,          // Start | Center | End | Justify
    pub italic: bool,
    pub font_family: Option<String>,   // string CSS con fallbacks
    pub line_height: f32,              // múltiplo; default 1.2
    pub runs: Option<Vec<(usize, usize, Color)>>,  // color por rango de BYTES
}
```

- `Center` es el default (apto para labels). Para editores/párrafos usar
  `.text_aligned(..., Alignment::Start)`.
- **Multicolor en una sola pasada de shaping**: `.text_runs(...)` colorea
  rangos de bytes — es la base del syntax highlighting (un nodo por línea, no
  por token). Anclado arriba-izquierda; el caller dimensiona el rect.
- El runtime mide el texto con parley durante el layout (`compute_with_measure`)
  para que taffy reserve el alto real del texto envuelto a varias líneas
  (evita "textos aplastados").
- Shaping completo: bidi, ligaduras, kerning, fallback CJK/emoji vía fontique.

---

## 10. Canvas custom y GPU directo

Dos hooks para pintar primitivas no expresables como composición de `View`s.
Conviven en el mismo árbol; el runtime pinta **toda la pasada vello primero**,
luego los `gpu_painter` en orden DFS.

### `paint_with` — vía vello (el default)
```rust
.paint_with(|scene: &mut vello::Scene, ts: &mut Typesetter, rect: PaintRect| {
    // dibujar BezPath, kurbo, texto con `ts`, etc. dentro de `rect`.
    // NO dejar push_layer sin pop_layer; NO resetear la scene.
})
```
Para: dominium-canvas, osciloscopios de pluma, charts de cosmos, pineal.
Bueno hasta ~500 K primitivos por frame (rebuild) o ~2 M (Scene reusada).

### `gpu_paint_with` — sube vertex buffers directo a wgpu, salta vello
```rust
.gpu_paint_with(|device, queue, encoder, view, rect: PaintRect, (vp_w, vp_h)| {
    // abrir begin_render_pass con LoadOp::Load (NO clear) para preservar vello.
    // (vp_w, vp_h) = tamaño en px de la TextureView destino, para calcular NDC.
})
```
Para volumen masivo: starfield Gaia de cosmos, particles de tinkuy, viewport de
nakui, pineal denso. Rango 100 K – 10 M+ primitivos. **No** soporta texto ni AA
fino ni múltiples grosores de stroke por flush. Para texto encima de un render
GPU, usar `view_overlay` (segunda Scene vello).

### ¿Cuándo cada uno?
| Pregunta | vello (`paint_with`) | GPU directo (`gpu_paint_with`) |
|---|---|---|
| Primitivos/frame | < ~500 K rebuild / < ~2 M Scene reusada | 100 K – 10 M+ |
| ¿Cambian cada frame? | sí, rebuild barato | mejor estático (buffer persistente) |
| Curvas Bezier | nativas | hay que teselar |
| Texto | sí | no |
| AA fino | sí (analítico) | no (sin MSAA) |

**Default: `paint_with`** salvo que ya midas que el volumen lo justifica
(factores ~11× a 1M en GPU mid sólo en el régimen persistente). El backend GPU
expone `GpuPipelines`/`GpuBatch` en `llimphi-raster` (§12).

---

## 11. Theme y paletas

`llimphi-theme::Theme` es un struct de slots semánticos de color. Cuatro presets
`const`: `Theme::dark()` (default), `light()`, `aurora()`, `sunset()`.

```rust
pub struct Theme {
    pub name: &'static str,
    // fondos
    pub bg_app, bg_panel, bg_panel_alt, bg_input, bg_input_focus,
    pub bg_button, bg_button_hover, bg_selected, bg_row_hover: Color,
    // texto
    pub fg_text, fg_muted, fg_placeholder, fg_destructive: Color,
    // bordes y acento
    pub border, border_focus, accent: Color,
}

Theme::all() -> Vec<Theme>                 // orden de rotación canónico
Theme::by_name(name) -> Option<Theme>
Theme::next_after(current_name) -> Theme   // para el theme-switcher
```

Tokens auxiliares en el mismo crate:
- `motion::{FAST=80ms, NORMAL=160ms, SLOW=320ms}` + `ease_out_cubic`,
  `ease_in_out_cubic`, `linear`.
- `alpha::{SCRIM, GLASS_PANEL, DISABLED, HINT}` (constantes `u8`).
- `radius::{XS=2, SM=4, MD=8, LG=12, XL=20}` (`f64`).

**Patrón de widgets**: cada widget define su `XxxPalette` con
`Palette::from_theme(&theme)`. Tu app guarda un `Theme` en el Model, deriva las
paletas que necesita en `view`, y se las pasa a los widgets. Para cambiar de
tema, el `theme-switcher` emite `Msg(next_theme)` y reconstruís todo.

---

## 12. Capas base

### `llimphi-hal` — superficie
```rust
Hal::new(compatible_surface: Option<&wgpu::Surface>) -> Result<Hal, HalError>   // async
trait Surface { fn size(); fn resize(w,h); fn acquire() -> Result<Frame,_>; fn present(frame, hal); }
WinitSurface::new(hal, window: Arc<Window>) -> Result<Self, HalError>
Frame::view() -> &wgpu::TextureView;  Frame::size() -> (u32,u32)
```
`Hal::new` pide adapter `Backends::PRIMARY` (Vulkan) y cae a `all()` sólo si no
hay — **no volver a `InstanceDescriptor::default()`**: el backend GL de Mesa
sobre Wayland segfaultea en el teardown. El runtime de `llimphi-ui` ya maneja
todo esto; sólo tocás HAL si escribís un runtime nuevo.

### `llimphi-raster` — rasterización
```rust
Renderer::new(hal) -> Result<Renderer,_>
Renderer::render(&mut self, hal, scene: &vello::Scene, frame: &Frame, base_color: Color)
// GPU directo:
GpuPipelines::new(device, color_format) -> Self   // campos: lines, tris, rects, bind_layout
GpuBatch::new(&pipelines)
  .line_width(w) .add_line(p0,p1,color) .add_polyline(&pts,color)
  .add_tri(a,b,c, ca,cb,cc) .add_tri_list(&verts,color) .add_rect(x,y,w,h,color)
  .primitive_count() -> u32
  .flush(device, queue, encoder, view, viewport, load_op)
```
Re-exporta `vello` y `peniko` (`Color`, `Image`, `Fill`, etc.).

### `llimphi-text` — shaping
```rust
Typesetter::new()                          // una por proceso (FontContext es caro)
  .layout(text, size_px, max_width, alignment, line_height, italic, font_family) -> Layout<()>
  .layout_runs(text, size_px, default_color, &runs, alignment, line_height) -> Layout<RunBrush>
TextBlock::simple(text, size_px, color, origin)
layout_block(ts, &block) / measure(ts, &block) -> Measurement
draw_layout(scene, &layout, color, origin) / draw_layout_runs(scene, &layout, origin)
Alignment::{Start, Center, End, Justify}
```

### `llimphi-motion` — tweens
```rust
trait Lerp { fn lerp(self, other, t: f32) -> Self; }   // impl para f32,f64,(f32,f32),(f64,f64),Color
Tween::new(from, to, duration, easing: fn(f32)->f32)   // o Tween::idle(value)
tween.value() / .progress() / .done()
animate(handle, duration, make_msg)                    // arranca los ticks del tween
```
Patrón: guardás `Tween<T>` en el Model, `animate(...)` en el update, la `view`
lee `tween.value()` cada repaint. El tween se auto-termina.

### `llimphi-icons` — iconos vectoriales (~23, grid 24×24)
```rust
Icon::{File, Folder, Save, Plus, Minus, X, Check, Edit, Trash, ChevronUp/Down/Left/Right,
       Home, Search, Info, Warning, Error, Bell, Settings, More, ...}
icon_view(Icon, color, stroke_width) -> View<Msg>
paint_icon(scene, rect, icon, color, stroke_width)     // dentro de un paint_with
```
`stroke_width` en unidades del grid 24×24 (1.6 es armónico).

### `llimphi-surface` — texturas externas
```rust
ExternalSurface::new(device, queue)        // barato de clonar (Arc<Mutex> interno)
  .upload(&rgba, w, h)                      // desde otro hilo/decoder/cámara
  .view(style) -> View<Msg>                 // blittea a su rect en el árbol Elm
  .blit(queue, encoder, dst_view, rect, viewport)   // o manual desde gpu_paint_with
```

---

## 13. Catálogo de widgets

Los widgets son **funciones puras** que devuelven `View<Msg>` (o specs que se
convierten a `View`). Son **stateless**: el estado vive en tu Model. Convención:
cada uno trae `XxxPalette::from_theme(&Theme)`. Crates en
`widgets/<nombre>/`, dep `llimphi-widget-<nombre>`.

### Controles

**button** — `button_view(label, &ButtonPalette, on_click: Msg) -> View`;
`button_styled(label, style, alignment, &palette, on_click)`.

**field** — wrapper de formulario (label + helper/error + requerido).
`field_view(FieldSpec { label, control: View<Msg>, required, helper, error, palette })`.

**text-input** — input single-line **con estado** `TextInputState`
(`new()`/`masked()`, `text()`, `set_text()`, `apply_key(&KeyEvent) -> bool`,
soporta undo/redo + selección con Shift). Render:
`text_input_view(&state, placeholder, focused, &palette, on_focus: Msg)`.

**text-area** — multilínea con estado `TextAreaState` (Enter = newline, sin
auto-submit). `text_area_view(&state, placeholder, focused, body_height, &palette, on_focus)`.

**slider** — sin estado. `slider_view(label, value, min, max, &palette,
on_change: Fn(DragPhase, delta_value) -> Option<Msg>)`. El delta viene en
unidades, no píxeles.

**switch** — `switch_view(progress: f32 [0..1], on_toggle: Msg, &palette)`. La
app guarda el `bool` y opcionalmente anima `progress` con un `Tween`.

**segmented** — N opciones exclusivas. `segmented_view(&[&str], selected: usize,
make_msg: Fn(usize)->Msg, &palette)`.

**progress** — `linear_progress_view(progress, track, fill, height)` y
`radial_progress_view(progress, track, fill, stroke_ratio)`. Sin eventos.

**spinner** — `spinner_view(color, stroke_ratio, speed_rev_per_sec)`. Animado por
reloj absoluto; requiere redraws periódicos (`spawn_periodic`).

**badge** — `count_badge_view(count, BadgeKind)` ("99+" si ≥100) y
`dot_badge_view(BadgeKind)`. `BadgeKind::{Info,Success,Warning,Error,Neutral}`.

**avatar** — `avatar_view(name, size_px)`: círculo determinista (color por hash
del nombre + inicial).

**tooltip** — render puro. `tooltip_view(TooltipSpec { anchor, viewport, side:
Side, text, palette })`. Se monta en `view_overlay`; la app controla visibilidad
con `on_pointer_enter/leave`.

**empty** — empty-state. `empty_view(Icon, title, description: Option<&str>, &palette)`.

**skeleton** — placeholder con shimmer. `skeleton_view`, `skeleton_box_view(w,h,..)`,
`skeleton_line_view(w,..)`. Requiere redraws periódicos.

**banner** — tira de status. `banner_view(BannerKind::{Info,Success,Warning,Error}, message)`.

### Contenedores y layout

**panel** — chrome (gradiente + hairline accent). `panel_view(children, PanelStyle)`;
`PanelStyle::{from_theme, from_theme_large, neutral}`. `panel_signature_painter(style)`
para reusar el look en un `paint_with`.

**card** — `card_view(children, CardOptions { accent, padding, gap, radius, signature }, &CardPalette)`.

**stat-card** — métrica de dashboard. `stat_card_view(label, value, description,
accent, &recent_items, &palette)`.

**tabs** — `tabs_view(TabsSpec { labels, active: usize, on_select: Fn(usize)->Msg,
content: View<Msg>, tab_height, palette, tab_width })`. Selección la maneja la app.

**splitter** — divisor draggable de 2 panes. `splitter_two(Direction::{Row,Column},
a, a_size, b, b_size, on_resize: Fn(DragPhase, delta)->Option<Msg>, &palette)`.
`PaneSize::{Fixed(px), Flex}`. La app acumula el delta en su Model.

**scroll** — área de scroll vertical con barra arrastrable. `scroll_y(offset,
content_len, viewport_len, content, on_scroll: Fn(delta_px)->Msg, &palette)`.
Stateless (offset en el Model); rueda autocontenida. Helpers: `clamp_offset`,
`ensure_visible` (selección a la vista), `approach` (scroll suave). Ver §8.

**tiled** — grilla auto cols×rows de tiles con title bar. `tiled_view(tiles, &palette)`,
`tiled_view_cols(tiles, cols, &palette)`, y variantes `*_reorderable*` con
`on_reorder: Fn(from, to)->Option<Msg>` (drag-to-swap por la title bar). `TileSpec { label, content }`.

**panes** — árbol binario BSP tipo tmux. La app guarda un `Layout`:
```rust
Layout::single(id) / Layout::Split { axis: Axis, ratio, first, second }
layout.split(target, new, axis) / .without(target) / .resize(&path, delta) / .leaves()
panes_view(&layout, focused: PaneId, leaf: FnMut(PaneId)->View, on_resize: Fn(Vec<Side>,DragPhase,delta)->Option<Msg>,
           on_focus: Fn(PaneId)->Msg, &palette)
```

**grid** — grilla 2D virtualizada. `ventana_visible(total, vp_w, vp_h, scroll_fila,
&metrics) -> VisibleWindow` para virtualizar, luego `grid_view(GridSpec { cells:
Vec<GridCell { content, label, selected, on_click }>, cols, metrics, caption, ... })`.

**list** — lista vertical virtualizada. `list_view(ListSpec { rows: Vec<ListRow {
label, selected, on_click }>, total, caption, truncated_hint, row_height, palette })`.
La app prefiltra las filas visibles.

**tree** — árbol expand/collapse. `tree_view(TreeSpec { rows: Vec<TreeRow { label,
depth, has_children, expanded, selected, on_toggle, on_select }>, row_height,
indent_px, palette })`. La app aplana el árbol según nodos expandidos.

**navigator** — navegador data-agnóstico de nodos en dos modos conmutables
(**árbol** ↔ **grafo**, reusa tree + nodegraph). Render-only: la app guarda
`expanded`/`selected`/`mode`. Pasa un bosque de `NavNode { id: u64, label,
kind: NavKind (Monad|Group|Dir|File|Other), children }` y callbacks por id.
```rust
navigator_view(NavSpec { roots, mode: NavMode::{Tree,Graph}, selected, palette, guides },
    is_expanded: Fn(u64)->bool, on_toggle: Fn(u64)->Msg,
    on_select: Fn(u64)->Msg, on_context: Option<Fn(u64)->Msg>)
// árbol: click selecciona, chevron expande, icono por kind. grafo: cables de
// contención padre→hijo, arrastrar selecciona, right-click abre. Pensado para
// el sidebar de Mónadas/archivos de pata, pero no sabe de nouser.
```

**app-header** — `app_header(label, actions: Vec<View<Msg>>, &palette)`.

**status-bar** — `status_bar_view(left, center, right, &palette)` con
`StatusSegment::text(..).with_icon(Icon).clickable(Msg).emphasized()`.

**breadcrumb** — `breadcrumb_view(&[&str], make_msg: Fn(usize)->Msg, &palette)`
(el último segmento no es clickable).

**modal** — diálogo centrado con scrim. `modal_view(ModalSpec { title, body:
View<Msg>, buttons: Vec<ModalButton>, size, viewport, on_dismiss, palette })`.
`ModalButton::{primary, cancel, destructive}(label, msg)`. Se monta en `view_overlay`.

**toast** — notificaciones efímeras bottom-right. La app guarda `Vec<Toast>`
(`Toast::{info,success,warning,error}(id, text, duration)`), filtra
`is_alive(now)`, y `toast_stack_view(&toasts, viewport, make_dismiss: Fn(u64)->Msg)`.

**splash** — splash de arranque (cuatro cuadrantes andinos). `splash_view(started_at:
Instant, bg, fg_text)`; basado en tiempo, requiere redraws.

### Ricos / interactivos

**nodegraph** — lienzo de nodos + cables Bezier. Sin estado (la app guarda
posiciones y `Wire`s).
```rust
NodeSpec { id: NodeId(u32), label, x, y, inputs: Vec<String>, outputs: Vec<String> }
Wire { from_node, from_output: PinIdx(u16), to_node, to_input }
nodegraph_view(&nodes, &wires, &palette, &metrics,
    on_drag_node: Fn(NodeId, DragPhase, dx, dy)->Option<Msg>,
    on_connect:   Fn(NodeId, PinIdx, NodeId, PinIdx)->Option<Msg>)
// + nodegraph_view_ex (right-click) y nodegraph_view_styled (tints por nodo/cable)
```

**timeline** — scrub clickeable. `timeline_view(progress: f32, &palette,
on_seek: Fn(f32 [0..1])->Option<Msg>)`.

**text-editor** — editor IDE (capa visual sobre el core agnóstico). La app guarda
`EditorState`:
```rust
EditorState::new(); .text(); .set_text(s); .has_selection(); .can_undo()/.can_redo();
.add_cursor_at(line,col);  .apply_key_with_clipboard(&KeyEvent, &mut dyn Clipboard) -> ApplyResult;
.ensure_caret_visible(visible_lines)
// nota: `metrics` se pasa POR VALOR; el callback es on_pointer: Fn(PointerEvent)->Option<Msg>
text_editor_view(&state, &EditorPalette, metrics: EditorMetrics, visible_lines: usize, on_pointer)
text_editor_view_highlighted(&state, &palette, metrics, visible_lines, language: Language, on_pointer)
text_editor_view_full(&state, &palette, metrics, visible_lines, language, match_ranges: &[(usize,usize)], on_pointer)
syntax_palette_dark(&theme) -> SyntaxPalette   // en lib.rs del widget
```

**text-editor-core** — núcleo **agnóstico** (sin GPU, sin Llimphi; sólo
`peniko::Color`). Reutilizable en TUI/web/headless. Tipos clave:
- `Buffer` (sobre `ropey`): `from_str`, `text`, `insert(offset,s)`, `delete(s,e)`,
  `offset_to_pos`, `pos_to_offset`, `slice`, `line(n)`.
- `Pos { line, col }`, `Selection { anchor, caret }`, `Cursor { caret, anchor:
  Option, desired_col }` con `move_left/right/up/down/word_left/...`,
  `selection_range(&buf)`, `collapse`.
- Ops: `replace_selection`, `delete_backward/forward`, `indent_or_insert_tab`,
  `insert_newline_auto_indent` → devuelven `EditDelta { start, removed, inserted,
  cursor_before, cursor_after }` con `.apply()/.undo()`.
- `UndoStack`: `push(delta)`, `undo/redo(&mut buf, &mut cursor) -> bool`, `can_undo/redo`.
- `FindState { query, case_sensitive }`: `all_matches`, `find_next`, `find_prev`.
- Matching de brackets: `find_bracket_pair(&buf, &cursor) -> Option<(Pos, Pos)>`, `Direction`.
- `Clipboard` (trait `get/set`), `MemClipboard`, `NullClipboard`.
- `Diagnostic { range: DiagnosticRange { start: Pos, end: Pos }, severity: Severity,
  message: String, source: Option<String> }` (+ ctors `error(..)`, `warning(..)`);
  `Severity::{Error, Warning, Information, Hint}`.
- Highlight tree-sitter: `Language::{Plain, Rust, Python, Wat}`
  (+ `Language::from_cell_language(s)`); `Highlighter::new(lang)` con
  `.highlight(&mut self, source: &str) -> Vec<Vec<Span>>` (un `Vec<Span>` **por
  línea**), `.set_language(lang)`, `.language()`; helpers de módulo
  `invalidate_tree_cache(lang)` y `apply_pending_edits(lang, &edits)` para el
  caché incremental. `TokenKind`, `Span`, `SyntaxPalette::color(kind)`.

**text-editor-lsp** — cliente LSP por stdin/stdout. `trait LspClient` (fire-and-forget
`request_*` + lecturas de caché `latest_*`/`clear_*`): completions, hover,
definition, references, rename, formatting, signature help, document symbols.
`RustAnalyzerClient::start(workspace_root)`; `NoopLspClient` para tests.

**clipboard** — portapapeles del sistema vía `arboard`. `SystemClipboard::new()`,
`is_available()`, impl `Clipboard`. No-op silencioso si no hay display (CI headless).

**menubar** — barra de menú mac-style. `menubar_view(&MenuBarSpec { menu: &AppMenu,
open: Option<usize>, theme, viewport, height, on_open: Fn(Option<usize>)->Msg,
on_command: Fn(&str)->Msg })`; dropdown en `view_overlay` con `menubar_overlay(spec)`
o `menubar_overlay_animated(spec, active, appear)`. Navegación por teclado:
`menubar_nav`, `menubar_command_at`.

**edit-menu** — menú estándar de edición sobre un editor.
`EditFlags::from_editor(&state, masked)`, `edit_context_menu(anchor, viewport,
&theme, flags, on_action: Fn(EditAction)->Msg, on_dismiss)` →
`ContextMenuSpec`. `apply(&mut state, EditAction, &mut clipboard) -> ApplyResult`.
`EditAction::{Undo,Redo,Cut,Copy,Paste,Delete,SelectAll}`.

**context-menu** — menú contextual genérico (look "webpage"). `ContextMenuItem::
action(label).with_shortcut(..).icon(..).disabled().destructive().submenu(children)`
o `::separator()`. `context_menu_view(ContextMenuSpec { anchor, viewport, header,
items, active, on_pick: Fn(usize)->Msg, on_dismiss, palette })`; `context_menu_view_ex`
con submenús/animación. Se monta en `view_overlay` con scrim.

**theme-switcher** — `theme_switcher_view(&current: &Theme, on_change: Fn(Theme)->Msg)`
(+ `_styled`/`_flex`). Cicla `Theme::next_after`.

**shortcuts-help** — overlay "?" con atajos agrupados. `shortcuts_help_view(
ShortcutsHelpSpec { title, groups: Vec<ShortcutGroup { title, entries:
Vec<ShortcutEntry { keys, description }> }>, viewport, on_dismiss, palette })`.

**wawa-mark** — sello vectorial del SO wawa. `wawa_mark_view(&WawaMarkPalette)`;
`paint_mark(scene, rect, &palette)` para canvas custom. Usar en contenedor cuadrado.

---

## 14. Catálogo de módulos

Los módulos encapsulan **estado + comportamiento** (a diferencia de los widgets).
Todos siguen el mismo contrato:

```
State  +  Msg  +  Action  +  apply(state, msg, ...) -> Action
                            +  on_key(state, &KeyEvent) -> Option<Msg>
                            +  open_shortcut(&KeyEvent) -> bool
                            +  view(state, ..., to_host: F) -> View<HostMsg>
                            +  Palette
```

La app guarda `Option<ModuleState>` (o el state directo, p. ej. bookmarks),
rutea el atajo de apertura con `open_shortcut`, rutea teclas con `on_key`, aplica
`Msg`s con `apply`, y monta el `view` pasando un mapeo `to_host: Fn(ModuleMsg) ->
HostMsg`. Cuando `apply` devuelve una `Action` (p. ej. `Invoke(id)`, `OpenAt{..}`,
`GoTo{..}`), la app ejecuta el efecto. Crates en `modules/<nombre>/`.

| Módulo | Atajo | Acción que devuelve | Propósito |
|---|---|---|---|
| **command-palette** | `Ctrl+Shift+P` | `Invoke(String)` | paleta de comandos fuzzy. El host declara `&[Command]` |
| **file-picker** | `Ctrl+P` | `Open(PathBuf)` | fuzzy file picker; host pasa `&[PathBuf]` + `root` |
| **fif** (find-in-files) | `Ctrl+Shift+F` | `OpenAt{path,line,col}`, `Searched{..}`, `Replaced{..}` | buscar/reemplazar; dual-view (dialog + barra). `search()` / `replace_all()` hacen el I/O |
| **diff-viewer** | `Ctrl+Shift+D` | — | diff side-by-side. `DiffState::new(before_label, after_label, before, after)` computa con `similar` |
| **mini-map** | `Ctrl+Shift+M` | `JumpTo(line)` | minimapa del buffer; agnóstico del editor (recibe `Snapshot`) |
| **bookmarks** | `Ctrl+Alt+B` toggle, `Ctrl+Shift+B` lista, `Ctrl+Alt+N/P` nav | `JumpTo{path,line}` | marcadores per-file persistentes (state directo, no Option) |
| **symbol-outline** | `Ctrl+Shift+O` | `GoTo{line,col}` | outline de símbolos; host arma `Vec<SymbolItem>` (LSP/tree-sitter/custom) |
| **selector** | — | — | abstracción portátil abrir/guardar: `trait Selector` (`HostSelector` con PathBuf, `WawaSelector` placeholder content-addressed) |
| **plugin-host** | — | `OpenAt{..}`, `SetStatus(..)` | runtime WASM (wasmi) con permisos por bitfield; `PluginHost::load_from_dir`/`invoke(id, cap, args)` |
| **shuma-term** | `` Ctrl+` `` | `SetStatus(..)` | terminal integrada. `spawn(cwd)` lanza PTY (`shuma_exec`), `vt100::Parser` renderiza; `Tick` drena el PTY |

Patrón típico de integración (command-palette):
```rust
struct Model { palette: Option<PaletteState>, commands: Vec<Command>, /* … */ }
enum Msg { Palette(PaletteMsg), /* … */ }

// on_key:
if command_palette::open_shortcut(ev) { return Some(Msg::Palette(PaletteMsg::Open)); }
if let Some(_) = &model.palette { return command_palette::on_key(p, ev).map(Msg::Palette); }

// update:
Msg::Palette(m) => {
    if let Some(state) = model.palette.as_mut() {
        match command_palette::apply(state, m, &model.commands) {
            PaletteAction::Invoke(id) => { /* ejecutar comando id */ model.palette = None; }
            PaletteAction::Close => model.palette = None,
            PaletteAction::None => {}
        }
    }
}

// view_overlay:
model.palette.as_ref().map(|s|
    command_palette::view(s, &model.commands, &palette, Msg::Palette))
```

---

## 15. `llimphi-workspace` — chasis tipo tmux

Monta cualquier componente en un layout intercambiable con splits resizables
(máquina de estados de foco/split/cierre + chrome estándar). Construido sobre
`llimphi-widget-panes`.

```rust
Workspace::new()
  .focused() -> PaneId         .count()      .leaves() -> Vec<PaneId>     .layout() -> &Layout
  .focus(id)  .split(Axis) -> PaneId   .close() -> Option<PaneId>   .resize(&path, delta)
  .apply(WsMsg) -> WsEffect

enum WsMsg { Focus(PaneId), Split(Axis), Close, Resize(Vec<Side>, f32) }
enum WsEffect { None, Created(PaneId), Closed(PaneId) }

workspace_view(&ws, &WorkspacePalette,
    leaf: FnMut(PaneId)->View<Host>,           // materializa el contenido de cada panel
    lift: Fn(WsMsg)->Host)                      // sube los Msg del chasis a tu Msg
```

Patrón: `enum Msg { Ws(WsMsg), Panel(PaneId, PanelMsg) }`. En `update`,
`ws.apply(msg)` te avisa con `WsEffect::{Created,Closed}(id)` para que crees o
destruyas el estado del panel correspondiente.

---

## 16. Reglas duras y gotchas

### 🔴 Cómputo pesado fuera del hilo de UI (PRIORIDAD URGENTE)
Ningún `update`/`init`/handler puede ejecutar trabajo **síncrono** pesado
(efemérides, simulación, IO, parse, embeddings, layout de árboles grandes).
Bloquea el hilo → "Not Responding". **`init` corre dentro de `resumed`, después
de crear la ventana**, así que un cómputo pesado ahí ya congela una ventana
visible.

Patrón (referencia: `cosmos-app-llimphi`):
```rust
// Model: Option<Resultado> (None = "calculando…") + flag dirty + contador de generación.
struct Model { x: Option<Resultado>, x_dirty: bool, x_gen: u64 }
enum Msg { XComputed(u64, Arc<Resultado>) }

// al FINAL de update() (que tiene el Handle):
if m.x_dirty {
    m.x_dirty = false;
    m.x_gen = m.x_gen.wrapping_add(1);
    let (gen, input) = (m.x_gen, m.input.clone());     // sólo lo que el worker necesita (Send)
    handle.spawn(move || Msg::XComputed(gen, Arc::new(compute(&input))));
}
// al recibir: aplicar SÓLO si la generación sigue vigente (evita que un
// resultado tardío pise a uno más nuevo en drags/toggles rápidos).
Msg::XComputed(gen, x) => if gen == m.x_gen {
    m.x = Some(Arc::try_unwrap(x).unwrap_or_else(|a| (*a).clone()));
}
```
La **generación** es imprescindible si el recálculo se dispara seguido. Ver
[`COMPUTO-FUERA-DEL-HILO-UI.md`](COMPUTO-FUERA-DEL-HILO-UI.md) y su checklist por app.

### Otras
- **Solvers iterativos** (Newton/bisección): cota dura `for _ in 0..N`, nunca
  `loop {}` con corte pegado al epsilon de f64 — en debug no converge → loop
  infinito.
- **Backend GPU**: preferir Vulkan (`Backends::PRIMARY`); el GL de Mesa sobre
  Wayland segfaultea en el teardown. Ya está hecho en `Hal::new`, no revertir.
- **Un nodo es draggable o clickable**, no ambos.
- **`alpha` y `clip`** crean capas intermedias: tienen costo, usar sólo cuando
  hace falta.
- **`paint_with`** no debe dejar `push_layer` sin `pop_layer` ni resetear la
  Scene.
- **Hit-test respeta `.transform()`**: un nodo rotado/escalado/trasladado recibe
  los clicks donde se ve pintado (el runtime invierte el afín acumulado). Lo que
  **no** se ajusta todavía: la posición local que reciben los handlers `*_at` se
  reporta en coords de pantalla, no en el espacio local del nodo transformado.
- **GPUI está extinto**: no agregar dependencias ni código GPUI (regla §3 de
  `CLAUDE.md`).
- **Texto en regla pesada**: crear un `Typesetter` por frame es caro
  (`FontContext::new` enumera fuentes del sistema). El runtime ya cachea uno y lo
  pasa a `paint_with`.

---

## 17. Comandos y demos

```bash
cargo check --workspace                              # smoke test mínimo (debe pasar siempre)
cargo run -p <crate> --release                       # correr una app
cargo run -p <crate> --example <demo> --release      # correr un demo

# demos del propio framework:
cargo run -p llimphi-ui      --example counter --release   # bucle Elm completo
cargo run -p llimphi-ui      --example editor  --release   # text field + teclado
cargo run -p llimphi-ui      --example gpu_paint_demo --release
cargo run -p llimphi-gallery --release                     # showcase de TODO el kit
cargo run -p nada            --release                     # editor real para ejercitar widgets

# benchmark GPU directo vs vello:
cargo run -p llimphi-gpu-bench --release
```

`llimphi-gallery` (`src/main.rs`, ~967 líneas) es la **referencia viva** del
patrón completo: `Model`/`Msg`/`init`/`update`/`view`/`view_overlay` con overlays
mutuamente excluyentes (modal > atajos > toasts > context-menu > dropdown).
Controles: click en switches/segments; "Mostrar toast"/"Abrir modal"; `?` abre
atajos; `Esc` cierra el overlay activo.

---

## 18. Cheat-sheet

```rust
// ── App mínima ──────────────────────────────────────────────
impl App for X { type Model; type Msg; init; update; view; }
llimphi_ui::run::<X>();

// ── Nodo ────────────────────────────────────────────────────
View::new(Style{ flex_direction, size, gap, padding, align_items, justify_content, ..default() })
    .fill(c).fill_gradient(g).hover_fill(c).radius(r).radius_corners(tl,tr,br,bl).shadow(sh).border(w,c).clip(b).alpha(a).transform(xf).animated(key,dur)
    .text(s, px, c) | .text_aligned(s,px,c,al) | .text_runs(s,px,c,runs,al) | .text_weight(w) | .bold() | .ellipsis(n) | .max_lines(n)
    .image(img) | .paint_with(|scene,ts,rect|{}) | .gpu_paint_with(|d,q,enc,view,rect,vp|{})
    .on_click(m) | .on_click_at(|lx,ly,w,h|) | .on_right_click(m) | .on_middle_click(m)
    .on_pointer_enter(m) | .on_pointer_leave(m)
    .draggable(|ph,dx,dy|) | .draggable_at(|ph,dx,dy,lx0,ly0|)
    .drag_payload(id) | .on_drop(|id|) | .drop_hover_fill(c)
    .children(vec![..])

// ── Efectos ─────────────────────────────────────────────────
handle.spawn(|| Msg::Done(compute()));          // worker → reentra al update
handle.spawn_periodic(dur, || Msg::Tick);       // feed periódico
handle.dispatch(Msg::X);  handle.quit();

// ── Estilo de layout (taffy prelude) ────────────────────────
length(px)  percent(0..1)  Dimension::auto()
FlexDirection::{Row,Column}  AlignItems::{Start,Center,End,Stretch}
JustifyContent::{Start,Center,End,SpaceBetween}

// ── Theme ───────────────────────────────────────────────────
Theme::dark()/light()/aurora()/sunset();  Theme::next_after(name);  XxxPalette::from_theme(&t)

// ── Overlay (menús/modales) ─────────────────────────────────
fn view_overlay(m) -> Option<View<Msg>> { if m.open { Some(menu) } else { None } }
```

---

## 19. Índice de crates

**Framework** (`02_ruway/llimphi/`):
`llimphi-hal` · `llimphi-raster` · `llimphi-text` · `llimphi-layout` ·
`llimphi-compositor` · `llimphi-ui` · `llimphi-theme` · `llimphi-motion` ·
`llimphi-icons` · `llimphi-surface` · `llimphi-workspace` · `llimphi-gallery` ·
`llimphi-gpu-bench`.

**Widgets** (`widgets/`, dep `llimphi-widget-<n>`): app-header · avatar · badge ·
banner · breadcrumb · button · card · clipboard · context-menu · edit-menu ·
empty · field · gallery · grid · list · menubar · modal · navigator · nodegraph ·
panel · panes · progress · segmented · shortcuts-help · skeleton · slider · splash ·
splitter · stat-card · status-bar · switch · tabs · text-area · text-editor ·
text-editor-core · text-editor-lsp · text-input · theme-switcher · tiled ·
timeline · toast · tooltip · tree · wawa-mark.

**Módulos** (`modules/`): bookmarks · command-palette · diff-viewer · fif ·
file-picker · mini-map · plugin-host · selector · shuma-term · symbol-outline.

**Android** (`android/`): clear-screen-android · vello-hello-android ·
vello-text-android.

---

> Documentos hermanos: [`SDD.md`](SDD.md) (diseño y roadmap),
> [`COMPUTO-FUERA-DEL-HILO-UI.md`](COMPUTO-FUERA-DEL-HILO-UI.md) (regla de
> concurrencia), [`README.md`](README.md) / [`LEEME.md`](LEEME.md) (overview).
> Las firmas de este manual reflejan el código al 2026-06-01; ante divergencia,
> la fuente autoritativa es el `lib.rs` de cada crate.
