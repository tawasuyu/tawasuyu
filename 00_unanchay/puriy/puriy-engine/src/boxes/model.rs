//! Modelo de datos del box tree: `Color`, `Display`, `BoxNode` (el nodo con
//! todos sus campos visuales/layout), `BoxTree`, y los tipos auxiliares de
//! `<svg>` (`SvgScene`/`SvgPrim`/`PathCmd`), `<select>` (`SelectInfo`), `<form>`
//! (`FormInfo`/`FormMethod`), inputs (`InputKind`) e imágenes (`ImageData`).
//! Extraído de `boxes/mod.rs` (regla #1). Comparte tipos del crate vía `use super::*`.
use super::*;

/// Color RGBA, 8 bits por canal. Suficiente para CSS color values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb_const(0, 0, 0);
    pub const WHITE: Color = Color::rgb_const(255, 255, 255);
    pub const TRANSPARENT: Color = Color { r: 0, g: 0, b: 0, a: 0 };

    pub const fn rgb_const(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgb_const(r, g, b)
    }
}

/// Modos de visualización soportados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    /// CSS flexbox container (block-level). El layout se delega a taffy
    /// con `flex_direction`, `justify_content`, `align_items`, `gap` y
    /// `flex_wrap` provistos por las propiedades del nodo.
    Flex,
    /// `inline-flex`: igual que Flex pero se comporta como inline en el
    /// flow del padre.
    InlineFlex,
    /// CSS grid container — mapea al algoritmo de grid de taffy con
    /// `grid_template_columns` y `grid_template_rows` del nodo.
    Grid,
    /// `inline-grid`: igual que Grid pero inline en el flow del padre.
    InlineGrid,
    None,
}

/// Un nodo del árbol de boxes — render-ready.
#[derive(Debug, Clone)]
pub struct BoxNode {
    pub display: Display,
    pub background: Option<Color>,
    pub color: Color,
    pub font_size: f32,
    /// 400 = normal, 700 = bold. Por ahora discreto: `< 600` se trata
    /// como normal y `>= 600` como bold (Llimphi text aún no expone
    /// weight axis arbitrario).
    pub font_weight: u16,
    /// CSS `font-style`: normal vs italic/oblique. Heredable.
    pub font_style: crate::style::FontStyle,
    /// CSS `font-family` como string CSS (acepta listas con fallbacks).
    /// `None` = default del runtime. Heredable.
    pub font_family: Option<String>,
    pub margin: Sides<f32>,
    pub padding: Sides<f32>,
    /// Ancho explícito CSS (`auto` por defecto).
    pub width: LengthVal,
    /// Alto explícito CSS (`auto` por defecto = lo dimensiona el contenido).
    pub height: LengthVal,
    /// Tope superior del ancho.
    pub max_width: LengthVal,
    /// Alineación del texto inline dentro del bloque.
    pub text_align: TextAlign,
    /// Multiplicador line-height (font-size * line_height = altura
    /// de línea). `None` → caller usa 1.2 como default (matchea
    /// browser CSS `normal`; antes 1.4 — más generoso pero menos
    /// compacto que el render real).
    pub line_height: Option<f32>,
    /// Ancho del border en px por lado.
    pub border_widths: Sides<f32>,
    /// Color del border por lado. `None` = ese lado no se dibuja.
    pub border_colors: Sides<Option<Color>>,
    /// Radio corner-radius en px por esquina.
    pub border_radii: Corners<f32>,
    /// Background a aplicar cuando el nodo está bajo el mouse. `None` =
    /// no hay regla `:hover` que cambie el background del nodo. El
    /// chrome lo plug-ea vía `View::hover_fill`. Restyle completo en
    /// hover (cambios de color/border) queda fuera de scope por ahora.
    pub hover_background: Option<Color>,
    /// Background a aplicar cuando el nodo está focado (input/textarea
    /// actualmente focado por el usuario). Mismo modelo limitado que
    /// `hover_background`: sólo el delta de bg, no se propaga a
    /// ancestros (`:focus` aplica al sujeto del selector).
    pub focus_background: Option<Color>,
    /// Box-shadow propagado a `paint_with` en el chrome.
    pub box_shadow: Option<BoxShadow>,
    /// `z-index` aplicado al stacking order entre hermanos positioned.
    /// El chrome lo usa para reordenar children out-of-flow ascending —
    /// el mayor pinta encima. Para `position: static` se ignora.
    pub z_index: i32,
    /// Línea decorativa que el chrome dibuja sobre la hoja de texto
    /// (underline / line-through / overline). `None` = sin decoración.
    pub text_decoration: TextDecorationLine,
    /// Propiedades de flex container — sólo relevantes si `display` es
    /// `Flex`/`InlineFlex`. El chrome las mapea 1:1 a taffy.
    pub flex_direction: FlexDirection,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    /// `align-content` (distribución de líneas/pistas en el eje cruzado).
    pub align_content: AlignContent,
    /// `justify-items`/`justify-self` (grid). `None`/`Auto` = default taffy.
    pub justify_items: Option<AlignItems>,
    pub justify_self: AlignSelf,
    pub flex_wrap: FlexWrap,
    pub gap_row: f32,
    pub gap_column: f32,
    /// Modelo de caja: cómo cuenta padding/border en width.
    pub box_sizing: BoxSizing,
    /// Mínimos y máximo extra del axis sizing (width/max_width ya existían).
    pub min_width: LengthVal,
    pub min_height: LengthVal,
    pub max_height: LengthVal,
    /// CSS `aspect-ratio`. `None` = sin relación impuesta (`auto`).
    pub aspect_ratio: Option<f32>,
    /// `hidden` aplica clip() en el chrome.
    pub overflow: Overflow,
    /// `white-space` define cómo collapse_whitespace trata el texto.
    pub white_space: WhiteSpace,
    /// Aplicado al texto del nodo (si es leaf) o propagado por
    /// herencia a hijos text leaf.
    pub text_transform: TextTransform,
    /// 0..1 — el chrome multiplica el alpha del background/border.
    pub opacity: f32,
    /// Item-side de flex.
    pub align_self: AlignSelf,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: LengthVal,
    /// Outline pintado fuera del border (sin afectar layout).
    pub outline: Outline,
    /// Gradiente de fondo (linear-gradient). Si Some, el chrome lo
    /// pinta encima/en lugar del background sólido.
    pub background_gradient: Option<LinearGradient>,
    pub position: Position,
    pub inset_top: LengthVal,
    pub inset_right: LengthVal,
    pub inset_bottom: LengthVal,
    pub inset_left: LengthVal,
    pub vertical_align: VerticalAlign,
    pub visibility: Visibility,
    pub pointer_events: PointerEvents,
    pub text_indent: f32,
    pub word_spacing: f32,
    /// Espacio extra entre letras (px). Heredable. Almacenado pero aún no
    /// pintado (mismo estado que `word_spacing`).
    pub letter_spacing: f32,
    pub text_shadows: Vec<TextShadow>,
    pub transforms: Vec<Transform>,
    pub grid_template_columns: Vec<GridTrackSize>,
    pub grid_template_rows: Vec<GridTrackSize>,
    /// Texto plano del nodo (sólo para hojas de texto). Para nodos con
    /// hijos el texto vive en los hijos.
    pub text: Option<String>,
    pub children: Vec<BoxNode>,
    /// Tag HTML que originó el box (para debug y feature detection).
    pub tag: Option<String>,
    /// Destino absoluto si el nodo es un `<a href="…">`. Ya resuelto
    /// contra la URL base del documento — los consumidores no tienen
    /// que conocer la base.
    pub link: Option<String>,
    /// Imagen decodificada (RGBA8) si el nodo es un `<img src>` que
    /// pudo descargarse y decodificarse. PNG/JPEG soportados; otros
    /// formatos dejan `None` y el chrome muestra un placeholder.
    pub image: Option<ImageData>,
    /// `true` si el nodo es un `<details>` que arrancó con el atributo
    /// `open`. El chrome usa esto para inicializar el estado open/closed
    /// del primer render; subsiguientes toggles los gestiona él. Para
    /// nodos que no son `<details>` queda en `false` y no se consulta.
    pub details_open_attr: bool,
    /// `true` si el `<a>` lleva `target="_blank"` (o cualquier target
    /// no-self). El chrome lo usa para abrir en nueva pestaña al click.
    /// `false` para todo lo demás.
    pub link_new_tab: bool,
    /// Si el `<a>` lleva `download[=filename]`, el chrome descarga el
    /// target en lugar de navegarlo. `Some(String::new())` = usar el
    /// filename del path; `Some("foo.pdf")` = filename override.
    pub link_download: Option<String>,
    /// Imagen decodificada del CSS `background-image: url(...)`. `None`
    /// si la propiedad no estaba o si la descarga/decode falló. El
    /// chrome la pinta como background (detrás del background sólido y
    /// gradient).
    pub background_image: Option<ImageData>,
    /// Si el nodo es un `<input>` de tipo texto o un `<textarea>`, el
    /// chrome lo renderea como widget editable. `None` para todo lo
    /// demás. Multilinea = textarea.
    pub input_kind: Option<InputKind>,
    /// Valor inicial del input (atributo `value`). Sólo se consulta al
    /// crear el `TextInputState` la primera vez por pestaña; los toggles
    /// y typings los maneja el chrome.
    pub input_initial: Option<String>,
    /// Para `<input type=checkbox|radio>`: estado `checked` inicial.
    /// `false` por default.
    pub input_checked_initial: bool,
    /// `true` si el `<input>`/`<textarea>` lleva el attr `autofocus`. El
    /// chrome busca el primer matching al recibir `Msg::Loaded` y le
    /// asigna `focused_input` para empezar la sesión con el cursor ahí.
    pub input_autofocus: bool,
    /// Placeholder del input — atributo `placeholder` del `<input>` /
    /// `<textarea>`. `None` si vacío.
    pub input_placeholder: Option<String>,
    /// Atributo `name` del input — clave del par `name=value` que va al
    /// query string al submit. `None` = el input no se envía.
    pub input_name: Option<String>,
    /// Índice (en `BoxTree.forms`) del `<form>` que contiene a este nodo
    /// (más cercano hacia arriba en la jerarquía). `None` = no está
    /// dentro de un form, no se puede submitear.
    pub form_idx: Option<usize>,
    /// Si el nodo es `<select>`, este campo lleva la lista de opciones
    /// (con `value` y `label`) y el índice por default. El chrome lo
    /// rendera como dropdown editable y guarda el índice seleccionado
    /// en su `TabState`.
    pub select: Option<SelectInfo>,
    /// Si el nodo es `<svg>`, lista de primitivas a pintar. El chrome
    /// las renderea adentro del rect del nodo (escalado por `viewBox` si
    /// existe; sino cada primitiva usa sus coords nativas).
    pub svg: Option<SvgScene>,
    /// Si el nodo es `<canvas>`, su tamaño intrínseco `(width, height)` en
    /// px CSS, tomado de los atributos `width`/`height` (default 300×150
    /// por spec). El chrome casa el `element_id` de este box con el contexto
    /// 2D del runtime JS (`__puriy_collect_canvas`) y drena sus comandos de
    /// dibujo para pintarlos con vello. Fase 7.196.
    pub canvas: Option<(f32, f32)>,
    /// Atributo HTML `id="..."` del elemento — usado por fragment
    /// navigation (`<a href="#foo">` busca el nodo con `element_id ==
    /// Some("foo")` y scrollea hasta él). `None` para nodos sin id y
    /// para nodos sintéticos (markers, wrappers Document, hojas Text).
    pub element_id: Option<String>,
    /// Clases CSS del nodo (atributo `class="a b c"` split por espacio).
    /// Vacío para nodos sin clase. Para que el snapshot pasado a `puriy-js`
    /// pueda indexar elementos por class y soportar `querySelector('.foo')`
    /// — Fase 7.8.
    pub class_list: Vec<String>,
    /// **Todos** los atributos HTML del elemento (name lowercased + value
    /// literal). Esto incluye `data-*`, `aria-*`, `href`, `src`, `title`,
    /// `role`, etc. Los atributos ya parseados como campos dedicados
    /// (`id`, `class`, `href` para links, `src` para imgs, `value` para
    /// inputs) también aparecen acá — son redundantes pero permiten que
    /// `getAttribute('id')` funcione uniformemente desde JS sin sub-rutas.
    /// Fase 7.16. Antes (7.11) este campo se llamaba `dataset` y sólo
    /// guardaba los `data-*` sin prefijo.
    pub attributes: Vec<(String, String)>,
    /// Animación CSS resuelta para el runtime de tween (`anim.rs`). `Some`
    /// sólo cuando el nodo tiene `animation: <name> …` Y el `<name>` matchea
    /// un `@keyframes` conocido. El chrome la consume por frame:
    /// `anim::animation_progress(&binding, elapsed)` da el progreso eased y
    /// `anim::sample_keyframes(&keyframes, p)` el overlay a mergear sobre el
    /// estilo base. `None` = nodo no animado.
    pub animation: Option<AnimationInstance>,
    /// Bindings `transition` declarados en el nodo. El chrome los consulta
    /// (`anim::transition_for`) para tweenear cambios de estado (hover, etc.).
    pub transitions: Vec<crate::style::TransitionBinding>,
    /// Identidad estable del nodo dentro del árbol (1..N en orden DFS
    /// pre-orden), asignada por un post-pass de `build`. Permite al chrome
    /// llevar estado por-nodo (p. ej. el tween de `transition` en hover)
    /// keyeado por id, sin depender de contar índices en walks paralelos
    /// frágiles. `0` = sin asignar (raíz vacía o nodos sintetizados por
    /// mutaciones JS post-load, que no participan de transiciones).
    pub node_id: u32,
}

/// Animación CSS lista para tween: el binding parseado + la definición de
/// `@keyframes` ya resuelta por nombre. Vive en el `BoxNode` para que el
/// chrome no tenga que cargar la tabla de keyframes del `StyleEngine`.
/// Rescatado del frente engine.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationInstance {
    pub binding: crate::style::AnimationBinding,
    pub keyframes: crate::style::Keyframes,
}

impl BoxNode {
    /// Filtra `attributes` por prefijo `data-` y devuelve `(suffix, value)`.
    /// Spec del `el.dataset` API: `data-foo-bar` → key `foo-bar`. Cada
    /// llamada recorre los atributos; para nodos con miles no es óptimo
    /// pero es lo esperado para el uso típico (<10 attrs por elemento).
    /// Fase 7.16 — antes vivía como campo separado `dataset`.
    pub fn dataset(&self) -> Vec<(&str, &str)> {
        let mut out = Vec::new();
        for (k, v) in &self.attributes {
            if let Some(rest) = strip_data_prefix(k) {
                out.push((rest, v.as_str()));
            }
        }
        out
    }
}

/// Devuelve el sufijo si `name` empieza con `data-` (case-insensitive),
/// o `None` en caso contrario. Helper local porque `attributes` guarda
/// nombres lowercased; el matching es sobre prefix de 5 bytes ASCII.
pub(crate) fn strip_data_prefix(name: &str) -> Option<&str> {
    let b = name.as_bytes();
    if b.len() > 5 && b[..5].eq_ignore_ascii_case(b"data-") {
        Some(&name[5..])
    } else {
        None
    }
}

/// Escena SVG minimal: lista de primitivas + viewBox opcional.
#[derive(Debug, Clone)]
pub struct SvgScene {
    pub width: f32,
    pub height: f32,
    /// `(min_x, min_y, w, h)` del viewBox, o `None` si el SVG no lo
    /// declaró (las primitivas van directo a coords del viewport del svg).
    pub view_box: Option<(f32, f32, f32, f32)>,
    pub prims: Vec<SvgPrim>,
}

#[derive(Debug, Clone)]
pub enum SvgPrim {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        rx: f32,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_w: f32,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_w: f32,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stroke: Color,
        stroke_w: f32,
    },
    /// Polygon (cerrado) o polyline (abierto) — los puntos vienen del
    /// atributo `points="x1,y1 x2,y2 …"`.
    Polyline {
        points: Vec<(f32, f32)>,
        closed: bool,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_w: f32,
    },
    /// Path con secuencia de comandos. Subset: M (moveTo), L (lineTo),
    /// H/V (horizontal/vertical lineTo), C (cubic bezier), Q (quadratic
    /// bezier), Z (closepath). Todos en abs y rel (m/l/h/v/c/q/z).
    Path {
        d: Vec<PathCmd>,
        fill: Option<Color>,
        stroke: Option<Color>,
        stroke_w: f32,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum PathCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    QuadTo(f32, f32, f32, f32),
    ClosePath,
}

/// Datos de un `<select>` para renderizarlo como dropdown.
#[derive(Debug, Clone)]
pub struct SelectInfo {
    pub options: Vec<SelectOption>,
    /// Índice del `<option selected>` inicial, o `0` si ninguno lo era.
    pub initial: usize,
}

#[derive(Debug, Clone)]
pub struct SelectOption {
    /// Texto que el usuario ve.
    pub label: String,
    /// Valor que va al querystring (cae al `label` si el HTML no
    /// proveyó atributo `value`).
    pub value: String,
}

/// Metadata por `<form>` del documento — el chrome la usa al submit.
#[derive(Debug, Clone)]
pub struct FormInfo {
    /// URL absoluta del action (resuelta contra el base). `None` =
    /// submit a la URL actual de la página (CSS spec).
    pub action: Option<String>,
    /// Método HTTP del form — sólo soportamos `GET` por ahora (el más
    /// común y el que funciona sin manejo de bodies/cookies en puriy).
    pub method: FormMethod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormMethod {
    Get,
    /// POST no está implementado todavía — el chrome trata como GET y
    /// muestra un hint en status.
    Post,
}

/// Subconjunto de `<input type=...>` que renderemos como widget de texto.
/// Todo lo demás (checkbox/radio/file/range/submit/...) se trata como
/// box normal por ahora.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// `<input type=text>`, `<input>` sin type, search, email, url, tel,
    /// number, password — todos se ven como una línea editable. password
    /// idealmente mostraría bullets, eso lo decide el chrome.
    Text,
    Password,
    Search,
    /// `<textarea>` — multilínea.
    TextArea,
    /// `<input type=checkbox>` — toggle booleano.
    Checkbox,
    /// `<input type=radio>` — exclusivo por nombre de grupo (`name`
    /// compartido entre múltiples radios del mismo form).
    Radio,
    /// `<input type=submit|button>` — botón con label desde `value` (o
    /// `Submit` por default). Click submitea el form si está dentro de
    /// uno; sino no-op.
    Submit,
}

/// Imagen RGBA8 lista para que el chrome la envuelva en `peniko::Image`.
/// `rgba` tiene exactamente `4 * width * height` bytes en orden RGBA.
#[derive(Debug, Clone)]
pub struct ImageData {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Árbol de boxes. Wrapper para poder agregar utilidades.
#[derive(Debug, Clone)]
pub struct BoxTree {
    pub root: BoxNode,
    /// Forms del documento en orden DFS. Cada `<input>` que cae dentro
    /// de uno tiene `BoxNode.form_idx = Some(i)`.
    pub forms: Vec<FormInfo>,
    /// Motor de estilos del documento, retenido para poder re-correr la
    /// cascada CSS tras una mutación que cambie qué reglas matchean
    /// (`classList.add/remove/toggle`, `className`, `setAttribute('class')`).
    /// El DOM original se dropea tras la carga (es `!Send`), así que el
    /// restyle reconstruye un DOM espejo del propio box tree (Fase 7.184).
    pub styles: StyleEngine,
}
