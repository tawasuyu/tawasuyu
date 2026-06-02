//! llimphi-compositor — el núcleo declarativo de Llimphi, sin winit.
//!
//! Aquí vive el árbol de vista `View<Msg>` (DSL declarativo), su instalación
//! sobre taffy (`mount`), el pintado a `vello::Scene` (`paint`/`paint_gpu`) y
//! el hit-test. Nada de esto necesita una ventana ni `llimphi-hal`: la
//! composición `view → layout → scene` es pura y reutilizable.
//!
//! El runtime que la maneja vive aparte:
//! - `llimphi-ui` la corre sobre winit (`run<A: App>()`).
//! - a futuro, un runtime sobre el framebuffer del kernel `wawa` puede
//!   reusar exactamente este compositor sin arrastrar winit.
//!
//! `wgpu` entra sólo por la firma de [`GpuPaintFn`] (tipos de Device/Queue/
//! Encoder/TextureView); `wgpu` no depende de winit, así que el compositor
//! sigue libre de windowing.

use std::collections::HashMap;
use std::sync::Arc;

use llimphi_layout::taffy::NodeId;
use llimphi_layout::{ComputedLayout, LayoutTree, Style};
use vello::kurbo::{Affine, Point, Rect as KurboRect, RoundedRect};
use vello::peniko::{Color, Fill, Image, Mix};

mod render;
mod view;
pub use render::*;

/// Texto a pintar dentro de un nodo. Alineación por defecto `Center`
/// (horizontal y vertical), apta para labels de botón. Para layouts tipo
/// editor o párrafo, usar `.text_aligned(...)` con `Alignment::Start`.
pub struct TextSpec {
    pub content: String,
    pub size_px: f32,
    pub color: Color,
    pub alignment: llimphi_text::Alignment,
    /// `true` = forzar variante italic en la fuente activa. Default false.
    pub italic: bool,
    /// CSS-style font-family string (acepta lista con fallbacks). `None`
    /// = la fuente default de parley.
    pub font_family: Option<String>,
    /// Múltiplo de interlínea (`line-height` / `font-size`). 1.2 es el
    /// default que usaban todos los callers; puriy lo sobreescribe con el
    /// valor computado de CSS. Se usa tanto al **medir** (para que taffy
    /// reserve el alto correcto) como al **pintar**, así medida y dibujo
    /// coinciden.
    pub line_height: f32,
    /// Colores por rango de **bytes** sobre `content`, para texto multicolor
    /// (syntax highlighting) en una sola pasada de shaping. `None` = color
    /// uniforme (`color`). Cuando es `Some`, el runtime usa
    /// `Typesetter::layout_runs` + `draw_layout_runs`, y `color` actúa como
    /// color por defecto de lo no cubierto por ningún run.
    pub runs: Option<Vec<(usize, usize, Color)>>,
}

/// Fase de un drag activo. `Move` se emite por cada `CursorMoved` con el
/// delta desde el evento anterior; `End` se emite al soltar el botón.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragPhase {
    Move,
    End,
}

/// Handler de drag. Recibe la fase + delta (`dx`, `dy`) **desde el evento
/// anterior** (no acumulado desde el press). Devolver `None` deja el drag
/// activo sin disparar Msg. `Arc<dyn Fn>` para que el runtime pueda
/// clonarlo barato al iniciar el drag y mantenerlo vivo aunque el cache
/// de la vista se regenere mientras tanto.
pub type DragFn<Msg> = Arc<dyn Fn(DragPhase, f32, f32) -> Option<Msg> + Send + Sync>;

/// Handler de drop. El runtime lo invoca cuando un drag activo se suelta
/// sobre este nodo. Recibe el `payload` `u64` que el origen del drag
/// declaró vía [`View::drag_payload`]. Devolver `None` ignora el drop.
///
/// Los IDs `u64` son opacos para el runtime: el widget elige una
/// convención (índice de tile, hash del item, etc.) y el handler decide
/// qué Msg emitir en función de ese ID.
pub type DropFn<Msg> = Arc<dyn Fn(u64) -> Option<Msg> + Send + Sync>;

/// Handler de click con posición. Recibe `(x_local, y_local, rect_w,
/// rect_h)`: las dos primeras son la posición del cursor **relativa a
/// la esquina superior-izquierda del nodo** y las dos últimas son el
/// ancho/alto actual del nodo en pixels — útil cuando el caller
/// necesita centrar o normalizar. Devolver `None` no dispara update.
pub type ClickAtFn<Msg> = Arc<dyn Fn(f32, f32, f32, f32) -> Option<Msg> + Send + Sync>;

/// Variante de [`DragFn`] que **conoce la posición inicial del press**
/// relativa al rect del nodo. Útil cuando el caller necesita identificar
/// qué entidad (Concepto, lemming, etc.) bajo el cursor agarró el drag.
/// Recibe `(phase, dx, dy, initial_lx, initial_ly)`.
pub type DragAtFn<Msg> = Arc<dyn Fn(DragPhase, f32, f32, f32, f32) -> Option<Msg> + Send + Sync>;

/// Rect absoluto del nodo (en coordenadas físicas del frame). Lo
/// recibe el callback de [`View::paint_with`] para que pueda
/// posicionar sus primitivas custom dentro del nodo.
#[derive(Debug, Clone, Copy, Default)]
pub struct PaintRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Callback de pintura custom. El runtime lo invoca durante el paint
/// del nodo (entre el `fill`/`image` y el `text`) con el `Scene` vivo
/// + el `Typesetter` cacheado del runtime + el rect absoluto del nodo.
/// Pensado para "canvas elements" tipo `dominium-canvas`,
/// `pluma-editor` (osciloscopio de coherencia), `cosmos` (charts).
///
/// El `Typesetter` se pasa porque crearlo por frame es caro
/// (`FontContext::new` enumera las fontes del sistema vía fontique).
/// Los callers que no necesiten texto pueden ignorar el argumento.
///
/// El callback no debe llamar a `scene.push_layer` sin un `pop_layer`
/// correspondiente, ni reset el scene — sólo agregar primitivas que
/// pertenezcan al rect del nodo.
pub type PaintFn = Arc<
    dyn Fn(&mut vello::Scene, &mut llimphi_text::Typesetter, PaintRect) + Send + Sync,
>;

/// Callback de pintura GPU directo, sin vello intermedio. Recibe el
/// `device`/`queue` ya construidos por el runtime más un
/// `CommandEncoder` y la `TextureView` del frame (la intermediate
/// `Rgba8Unorm` de `WinitSurface`), todo durante el paint del nodo.
///
/// El caller abre su propio `begin_render_pass` con `LoadOp::Load` para
/// no sobrescribir lo que ya pintó vello, dibuja sus primitivas y
/// cierra el pass. El runtime se encarga de dispatchear (`queue.submit`)
/// el encoder ya con todas las pasadas de todos los nodos acumuladas —
/// es un solo submit por frame.
///
/// **Orden de pintura en Fase 1**: todos los `gpu_painter` corren
/// DESPUÉS de la pasada completa de vello (fill, image, painter,
/// text) sobre el `mounted` tree. Entre sí mantienen el orden DFS
/// pre-orden. Si una app necesita pintar texto **encima** del render
/// GPU directo, la forma idiomática es ponerlo en `App::view_overlay`,
/// que se renderiza como una segunda Scene de vello encima de todo.
///
/// Pensado para apps con volumen masivo de primitivos (cosmos
/// starfield Gaia, tinkuy particle viewer, nakui viewport, pineal
/// denso) — el hook que paga el costo de mantener pipelines WGSL
/// propias en `llimphi-raster` (ver `02_ruway/llimphi/SDD.md`
/// §"Roadmap — GPU directo wgpu").
pub type GpuPaintFn = Arc<
    dyn Fn(
            &wgpu::Device,
            &wgpu::Queue,
            &mut wgpu::CommandEncoder,
            &wgpu::TextureView,
            PaintRect,
            (u32, u32),
        ) + Send
        + Sync,
>;

/// Nodo de la vista declarativa. Estilo de layout (taffy) + relleno opcional
/// (vello) + texto opcional (skrifa+vello) + Msg al click opcional + hijos.
pub struct View<Msg> {
    pub style: Style,
    pub fill: Option<Color>,
    /// Relleno cuando el cursor está sobre este nodo. Sin valor (`None`)
    /// = no se reacciona al hover.
    pub hover_fill: Option<Color>,
    pub radius: f64,
    pub text: Option<TextSpec>,
    /// Imagen a pintar dentro del rect del nodo. Se centra y escala
    /// preservando aspect ratio (`min(rect.w/img.w, rect.h/img.h)`).
    /// El alfa por píxel de la imagen y el `Image::alpha` global se
    /// respetan; el `fill` (si lo hay) se pinta debajo como background.
    pub image: Option<Image>,
    /// Callback de pintura custom. Si está presente, el runtime lo
    /// invoca durante el paint del nodo con el `Scene` vivo + el rect
    /// absoluto. Pensado para "canvas elements" (dominium, pluma,
    /// cosmos) que pintan primitivas custom no expresables como una
    /// composición de Views.
    pub painter: Option<PaintFn>,
    /// Pintor GPU directo. Se invoca DESPUÉS de la pasada vello del
    /// frame; comparte tree y orden DFS con los demás. Ver
    /// [`GpuPaintFn`].
    pub gpu_painter: Option<GpuPaintFn>,
    pub on_click: Option<Msg>,
    /// Handler de click que recibe la posición **relativa al rect del
    /// nodo** (esquina superior-izquierda del nodo = `(0, 0)`). Útil
    /// para canvas elements que quieren mapear el click a coordenadas
    /// de mundo. Si está presente, gana sobre `on_click`. Devolver
    /// `None` no dispara update.
    pub on_click_at: Option<ClickAtFn<Msg>>,
    /// Equivalente a `on_click` pero para el botón derecho del ratón.
    /// Pensado para menús contextuales: el nodo declara qué `Msg`
    /// emitir cuando se le hace right-click, y la app abre el overlay
    /// con el menú.
    pub on_right_click: Option<Msg>,
    /// Variante posicional de [`Self::on_right_click`]. Útil para
    /// grillas que necesitan saber *qué celda* del rect recibió el
    /// click derecho (la celda no es un nodo aparte, sino una región
    /// dentro del nodo). Si está presente, gana sobre `on_right_click`.
    pub on_right_click_at: Option<ClickAtFn<Msg>>,
    /// Equivalente a `on_click` pero para el botón del medio del ratón
    /// (rueda presionada). Pensado para abrir en pestaña nueva — los
    /// browsers usan middle-click como atajo equivalente a Ctrl+Click.
    pub on_middle_click: Option<Msg>,
    /// Handler de drag. Si está presente, este nodo arrastra (y NO emite
    /// `on_click` al presionar — un nodo es uno u otro).
    pub drag: Option<DragFn<Msg>>,
    /// Variante de drag que recibe la posición inicial del press relativa
    /// al rect del nodo. Gana sobre `drag` si ambos están presentes.
    pub drag_at: Option<DragAtFn<Msg>>,
    /// Payload `u64` que viaja con el drag iniciado sobre este nodo. Lo
    /// recibe el handler [`Self::on_drop`] del drop target. Sin payload,
    /// el drag funciona igual pero ningún drop target reacciona.
    pub drag_payload: Option<u64>,
    /// Handler invocado al soltar un drag sobre este nodo (drop target).
    pub on_drop: Option<DropFn<Msg>>,
    /// Color a pintar mientras un drag activo está hovereando este drop
    /// target. Sobrepone a `fill`/`hover_fill` cuando aplica.
    pub drop_hover_fill: Option<Color>,
    /// Si `true`, los descendientes se recortan al rect del nodo (vía
    /// `scene.push_layer` con `Mix::Clip`). El hit-test también respeta
    /// el recorte: clicks fuera del rect ignoran a los hijos.
    pub clip: bool,
    /// Msg a emitir cuando el cursor entra al rect del nodo (transición
    /// no-hover → hover). Útil para previews tipo "URL del link al
    /// pasar el mouse".
    pub on_pointer_enter: Option<Msg>,
    /// Msg a emitir cuando el cursor sale del rect del nodo.
    pub on_pointer_leave: Option<Msg>,
    /// Opacidad multiplicada sobre TODO el subtree (este nodo + hijos),
    /// en `[0.0, 1.0]`. Se realiza con `scene.push_layer(Mix::Normal, a, …)`
    /// alrededor del rect del nodo: el subárbol se rasteriza en una capa
    /// intermedia y se compone al alfa indicado contra lo que ya hay
    /// detrás. `None` = sin capa (caso de la abrumadora mayoría de
    /// nodos). Útil para fade-in/out de overlays, ghosts mientras se
    /// arrastra, modales que aparecen, panels "vidrio". Note que la
    /// composición tiene costo (allocate + blit), por lo que sólo
    /// poblar este slot cuando hace falta — no es un atributo gratis.
    pub alpha: Option<f32>,
    /// Transformación afín 2D aplicada a este nodo y todo su subtree
    /// **alrededor del centro de su propio rect** (convención CSS
    /// `transform-origin: 50% 50%`). El runtime resuelve el centro en
    /// `paint` (sólo entonces conoce el layout computado) y compone
    /// `T(centro) · transform · T(-centro)` sobre la transformación
    /// acumulada del padre, así nodos anidados transforman en el espacio
    /// ya transformado de su ancestro — igual que CSS. `None` = identidad
    /// (la abrumadora mayoría de nodos). Pensado para `transform`/
    /// `@keyframes` CSS de puriy (rotate/scale/translate). El hit-test
    /// **respeta** el afín (un nodo transformado recibe clicks donde se ve
    /// pintado). Limitación restante: los `painter`/`runs` custom no heredan
    /// el afín, y la posición local que reciben los handlers `*_at` se
    /// reporta en espacio de pantalla, no en el espacio local del nodo.
    pub transform: Option<Affine>,
    pub children: Vec<View<Msg>>,
}

/// Versión "instalada" del árbol: cada nodo tiene su NodeId de taffy, color
/// y handler. Se mantiene en orden de inserción (recorrido pre-orden), así
/// el hit-test puede iterar al revés para honrar el orden de pintado.
///
/// `pub` (con campos `pub`) porque el runtime (llimphi-ui) lee el árbol
/// montado para hit-test y para la pasada GPU directa, pero vive en otro
/// crate. No se construye fuera de [`mount`].
pub struct Mounted<Msg> {
    pub root: NodeId,
    pub nodes: Vec<MountedNode<Msg>>,
    /// Contenido de texto por nodo-hoja, para que el runtime lo mida con
    /// parley durante `compute_with_measure` y taffy reserve el alto real
    /// del texto envuelto (varias líneas) en vez de una sola. Sin esto un
    /// párrafo que envuelve a N líneas se aplastaría en la altura de una
    /// (el bug clásico de "textos aplastados"). Sólo se pueblan hojas con
    /// texto uniforme (sin `runs` multicolor, que el caller dimensiona).
    pub text_measures: HashMap<NodeId, TextMeasure>,
}

/// Datos de un nodo-hoja de texto necesarios para medirlo (shaping +
/// line-break) sin volver a tocar el `View`. Lo consume el runtime en la
/// función de medición que le pasa a [`LayoutTree::compute_with_measure`].
#[derive(Clone)]
pub struct TextMeasure {
    pub content: String,
    pub size_px: f32,
    pub alignment: llimphi_text::Alignment,
    pub italic: bool,
    pub font_family: Option<String>,
    pub line_height: f32,
}

pub struct MountedNode<Msg> {
    pub id: NodeId,
    pub fill: Option<Color>,
    pub hover_fill: Option<Color>,
    pub radius: f64,
    pub text: Option<TextSpec>,
    pub image: Option<Image>,
    pub painter: Option<PaintFn>,
    pub gpu_painter: Option<GpuPaintFn>,
    pub on_click: Option<Msg>,
    pub on_click_at: Option<ClickAtFn<Msg>>,
    pub on_right_click: Option<Msg>,
    pub on_right_click_at: Option<ClickAtFn<Msg>>,
    pub on_middle_click: Option<Msg>,
    pub drag: Option<DragFn<Msg>>,
    pub drag_at: Option<DragAtFn<Msg>>,
    pub drag_payload: Option<u64>,
    pub on_drop: Option<DropFn<Msg>>,
    pub drop_hover_fill: Option<Color>,
    pub clip: bool,
    pub on_pointer_enter: Option<Msg>,
    pub on_pointer_leave: Option<Msg>,
    pub alpha: Option<f32>,
    /// Transformación afín 2D del nodo (alrededor del centro de su rect).
    /// Ver [`View::transform`]. `paint` la compone con la del padre.
    pub transform: Option<Affine>,
    /// Índice (exclusivo) del fin del subárbol en `Mounted::nodes`. Los
    /// descendientes ocupan `[idx + 1, subtree_end)`. Hace de "barrera" en
    /// paint/hit_test para `pop_layer` y para saltar subárboles enteros.
    pub subtree_end: usize,
}
