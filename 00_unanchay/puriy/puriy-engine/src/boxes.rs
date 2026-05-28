//! Box tree — output del engine, entrada de `llimphi-raster`.
//!
//! Un [`BoxNode`] es la unidad de pintado: rectángulo con fondo opcional
//! + texto opcional + lista ordenada de hijos. No hay layout real (no
//! corremos taffy todavía) — sólo posicionamiento naive: cada bloque
//! apila vertical, cada inline se concatena en la línea. Es suficiente
//! para que Llimphi pueda dibujar example.com legible.
//!
//! Fase 3 reemplazará este pase por `llimphi-layout` con taffy.

use markup5ever_rcdom::{Handle, NodeData};

use crate::dom::{self, DomTree};
use crate::style::{
    AlignItems, AlignSelf, BoxShadow, BoxSizing, ComputedStyle, Corners, FlexDirection, FlexWrap,
    GridTrackSize, JustifyContent, LengthVal, LinearGradient, ListStyleType, Outline, Overflow,
    PointerEvents, Position, Sides, StyleEngine, TextAlign, TextDecorationLine, TextShadow,
    TextTransform, Transform, VerticalAlign, Visibility, WhiteSpace,
};

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
    /// Tope superior del ancho.
    pub max_width: LengthVal,
    /// Alineación del texto inline dentro del bloque.
    pub text_align: TextAlign,
    /// Multiplicador line-height (font-size * line_height = altura
    /// de línea). `None` → caller usa 1.4 como default.
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
    /// Línea decorativa que el chrome dibuja sobre la hoja de texto
    /// (underline / line-through / overline). `None` = sin decoración.
    pub text_decoration: TextDecorationLine,
    /// Propiedades de flex container — sólo relevantes si `display` es
    /// `Flex`/`InlineFlex`. El chrome las mapea 1:1 a taffy.
    pub flex_direction: FlexDirection,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub flex_wrap: FlexWrap,
    pub gap_row: f32,
    pub gap_column: f32,
    /// Modelo de caja: cómo cuenta padding/border en width.
    pub box_sizing: BoxSizing,
    /// Mínimos y máximo extra del axis sizing (width/max_width ya existían).
    pub min_width: LengthVal,
    pub min_height: LengthVal,
    pub max_height: LengthVal,
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
    /// Atributo HTML `id="..."` del elemento — usado por fragment
    /// navigation (`<a href="#foo">` busca el nodo con `element_id ==
    /// Some("foo")` y scrollea hasta él). `None` para nodos sin id y
    /// para nodos sintéticos (markers, wrappers Document, hojas Text).
    pub element_id: Option<String>,
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
}

impl BoxTree {
    /// Cuenta total de boxes (incluyendo la raíz).
    pub fn descendants_count(&self) -> usize {
        count(&self.root)
    }

    /// Recorre el árbol pre-order y aplica `f` a cada box.
    pub fn walk(&self, mut f: impl FnMut(&BoxNode)) {
        walk_inner(&self.root, &mut f);
    }

    /// Estima la posición vertical (px desde el top del documento) del
    /// nodo con `element_id == id`. Usada por fragment navigation
    /// (`<a href="#foo">`) — el chrome ajusta `scroll_y` a este valor.
    /// La estimación suma margin+padding de bloques y `font_size *
    /// line_height` de hojas de texto en orden DFS; ignora layout real
    /// (taffy todavía no corrió cuando el chrome resuelve el click), así
    /// que el salto puede caer ~1 línea arriba o abajo del target. Es
    /// suficiente para que el usuario vea el destino sin perderse.
    pub fn find_element_y(&self, id: &str) -> Option<f32> {
        let mut acc = 0.0_f32;
        find_y_inner(&self.root, id, &mut acc)
    }
}

fn find_y_inner(b: &BoxNode, target: &str, acc: &mut f32) -> Option<f32> {
    if b.element_id.as_deref() == Some(target) {
        return Some(*acc);
    }
    if b.text.is_some() {
        // Hoja de texto: una línea de altura font_size * line_height.
        *acc += b.font_size * b.line_height.unwrap_or(1.4);
        return None;
    }
    // Block-ish: contribución de borders verticales del lado top.
    *acc += b.margin.top + b.padding.top;
    for c in &b.children {
        if let Some(y) = find_y_inner(c, target, acc) {
            return Some(y);
        }
    }
    *acc += b.padding.bottom + b.margin.bottom;
    None
}

impl BoxTree {
    /// Estima la y del N-ésimo (1-based) leaf de texto cuyo contenido
    /// contiene `query_lower` (la query debe venir ya lowercased — el
    /// caller suele hacerlo una vez fuera del walk). Usado por la find
    /// bar para auto-scroll al match actual con Enter/Shift+Enter.
    pub fn find_y_of_match(&self, query_lower: &str, nth_1based: usize) -> Option<f32> {
        if query_lower.is_empty() || nth_1based == 0 {
            return None;
        }
        let mut acc = 0.0_f32;
        let mut seen = 0_usize;
        find_match_y_inner(&self.root, query_lower, nth_1based, &mut acc, &mut seen)
    }
}

fn find_match_y_inner(
    b: &BoxNode,
    query: &str,
    target_nth: usize,
    acc: &mut f32,
    seen: &mut usize,
) -> Option<f32> {
    if let Some(text) = &b.text {
        if text.to_lowercase().contains(query) {
            *seen += 1;
            if *seen == target_nth {
                return Some(*acc);
            }
        }
        *acc += b.font_size * b.line_height.unwrap_or(1.4);
        return None;
    }
    *acc += b.margin.top + b.padding.top;
    for c in &b.children {
        if let Some(y) = find_match_y_inner(c, query, target_nth, acc, seen) {
            return Some(y);
        }
    }
    *acc += b.padding.bottom + b.margin.bottom;
    None
}

fn count(b: &BoxNode) -> usize {
    1 + b.children.iter().map(count).sum::<usize>()
}

fn walk_inner(b: &BoxNode, f: &mut impl FnMut(&BoxNode)) {
    f(b);
    for c in &b.children {
        walk_inner(c, f);
    }
}

/// Construye el árbol de boxes desde un DOM y un StyleEngine.
///
/// `base_url` se usa para resolver los `href` de `<a>` a URLs
/// absolutos. Pasale el URL del documento (puede ser `about:blank`
/// para HTML inline).
pub fn build(dom: &DomTree, styles: &StyleEngine, base_url: &str) -> BoxTree {
    // `<base href="...">` en el `<head>` override la base URL. Si está
    // ausente o inválido, fallback al URL del documento.
    let doc_base = url::Url::parse(base_url).ok();
    let base = dom
        .base_href()
        .as_deref()
        .and_then(|href| {
            // El base href puede ser absoluto o relativo al URL del doc.
            url::Url::parse(href)
                .ok()
                .or_else(|| doc_base.as_ref().and_then(|b| b.join(href).ok()))
        })
        .or(doc_base);
    let body = dom.find("body").unwrap_or_else(|| dom.document());
    let mut root = build_node(&body, styles, base.as_ref(), None).unwrap_or_else(empty_root);
    let mut forms: Vec<FormInfo> = Vec::new();
    // Pre-walk del DOM para coleccionar `<form>` (orden DFS) con sus
    // attributes resueltos contra base. La asignación de form_idx por
    // input se hace en un post-pass sobre el box tree con el mismo
    // criterio DFS — ambos walks coinciden porque el box tree refleja
    // el DOM (sólo dropea text-whitespace inter-block; los <form> son
    // block-level y nunca se descartan).
    collect_forms_dom(&body, base.as_ref(), &mut forms);
    let mut form_stack: Vec<usize> = Vec::new();
    let mut form_cursor: usize = 0;
    assign_form_idx(&mut root, &mut form_stack, &mut form_cursor);
    BoxTree { root, forms }
}

fn collect_forms_dom(node: &Handle, base: Option<&url::Url>, out: &mut Vec<FormInfo>) {
    if let markup5ever_rcdom::NodeData::Element { .. } = &node.data {
        if dom::element_name(node).as_deref() == Some("form") {
            let action = dom::attr(node, "action").and_then(|a| resolve_href(base, &a));
            let method = dom::attr(node, "method")
                .map(|m| {
                    if m.eq_ignore_ascii_case("post") {
                        FormMethod::Post
                    } else {
                        FormMethod::Get
                    }
                })
                .unwrap_or(FormMethod::Get);
            out.push(FormInfo { action, method });
        }
    }
    for c in node.children.borrow().iter() {
        collect_forms_dom(c, base, out);
    }
}

fn assign_form_idx(b: &mut BoxNode, stack: &mut Vec<usize>, cursor: &mut usize) {
    let is_form = b.tag.as_deref() == Some("form");
    if is_form {
        stack.push(*cursor);
        *cursor += 1;
    }
    if b.input_kind.is_some() || b.select.is_some() {
        b.form_idx = stack.last().copied();
    }
    for c in &mut b.children {
        assign_form_idx(c, stack, cursor);
    }
    if is_form {
        stack.pop();
    }
}

/// Recolecta primitivas de un `<svg>`: rect/circle/line directos.
/// Soporta atributos `viewBox`, `width`, `height`, `fill`, `stroke`,
/// `stroke-width`. Sin transforms ni groups recursivos.
fn collect_svg(svg_node: &Handle) -> SvgScene {
    let width = dom::attr(svg_node, "width")
        .and_then(|s| s.trim_end_matches("px").trim().parse::<f32>().ok())
        .unwrap_or(300.0);
    let height = dom::attr(svg_node, "height")
        .and_then(|s| s.trim_end_matches("px").trim().parse::<f32>().ok())
        .unwrap_or(150.0);
    let view_box = dom::attr(svg_node, "viewBox").and_then(|s| {
        let nums: Vec<f32> = s
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse::<f32>().ok())
            .collect();
        if nums.len() == 4 {
            Some((nums[0], nums[1], nums[2], nums[3]))
        } else {
            None
        }
    });
    let mut prims: Vec<SvgPrim> = Vec::new();
    collect_svg_prims(svg_node, &mut prims);
    SvgScene { width, height, view_box, prims }
}

fn collect_svg_prims(node: &Handle, out: &mut Vec<SvgPrim>) {
    if let markup5ever_rcdom::NodeData::Element { .. } = &node.data {
        match dom::element_name(node).as_deref() {
            Some("rect") => {
                let x = svg_num(node, "x", 0.0);
                let y = svg_num(node, "y", 0.0);
                let w = svg_num(node, "width", 0.0);
                let h = svg_num(node, "height", 0.0);
                let rx = svg_num(node, "rx", 0.0);
                out.push(SvgPrim::Rect {
                    x, y, w, h, rx,
                    fill: svg_color(node, "fill"),
                    stroke: svg_color(node, "stroke"),
                    stroke_w: svg_num(node, "stroke-width", 1.0),
                });
            }
            Some("circle") => {
                let cx = svg_num(node, "cx", 0.0);
                let cy = svg_num(node, "cy", 0.0);
                let r = svg_num(node, "r", 0.0);
                out.push(SvgPrim::Circle {
                    cx, cy, r,
                    fill: svg_color(node, "fill"),
                    stroke: svg_color(node, "stroke"),
                    stroke_w: svg_num(node, "stroke-width", 1.0),
                });
            }
            Some("line") => {
                let x1 = svg_num(node, "x1", 0.0);
                let y1 = svg_num(node, "y1", 0.0);
                let x2 = svg_num(node, "x2", 0.0);
                let y2 = svg_num(node, "y2", 0.0);
                if let Some(stroke) = svg_color(node, "stroke") {
                    out.push(SvgPrim::Line {
                        x1, y1, x2, y2,
                        stroke,
                        stroke_w: svg_num(node, "stroke-width", 1.0),
                    });
                }
            }
            Some("polygon") | Some("polyline") => {
                let closed = dom::element_name(node).as_deref() == Some("polygon");
                let points = parse_svg_points(&dom::attr(node, "points").unwrap_or_default());
                if !points.is_empty() {
                    out.push(SvgPrim::Polyline {
                        points,
                        closed,
                        fill: svg_color(node, "fill"),
                        stroke: svg_color(node, "stroke"),
                        stroke_w: svg_num(node, "stroke-width", 1.0),
                    });
                }
            }
            Some("path") => {
                if let Some(d) = dom::attr(node, "d") {
                    let cmds = parse_svg_path(&d);
                    if !cmds.is_empty() {
                        out.push(SvgPrim::Path {
                            d: cmds,
                            fill: svg_color(node, "fill"),
                            stroke: svg_color(node, "stroke"),
                            stroke_w: svg_num(node, "stroke-width", 1.0),
                        });
                    }
                }
            }
            // Containers transparentes: recurrir adentro.
            Some("g") | Some("svg") => {}
            // Resto (`text`, `defs`, `mask`, etc.) ignorado.
            _ => return,
        }
    }
    for c in node.children.borrow().iter() {
        collect_svg_prims(c, out);
    }
}

fn svg_num(node: &Handle, name: &str, default: f32) -> f32 {
    dom::attr(node, name)
        .and_then(|s| s.trim_end_matches("px").trim().parse::<f32>().ok())
        .unwrap_or(default)
}

/// Elige una URL del `srcset` HTML. Subset: cada candidato es `url
/// [descriptor]` separados por `,`. Descriptor puede ser `Nx`
/// (densidad) o `Nw` (ancho) o ausente. Estrategia: preferimos la
/// más alta densidad (`Nx`) o el ancho más grande (`Nw`); sin
/// viewport conocido al tiempo de parse, asumimos high-DPI por default.
pub(crate) fn pick_srcset(srcset: &str) -> Option<String> {
    if srcset.trim().is_empty() {
        return None;
    }
    let mut best_score: f32 = -1.0;
    let mut best_url: Option<String> = None;
    for entry in srcset.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (url, desc) = match entry.split_once(char::is_whitespace) {
            Some((u, d)) => (u.trim().to_string(), d.trim().to_string()),
            None => (entry.to_string(), String::new()),
        };
        let score: f32 = if let Some(rest) = desc.strip_suffix('x') {
            rest.parse::<f32>().unwrap_or(1.0) * 1000.0
        } else if let Some(rest) = desc.strip_suffix('w') {
            rest.parse::<f32>().unwrap_or(0.0)
        } else {
            // Sin descriptor — equivalente a 1x.
            1000.0
        };
        if score > best_score {
            best_score = score;
            best_url = Some(url);
        }
    }
    best_url
}

fn parse_svg_points(s: &str) -> Vec<(f32, f32)> {
    let nums: Vec<f32> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse::<f32>().ok())
        .collect();
    nums.chunks_exact(2).map(|c| (c[0], c[1])).collect()
}

/// Parser de `d=` minimal: soporta M/m, L/l, H/h, V/v, C/c, Q/q, Z/z.
/// No soporta A (arcs), T, S (smooth bezier).
fn parse_svg_path(d: &str) -> Vec<PathCmd> {
    // Tokenize: cada comando es una letra, cada arg es un f32 (separados
    // por whitespace o coma; el signo `-` puede arrancar un nuevo número
    // sin separador).
    let bytes = d.as_bytes();
    let mut i = 0;
    let n = bytes.len();
    let mut out: Vec<PathCmd> = Vec::new();
    let mut cx = 0.0_f32; // cursor x absoluto
    let mut cy = 0.0_f32;
    let mut start_x = 0.0_f32;
    let mut start_y = 0.0_f32;
    let mut current_cmd: u8 = 0;
    while i < n {
        let c = bytes[i];
        if c.is_ascii_whitespace() || c == b',' {
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() {
            current_cmd = c;
            i += 1;
            // Z/z no toma args — ejecutalo acá directamente, sino el
            // loop nunca llega al match (no hay número que dispare).
            if c == b'Z' || c == b'z' {
                out.push(PathCmd::ClosePath);
                cx = start_x;
                cy = start_y;
            }
            continue;
        }
        // c es dígito o `-`/`+`/`.`: leer un número.
        let read_num = |from: usize| -> Option<(f32, usize)> {
            let mut j = from;
            if j < n && (bytes[j] == b'-' || bytes[j] == b'+') {
                j += 1;
            }
            while j < n && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            if j < n && (bytes[j] == b'e' || bytes[j] == b'E') {
                j += 1;
                if j < n && (bytes[j] == b'-' || bytes[j] == b'+') {
                    j += 1;
                }
                while j < n && bytes[j].is_ascii_digit() {
                    j += 1;
                }
            }
            std::str::from_utf8(&bytes[from..j])
                .ok()
                .and_then(|s| s.parse::<f32>().ok())
                .map(|v| (v, j))
        };
        let read_args = |from: usize, count: usize| -> Option<(Vec<f32>, usize)> {
            let mut nums = Vec::with_capacity(count);
            let mut k = from;
            while nums.len() < count {
                while k < n && (bytes[k].is_ascii_whitespace() || bytes[k] == b',') {
                    k += 1;
                }
                let (v, after) = read_num(k)?;
                nums.push(v);
                k = after;
            }
            Some((nums, k))
        };
        let rel = current_cmd.is_ascii_lowercase();
        match current_cmd.to_ascii_uppercase() {
            b'M' => {
                let (args, after) = match read_args(i, 2) {
                    Some(v) => v,
                    None => break,
                };
                let (mut x, mut y) = (args[0], args[1]);
                if rel { x += cx; y += cy; }
                out.push(PathCmd::MoveTo(x, y));
                cx = x; cy = y;
                start_x = x; start_y = y;
                i = after;
                // M con args extra implícitamente lineTo.
                current_cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let (args, after) = match read_args(i, 2) {
                    Some(v) => v,
                    None => break,
                };
                let (mut x, mut y) = (args[0], args[1]);
                if rel { x += cx; y += cy; }
                out.push(PathCmd::LineTo(x, y));
                cx = x; cy = y;
                i = after;
            }
            b'H' => {
                let (args, after) = match read_args(i, 1) {
                    Some(v) => v,
                    None => break,
                };
                let mut x = args[0];
                if rel { x += cx; }
                out.push(PathCmd::LineTo(x, cy));
                cx = x;
                i = after;
            }
            b'V' => {
                let (args, after) = match read_args(i, 1) {
                    Some(v) => v,
                    None => break,
                };
                let mut y = args[0];
                if rel { y += cy; }
                out.push(PathCmd::LineTo(cx, y));
                cy = y;
                i = after;
            }
            b'C' => {
                let (args, after) = match read_args(i, 6) {
                    Some(v) => v,
                    None => break,
                };
                let (mut x1, mut y1, mut x2, mut y2, mut x, mut y) =
                    (args[0], args[1], args[2], args[3], args[4], args[5]);
                if rel {
                    x1 += cx; y1 += cy;
                    x2 += cx; y2 += cy;
                    x += cx; y += cy;
                }
                out.push(PathCmd::CubicTo(x1, y1, x2, y2, x, y));
                cx = x; cy = y;
                i = after;
            }
            b'Q' => {
                let (args, after) = match read_args(i, 4) {
                    Some(v) => v,
                    None => break,
                };
                let (mut x1, mut y1, mut x, mut y) = (args[0], args[1], args[2], args[3]);
                if rel {
                    x1 += cx; y1 += cy;
                    x += cx; y += cy;
                }
                out.push(PathCmd::QuadTo(x1, y1, x, y));
                cx = x; cy = y;
                i = after;
            }
            b'Z' => {
                out.push(PathCmd::ClosePath);
                cx = start_x;
                cy = start_y;
            }
            _ => {
                // Comando no soportado (`A`, `T`, `S`) — saltea un número
                // para evitar loops infinitos.
                if let Some((_, after)) = read_num(i) {
                    i = after;
                } else {
                    break;
                }
            }
        }
    }
    out
}

fn svg_color(node: &Handle, name: &str) -> Option<Color> {
    let v = dom::attr(node, name)?;
    let v = v.trim();
    if v.eq_ignore_ascii_case("none") {
        return None;
    }
    crate::style::parse_color_named_or_hex(v)
}

fn empty_root() -> BoxNode {
    BoxNode {
        display: Display::Block,
        background: None,
        color: Color::BLACK,
        font_size: 16.0,
        font_weight: 400,
        font_style: crate::style::FontStyle::Normal,
        font_family: None,
        margin: Sides::all(0.0),
        padding: Sides::all(0.0),
        width: LengthVal::Auto,
        max_width: LengthVal::Auto,
        text_align: TextAlign::Left,
        line_height: None,
        border_widths: Sides::all(0.0),
        border_colors: Sides::all(None),
        border_radii: Corners::all(0.0),
        hover_background: None,
        focus_background: None,
        box_shadow: None,
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::Start,
        align_items: AlignItems::Stretch,
        flex_wrap: FlexWrap::NoWrap,
        gap_row: 0.0,
        gap_column: 0.0,
        box_sizing: BoxSizing::ContentBox,
        min_width: LengthVal::Auto,
        min_height: LengthVal::Auto,
        max_height: LengthVal::Auto,
        overflow: Overflow::Visible,
        white_space: WhiteSpace::Normal,
        text_transform: TextTransform::None,
        opacity: 1.0,
        align_self: AlignSelf::Auto,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        flex_basis: LengthVal::Auto,
        outline: Outline::default(),
        background_gradient: None,
        position: Position::Static,
        inset_top: LengthVal::Auto,
        inset_right: LengthVal::Auto,
        inset_bottom: LengthVal::Auto,
        inset_left: LengthVal::Auto,
        vertical_align: VerticalAlign::Baseline,
        visibility: Visibility::Visible,
        pointer_events: PointerEvents::Auto,
        text_indent: 0.0,
        word_spacing: 0.0,
        text_shadows: Vec::new(),
        transforms: Vec::new(),
        grid_template_columns: Vec::new(),
        grid_template_rows: Vec::new(),
        text_decoration: TextDecorationLine::None,
        text: None,
        children: Vec::new(),
        tag: Some("body".into()),
        link: None,
        image: None,
        details_open_attr: false,
        link_new_tab: false,
        background_image: None,
        input_kind: None,
        input_initial: None,
        input_placeholder: None,
        input_name: None,
        input_checked_initial: false,
        input_autofocus: false,
        form_idx: None,
        select: None,
        svg: None,
        element_id: None,
    }
}

fn build_node(
    node: &Handle,
    styles: &StyleEngine,
    base: Option<&url::Url>,
    parent_style: Option<&ComputedStyle>,
) -> Option<BoxNode> {
    match &node.data {
        NodeData::Element { .. } => {
            let style = styles.compute_with_parent(node, parent_style);
            if style.display == Display::None {
                return None;
            }
            // Hover/focus styles: recomputamos con hover_active=true y
            // focus_active=true por separado y vemos si alguna pseudoclase
            // `:hover`/`:focus` cambió el background. Si sí, exponemos el
            // delta al chrome para que lo aplique cuando corresponda.
            // Resto del diff (color/border/etc.) queda fuera por ahora —
            // restyle completo requeriría re-mount del tree.
            let hover_style = styles.compute_with_parent_in_state(node, parent_style, true);
            let hover_background = if hover_style.background != style.background {
                hover_style.background
            } else {
                None
            };
            let focus_style =
                styles.compute_with_parent_for_state(node, parent_style, false, true);
            let focus_background = if focus_style.background != style.background {
                focus_style.background
            } else {
                None
            };
            let tag = dom::element_name(node);
            let link = match (tag.as_deref(), base) {
                (Some("a"), base) => dom::attr(node, "href").and_then(|h| resolve_href(base, &h)),
                _ => None,
            };
            let link_new_tab = tag.as_deref() == Some("a")
                && dom::attr(node, "target")
                    .map(|t| {
                        let t = t.trim().to_ascii_lowercase();
                        // `_blank` y cualquier target con nombre custom → nueva tab.
                        // `_self`/`_parent`/`_top` quedan como navegación in-place.
                        !t.is_empty() && t != "_self" && t != "_parent" && t != "_top"
                    })
                    .unwrap_or(false);

            let input_kind = match tag.as_deref() {
                Some("textarea") => Some(InputKind::TextArea),
                Some("input") => {
                    let t = dom::attr(node, "type")
                        .map(|s| s.trim().to_ascii_lowercase())
                        .unwrap_or_else(|| "text".to_string());
                    match t.as_str() {
                        "" | "text" | "email" | "url" | "tel" | "number" => Some(InputKind::Text),
                        "search" => Some(InputKind::Search),
                        "password" => Some(InputKind::Password),
                        "checkbox" => Some(InputKind::Checkbox),
                        "radio" => Some(InputKind::Radio),
                        "submit" | "button" | "reset" => Some(InputKind::Submit),
                        _ => None, // file, range, color, hidden, etc.
                    }
                }
                _ => None,
            };
            let input_initial = input_kind.and_then(|_| {
                if tag.as_deref() == Some("textarea") {
                    // El "value" del textarea es su texto interior.
                    let mut s = String::new();
                    for child in node.children.borrow().iter() {
                        if let markup5ever_rcdom::NodeData::Text { contents } = &child.data {
                            s.push_str(&contents.borrow());
                        }
                    }
                    Some(s)
                } else {
                    dom::attr(node, "value")
                }
            });
            let input_placeholder = input_kind.and_then(|_| dom::attr(node, "placeholder"));
            let input_name = input_kind.and_then(|_| dom::attr(node, "name"));
            let input_checked_initial = matches!(
                input_kind,
                Some(InputKind::Checkbox) | Some(InputKind::Radio)
            ) && dom::attr(node, "checked").is_some();
            let input_autofocus = input_kind.is_some() && dom::attr(node, "autofocus").is_some();
            // `<svg>`: coleccionamos las primitivas (rect/circle/line) y
            // el viewBox. Las primitivas del subárbol del SVG no son
            // descendientes del box tree (el `display: inline-block` del
            // `<svg>` mantiene su rect pero los hijos quedan fuera del
            // flow). El chrome usa `b.svg` para paint_with.
            let svg = if tag.as_deref() == Some("svg") {
                Some(collect_svg(node))
            } else {
                None
            };
            // `<select>`: coleccionamos opciones y el inicial seleccionado.
            let select = if tag.as_deref() == Some("select") {
                let mut opts: Vec<SelectOption> = Vec::new();
                let mut initial = 0usize;
                let mut seen_selected = false;
                for child in node.children.borrow().iter() {
                    if dom::element_name(child).as_deref() == Some("option") {
                        let label = dom::collect_text(child);
                        let value = dom::attr(child, "value").unwrap_or_else(|| label.clone());
                        if dom::attr(child, "selected").is_some() && !seen_selected {
                            initial = opts.len();
                            seen_selected = true;
                        }
                        opts.push(SelectOption { label, value });
                    }
                }
                if opts.is_empty() {
                    None
                } else {
                    Some(SelectInfo { options: opts, initial })
                }
            } else {
                None
            };
            // <img>: descarga + decode sync. Si falla, el campo queda
            // None y el chrome muestra placeholder con el alt. Resuelve
            // `srcset` antes que `src` (responsive images).
            let image = if tag.as_deref() == Some("img") {
                let src_candidate = pick_srcset(&dom::attr(node, "srcset").unwrap_or_default())
                    .or_else(|| dom::attr(node, "src"));
                src_candidate
                    .and_then(|s| resolve_href(base, &s))
                    .and_then(|abs| fetch_and_decode(&abs))
            } else if tag.as_deref() == Some("picture") {
                // `<picture>`: el primer `<source srcset>` que sirva
                // gana; sino caemos al `<img>` interno (que ya entra
                // como child y trae su src/srcset).
                let mut chosen: Option<String> = None;
                for child in node.children.borrow().iter() {
                    if dom::element_name(child).as_deref() == Some("source") {
                        if let Some(s) = dom::attr(child, "srcset") {
                            if let Some(c) = pick_srcset(&s) {
                                chosen = Some(c);
                                break;
                            }
                        }
                    }
                }
                chosen
                    .and_then(|s| resolve_href(base, &s))
                    .and_then(|abs| fetch_and_decode(&abs))
            } else {
                None
            };
            // `background-image: url(...)` — resolver contra base y
            // descargar/decode. Misma cache que `<img>` por la fetch::
            // global. Falla silenciosa → background_image queda None.
            let background_image = style
                .background_image_url
                .as_deref()
                .and_then(|u| resolve_href(base, u))
                .and_then(|abs| fetch_and_decode(&abs));
            let mut children = Vec::new();
            // <li>: prefija con marker (bullet o numeral según
            // `list-style-type`). Lo agregamos como un hijo Text inline
            // antes de procesar los hijos reales — hereda
            // color/font-size de `style`. Si `list-style-type: none` o
            // no estamos dentro de una lista reconocible, no se inyecta
            // marker.
            if tag.as_deref() == Some("li") {
                if let Some(marker) = li_marker(node, style.list_style_type) {
                    children.push(inline_text_with_style(marker, &style));
                }
            }
            // <img> sin imagen decodificada: muestra `alt`.
            if tag.as_deref() == Some("img") && image.is_none() {
                if let Some(alt) = dom::attr(node, "alt") {
                    if !alt.trim().is_empty() {
                        children.push(inline_text_with_style(format!("[img: {alt}]"), &style));
                    }
                }
            }
            for child in node.children.borrow().iter() {
                if let Some(b) = build_node(child, styles, base, Some(&style)) {
                    children.push(b);
                }
            }
            let children = strip_block_adjacent_whitespace(children, style.display);
            Some(BoxNode {
                display: style.display,
                background: style.background,
                color: style.color,
                font_size: style.font_size,
                font_weight: style.font_weight,
                font_style: style.font_style,
                font_family: style.font_family.clone(),
                margin: style.margin,
                padding: style.padding,
                width: style.width,
                max_width: style.max_width,
                text_align: style.text_align,
                line_height: style.line_height,
                border_widths: style.border_widths,
                border_colors: style.border_colors,
                border_radii: style.border_radii,
                hover_background,
                focus_background,
                box_shadow: style.box_shadow,
                flex_direction: style.flex_direction,
                justify_content: style.justify_content,
                align_items: style.align_items,
                flex_wrap: style.flex_wrap,
                gap_row: style.gap_row,
                gap_column: style.gap_column,
                box_sizing: style.box_sizing,
                min_width: style.min_width,
                min_height: style.min_height,
                max_height: style.max_height,
                overflow: style.overflow,
                white_space: style.white_space,
                text_transform: style.text_transform,
                opacity: style.opacity,
                align_self: style.align_self,
                flex_grow: style.flex_grow,
                flex_shrink: style.flex_shrink,
                flex_basis: style.flex_basis,
                outline: style.outline,
                background_gradient: style.background_gradient.clone(),
                position: style.position,
                inset_top: style.inset_top,
                inset_right: style.inset_right,
                inset_bottom: style.inset_bottom,
                inset_left: style.inset_left,
                vertical_align: style.vertical_align,
                visibility: style.visibility,
                pointer_events: style.pointer_events,
                text_indent: style.text_indent,
                word_spacing: style.word_spacing,
                text_shadows: style.text_shadows.clone(),
                transforms: style.transforms.clone(),
                grid_template_columns: style.grid_template_columns.clone(),
                grid_template_rows: style.grid_template_rows.clone(),
                text_decoration: style.text_decoration,
                text: None,
                children,
                tag: tag.clone(),
                link,
                image,
                details_open_attr: tag.as_deref() == Some("details")
                    && dom::attr(node, "open").is_some(),
                link_new_tab,
                background_image,
                input_kind,
                input_initial,
                input_placeholder,
                input_name: input_name.or_else(|| {
                    // `<select>` también necesita un `name` para submitear.
                    if tag.as_deref() == Some("select") {
                        dom::attr(node, "name")
                    } else {
                        None
                    }
                }),
                input_checked_initial,
                input_autofocus,
                form_idx: None,
                select,
                svg,
                element_id: dom::attr(node, "id").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            })
        }
        NodeData::Text { contents } => {
            let raw = contents.borrow().to_string();
            // CSS whitespace collapse: colapsa runs internos a un solo
            // espacio, preserva un espacio al inicio o fin si lo había
            // (caso clásico: `foo <a>bar</a> baz` debe rendear "foo bar
            // baz" — sin el espacio adyacente al link los tokens se
            // pegan al renderizarse en views vecinas).
            let parent = parent_style.unwrap_or(&ComputedStyle::default()).clone();
            let collapsed = collapse_whitespace(&raw, parent.white_space);
            let collapsed = apply_text_transform(collapsed, parent.text_transform);
            if collapsed.is_empty() {
                return None;
            }
            // El leaf de texto hereda las propiedades inheritables del
            // padre (color, font-size, font-weight, text-align,
            // line-height). Sin esto, todo texto sale negro 16px aunque
            // el `<p>` padre indique color rojo.
            Some(inline_text_with_style(collapsed, &parent))
        }
        _ => {
            // Document / Doctype / Comment → recurrir sólo en hijos. El
            // wrapper que producimos abajo es siempre `Display::Block`, así
            // que filtramos con ese display.
            let mut children = Vec::new();
            for child in node.children.borrow().iter() {
                if let Some(b) = build_node(child, styles, base, parent_style) {
                    children.push(b);
                }
            }
            let children = strip_block_adjacent_whitespace(children, Display::Block);
            if children.is_empty() {
                return None;
            }
            // Wrapeamos los hijos en un block transparente para no
            // perder la jerarquía. Heredamos lo del padre si lo hay.
            let p = parent_style.cloned().unwrap_or_default();
            Some(BoxNode {
                display: Display::Block,
                background: None,
                color: p.color,
                font_size: p.font_size,
                font_weight: p.font_weight,
                font_style: p.font_style,
                font_family: p.font_family.clone(),
                margin: Sides::all(0.0),
                padding: Sides::all(0.0),
                width: LengthVal::Auto,
                max_width: LengthVal::Auto,
                text_align: p.text_align,
                line_height: p.line_height,
                border_widths: Sides::all(0.0),
                border_colors: Sides::all(None),
                border_radii: Corners::all(0.0),
                hover_background: None,
        focus_background: None,
                box_shadow: None,
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::Start,
                align_items: AlignItems::Stretch,
                flex_wrap: FlexWrap::NoWrap,
                gap_row: 0.0,
                gap_column: 0.0,
                box_sizing: BoxSizing::ContentBox,
                min_width: LengthVal::Auto,
                min_height: LengthVal::Auto,
                max_height: LengthVal::Auto,
                overflow: Overflow::Visible,
                white_space: WhiteSpace::Normal,
                text_transform: TextTransform::None,
                opacity: 1.0,
                align_self: AlignSelf::Auto,
                flex_grow: 0.0,
                flex_shrink: 1.0,
                flex_basis: LengthVal::Auto,
                outline: Outline::default(),
                background_gradient: None,
                position: Position::Static,
                inset_top: LengthVal::Auto,
                inset_right: LengthVal::Auto,
                inset_bottom: LengthVal::Auto,
                inset_left: LengthVal::Auto,
                vertical_align: VerticalAlign::Baseline,
                visibility: Visibility::Visible,
                pointer_events: PointerEvents::Auto,
                text_indent: 0.0,
                word_spacing: 0.0,
                text_shadows: Vec::new(),
                transforms: Vec::new(),
                grid_template_columns: Vec::new(),
                grid_template_rows: Vec::new(),
                text_decoration: p.text_decoration,
                text: None,
                children,
                tag: None,
                link: None,
                image: None,
                details_open_attr: false,
                link_new_tab: false,
                background_image: None,
                input_kind: None,
                input_initial: None,
                input_placeholder: None,
        input_name: None,
        input_checked_initial: false,
        input_autofocus: false,
        form_idx: None,
        select: None,
        svg: None,
        element_id: None,
            })
        }
    }
}

/// Construye un nodo Text inline con el color/font/text-align/line-height
/// del estilo dado — usado tanto por hojas Text reales como por los
/// markers sintéticos (`•` de `<li>`, `[img: alt]` de `<img>` roto).
fn inline_text_with_style(s: String, style: &ComputedStyle) -> BoxNode {
    BoxNode {
        display: Display::Inline,
        background: None,
        color: style.color,
        font_size: style.font_size,
        font_weight: style.font_weight,
        font_style: style.font_style,
        font_family: style.font_family.clone(),
        margin: Sides::all(0.0),
        padding: Sides::all(0.0),
        width: LengthVal::Auto,
        max_width: LengthVal::Auto,
        text_align: style.text_align,
        line_height: style.line_height,
        border_widths: Sides::all(0.0),
        border_colors: Sides::all(None),
        border_radii: Corners::all(0.0),
        hover_background: None,
        focus_background: None,
        box_shadow: None,
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::Start,
        align_items: AlignItems::Stretch,
        flex_wrap: FlexWrap::NoWrap,
        gap_row: 0.0,
        gap_column: 0.0,
        box_sizing: BoxSizing::ContentBox,
        min_width: LengthVal::Auto,
        min_height: LengthVal::Auto,
        max_height: LengthVal::Auto,
        overflow: Overflow::Visible,
        white_space: WhiteSpace::Normal,
        text_transform: TextTransform::None,
        opacity: 1.0,
        align_self: AlignSelf::Auto,
        flex_grow: 0.0,
        flex_shrink: 1.0,
        flex_basis: LengthVal::Auto,
        outline: Outline::default(),
        background_gradient: None,
        position: Position::Static,
        inset_top: LengthVal::Auto,
        inset_right: LengthVal::Auto,
        inset_bottom: LengthVal::Auto,
        inset_left: LengthVal::Auto,
        vertical_align: VerticalAlign::Baseline,
        visibility: Visibility::Visible,
        pointer_events: PointerEvents::Auto,
        text_indent: 0.0,
        word_spacing: 0.0,
        text_shadows: Vec::new(),
        transforms: Vec::new(),
        grid_template_columns: Vec::new(),
        grid_template_rows: Vec::new(),
        text_decoration: style.text_decoration,
        text: Some(s),
        children: Vec::new(),
        tag: None,
        link: None,
        image: None,
        details_open_attr: false,
        link_new_tab: false,
        background_image: None,
        input_kind: None,
        input_initial: None,
        input_placeholder: None,
        input_name: None,
        input_checked_initial: false,
        input_autofocus: false,
        form_idx: None,
        select: None,
        svg: None,
        element_id: None,
    }
}

/// `true` si el nodo se comporta como block-level para el flujo (Block,
/// Flex, Grid, None). `Inline*` queda fuera — son del flow inline.
fn is_block_level(b: &BoxNode) -> bool {
    !matches!(
        b.display,
        Display::Inline | Display::InlineBlock | Display::InlineFlex | Display::InlineGrid
    )
}

/// `true` si el nodo es un leaf de texto inline cuyo contenido se reduce
/// a whitespace (incluye el caso post-collapse del CSS, que deja " "
/// como "espacio entre tokens"). `<br>` y otros inlines sin texto no
/// matchean (b.text es None).
fn is_ws_only_inline(b: &BoxNode) -> bool {
    matches!(b.display, Display::Inline | Display::InlineBlock)
        && b
            .text
            .as_ref()
            .map(|s| !s.is_empty() && s.chars().all(|c| c.is_whitespace()))
            .unwrap_or(false)
}

/// Quita los text-nodes whitespace-only que separan block siblings o
/// quedan adyacentes al borde de un block. Replica el comportamiento
/// estándar de los browsers: en HTML, el `\n  ` entre `</p>\n  <h2>`
/// produce un Text node " " que NO debe rendear (sino cada tag aporta
/// una línea visible vacía). Se preserva si está rodeado de inlines
/// (ahí sí lleva valor: separa tokens).
fn strip_block_adjacent_whitespace(
    children: Vec<BoxNode>,
    parent_display: Display,
) -> Vec<BoxNode> {
    // Cuando el padre es Inline (`<span>`, `<em>`, etc.) los hijos viven
    // en el inline-flow del *abuelo* block; los whitespace que tengan
    // dentro pueden ser parte de un token relevante ("foo<span> </span>
    // bar" debe mantener los dos espacios). No filtramos a este nivel —
    // el filtrado real ocurre cuando el padre sí establece un contexto
    // block (Block/Flex/Grid/InlineBlock/etc.).
    if matches!(parent_display, Display::Inline) {
        return children;
    }
    if children.iter().all(|c| !is_ws_only_inline(c)) {
        return children;
    }
    let block_levels: Vec<bool> = children.iter().map(is_block_level).collect();
    let n = children.len();
    let mut out = Vec::with_capacity(n);
    for (i, c) in children.into_iter().enumerate() {
        if is_ws_only_inline(&c) {
            // Si el vecino previo (o el "borde" si i=0) es block-level,
            // y el siguiente también (o no existe), drop. Si hay un
            // inline real a cualquier lado, mantenemos el espacio.
            let prev_is_block_or_edge = i == 0 || block_levels[i - 1];
            let next_is_block_or_edge = i + 1 >= n || block_levels[i + 1];
            if prev_is_block_or_edge && next_is_block_or_edge {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Descarga `url` y la decodifica a RGBA8. Devuelve `None` si la URL no
/// es HTTP(S), si la descarga falla, si el MIME no es imagen, o si el
/// decoder no soporta el formato. Sync: bloquea el thread caller — el
/// chrome ya está en un worker thread durante `Engine::load`. Pasa por
/// la cache global de bytes — recargas y navegación entre tabs no
/// re-descargan.
fn fetch_and_decode(url: &str) -> Option<ImageData> {
    let bytes = crate::fetch::fetch_bytes(url).ok()?;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.format()?; // formato no habilitado por features → None
    let img = reader.decode().ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    Some(ImageData { rgba: rgba.into_raw(), width, height })
}

/// Colapso de whitespace según `white-space`:
/// - `Normal` / `NoWrap`: runs internos → un espacio, leading/trailing
///   reducidos a uno; newlines colapsan igual.
/// - `Pre`: todo preservado.
/// - `PreWrap`: igual que Pre — el wrap es responsabilidad del layout.
/// - `PreLine`: runs de espacio/tab → un espacio, newlines preservados.
fn collapse_whitespace(s: &str, ws: WhiteSpace) -> String {
    match ws {
        WhiteSpace::Pre | WhiteSpace::PreWrap => s.to_string(),
        WhiteSpace::PreLine => {
            // Colapsa espacios/tabs (no '\n') a uno solo, preserva newlines.
            let mut out = String::with_capacity(s.len());
            let mut prev_space = false;
            for c in s.chars() {
                if c == '\n' {
                    out.push(c);
                    prev_space = false;
                } else if c.is_whitespace() {
                    if !prev_space {
                        out.push(' ');
                        prev_space = true;
                    }
                } else {
                    out.push(c);
                    prev_space = false;
                }
            }
            out
        }
        WhiteSpace::Normal | WhiteSpace::NoWrap => {
            let leading = s.chars().next().is_some_and(|c| c.is_whitespace());
            let trailing = s.chars().last().is_some_and(|c| c.is_whitespace());
            let core: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
            if core.is_empty() {
                // Sólo whitespace: lo dejamos como " " para no perder el
                // separador entre inlines vecinos.
                return if leading || trailing { " ".to_string() } else { String::new() };
            }
            let mut out = String::with_capacity(core.len() + 2);
            if leading {
                out.push(' ');
            }
            out.push_str(&core);
            if trailing {
                out.push(' ');
            }
            out
        }
    }
}

/// Aplica `text-transform` al texto. Capitalize convierte la primera
/// letra de cada palabra (separada por whitespace) a mayúscula.
fn apply_text_transform(s: String, t: TextTransform) -> String {
    match t {
        TextTransform::None => s,
        TextTransform::Uppercase => s.to_uppercase(),
        TextTransform::Lowercase => s.to_lowercase(),
        TextTransform::Capitalize => {
            let mut out = String::with_capacity(s.len());
            let mut start_of_word = true;
            for c in s.chars() {
                if c.is_whitespace() {
                    out.push(c);
                    start_of_word = true;
                } else if start_of_word {
                    out.extend(c.to_uppercase());
                    start_of_word = false;
                } else {
                    out.push(c);
                }
            }
            out
        }
    }
}

/// Construye el texto del marker de un `<li>`. Para tipos numerados
/// (`decimal`/`*-alpha`/`*-roman`) calcula la posición del item entre sus
/// hermanos `<li>` del mismo padre, respetando `<ol start>` y
/// `<li value>`. Devuelve `None` si `list-style-type: none`.
///
/// Marcadores con número usan `"N. "` (período + un espacio) — alineado
/// con el comportamiento de browsers. Marcadores con símbolo usan
/// `"<sym>  "` (doble espacio) para dar el airecito que tenía el bullet
/// hardcoded original.
fn li_marker(node: &Handle, kind: ListStyleType) -> Option<String> {
    match kind {
        ListStyleType::None => None,
        ListStyleType::Disc => Some("•  ".into()),
        ListStyleType::Circle => Some("◦  ".into()),
        ListStyleType::Square => Some("▪  ".into()),
        ListStyleType::Decimal => Some(format!("{}. ", ol_item_position(node))),
        ListStyleType::LowerAlpha => {
            Some(format!("{}. ", to_alpha(ol_item_position(node), false)))
        }
        ListStyleType::UpperAlpha => {
            Some(format!("{}. ", to_alpha(ol_item_position(node), true)))
        }
        ListStyleType::LowerRoman => {
            Some(format!("{}. ", to_roman(ol_item_position(node), false)))
        }
        ListStyleType::UpperRoman => {
            Some(format!("{}. ", to_roman(ol_item_position(node), true)))
        }
    }
}

/// Posición 1-indexed del `<li>` entre sus hermanos `<li>` del padre.
/// Respeta `<ol start="N">` (arranca el contador en N) y `<li value="N">`
/// (resetea el contador al valor dado para ese item y los siguientes).
/// Si `node` no es un `<li>` o no tiene padre, devuelve 1.
fn ol_item_position(node: &Handle) -> i32 {
    let Some(parent) = parent_handle(node) else { return 1 };
    let parent_is_ol = dom::element_name(&parent).as_deref() == Some("ol");
    let mut counter: i32 = if parent_is_ol {
        dom::attr(&parent, "start").and_then(|s| s.trim().parse().ok()).unwrap_or(1)
    } else {
        1
    };
    for child in parent.children.borrow().iter() {
        if dom::element_name(child).as_deref() != Some("li") {
            continue;
        }
        if let Some(v) = dom::attr(child, "value").and_then(|s| s.trim().parse::<i32>().ok()) {
            counter = v;
        }
        if std::rc::Rc::ptr_eq(child, node) {
            return counter;
        }
        counter += 1;
    }
    counter
}

/// Misma idea que `style::parent_of`. Lo duplicamos acá para no tocar
/// la visibilidad del helper en `style.rs`.
fn parent_handle(node: &Handle) -> Option<Handle> {
    let weak = node.parent.take();
    let restored = weak.clone();
    node.parent.set(restored);
    weak.and_then(|w| w.upgrade())
}

/// Convierte 1..N a alpha bijectiva base-26 (1=a, 26=z, 27=aa, 28=ab…).
/// Valores `<= 0` caen a `"0"` — el marker numérico igual se imprime.
fn to_alpha(mut n: i32, upper: bool) -> String {
    if n <= 0 {
        return n.to_string();
    }
    let mut buf: Vec<u8> = Vec::new();
    while n > 0 {
        n -= 1;
        let d = (n % 26) as u8;
        buf.push(if upper { b'A' + d } else { b'a' + d });
        n /= 26;
    }
    buf.reverse();
    // SAFETY: sólo ASCII A-Z/a-z.
    String::from_utf8(buf).expect("alpha ascii-only")
}

/// Romanos 1..3999. Fuera del rango caemos a decimal — matchea el
/// comportamiento de browsers (Chromium también).
fn to_roman(n: i32, upper: bool) -> String {
    if !(1..=3999).contains(&n) {
        return n.to_string();
    }
    const VALUES: &[(i32, &str, &str)] = &[
        (1000, "M", "m"),
        (900, "CM", "cm"),
        (500, "D", "d"),
        (400, "CD", "cd"),
        (100, "C", "c"),
        (90, "XC", "xc"),
        (50, "L", "l"),
        (40, "XL", "xl"),
        (10, "X", "x"),
        (9, "IX", "ix"),
        (5, "V", "v"),
        (4, "IV", "iv"),
        (1, "I", "i"),
    ];
    let mut n = n;
    let mut out = String::new();
    for (val, up, lo) in VALUES {
        while n >= *val {
            out.push_str(if upper { up } else { lo });
            n -= val;
        }
    }
    out
}

fn resolve_href(base: Option<&url::Url>, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    // Schemes que NO son web: el chrome no debería intentar navegar a ellos.
    let lc = href.to_ascii_lowercase();
    if lc.starts_with("javascript:")
        || lc.starts_with("mailto:")
        || lc.starts_with("tel:")
        || lc.starts_with("sms:")
        || lc.starts_with("data:")
    {
        return None;
    }
    // Fragmentos puros (`#foo`): resuelven a la URL actual + fragment.
    // El chrome detecta same-page navigation (mismo URL sans fragment)
    // y scrollea al elemento con id matching en lugar de recargar.
    if href.starts_with('#') {
        return base.and_then(|b| b.join(href).ok()).map(|u| u.to_string());
    }
    if let Ok(abs) = url::Url::parse(href) {
        // Sólo http/https son navegables por puriy hoy. file://, ftp://,
        // etc. quedan ignorados para no romper la pestaña.
        return match abs.scheme() {
            "http" | "https" | "about" => Some(abs.into()),
            _ => None,
        };
    }
    base.and_then(|b| b.join(href).ok()).and_then(|abs| {
        match abs.scheme() {
            "http" | "https" | "about" => Some(abs.into()),
            _ => None,
        }
    })
}

impl ComputedStyle {
    // Asegura que ComputedStyle es referenciable desde boxes (sin re-export
    // cycles). Sin este impl no haría falta; lo dejamos para forzar el
    // link en docs.
    #[doc(hidden)]
    pub fn _link(_: &Self) {}
}

#[cfg(test)]
mod tests {
    use crate::Engine;

    #[test]
    fn box_tree_no_vacio() {
        let html = "<html><body><h1>Hola</h1><p>Mundo</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        assert!(doc.box_tree.descendants_count() >= 3);
    }

    #[test]
    fn display_none_excluye_head() {
        let html = "<html><head><title>t</title></head><body><p>x</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // El árbol parte de body — head no debe haber aportado nada.
        let mut tags = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.tag {
                tags.push(t.clone());
            }
        });
        assert!(!tags.contains(&"title".to_string()));
        assert!(!tags.contains(&"head".to_string()));
    }

    #[test]
    fn ol_li_recibe_marker_decimal() {
        let html =
            "<html><body><ol><li>uno</li><li>dos</li><li>tres</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["1. ".to_string(), "2. ".into(), "3. ".into()]);
    }

    #[test]
    fn ul_li_recibe_marker_bullet() {
        let html = "<html><body><ul><li>a</li><li>b</li></ul></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.starts_with('•') {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers.len(), 2);
    }

    #[test]
    fn list_style_none_suprime_marker() {
        let html = r#"<html><head><style>
            ul { list-style-type: none }
        </style></head><body><ul><li>uno</li><li>dos</li></ul></body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut has_bullet = false;
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.contains('•') {
                    has_bullet = true;
                }
            }
        });
        assert!(!has_bullet, "no debería haber marker con list-style-type:none");
    }

    #[test]
    fn ol_start_corre_el_contador() {
        let html =
            "<html><body><ol start=\"5\"><li>x</li><li>y</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["5. ".to_string(), "6. ".into()]);
    }

    #[test]
    fn li_value_resetea_el_contador() {
        let html = "<html><body><ol><li>x</li><li value=\"10\">y</li><li>z</li></ol></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        assert_eq!(markers, vec!["1. ".to_string(), "10. ".into(), "11. ".into()]);
    }

    #[test]
    fn lower_roman_y_lower_alpha_aplican() {
        let html = r#"<html><head><style>
            .roman { list-style-type: lower-roman }
            .alpha { list-style-type: upper-alpha }
        </style></head><body>
          <ol class="roman"><li>a</li><li>b</li><li>c</li></ol>
          <ol class="alpha"><li>a</li><li>b</li></ol>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut markers = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(t) = &b.text {
                if t.ends_with(". ") {
                    markers.push(t.clone());
                }
            }
        });
        // ol.roman → i. ii. iii.   ol.alpha → A. B.
        assert_eq!(
            markers,
            vec![
                "i. ".to_string(),
                "ii. ".into(),
                "iii. ".into(),
                "A. ".into(),
                "B. ".into(),
            ]
        );
    }

    #[test]
    fn to_alpha_y_to_roman_son_correctos() {
        use super::{to_alpha, to_roman};
        assert_eq!(to_alpha(1, false), "a");
        assert_eq!(to_alpha(26, false), "z");
        assert_eq!(to_alpha(27, false), "aa");
        assert_eq!(to_alpha(28, false), "ab");
        assert_eq!(to_alpha(52, true), "AZ");
        assert_eq!(to_roman(4, false), "iv");
        assert_eq!(to_roman(9, true), "IX");
        assert_eq!(to_roman(1994, false), "mcmxciv");
        assert_eq!(to_roman(3999, true), "MMMCMXCIX");
        // Fuera de rango → decimal fallback.
        assert_eq!(to_roman(4000, false), "4000");
        assert_eq!(to_roman(0, true), "0");
    }

    #[test]
    fn estilo_inline_aplica_color() {
        let html = r#"<html><body><p style="color: #ff0000">x</p></body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found_red = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") && b.color == super::Color::rgb(255, 0, 0) {
                found_red = true;
            }
        });
        assert!(found_red, "no se encontró <p> con color rojo");
    }

    #[test]
    fn details_sin_open_attr_arranca_cerrado() {
        let html = r#"<html><body>
            <details><summary>Tit</summary><p>Contenido</p></details>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut details_attr: Vec<bool> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("details") {
                details_attr.push(b.details_open_attr);
            }
        });
        assert_eq!(details_attr, vec![false]);
    }

    #[test]
    fn details_con_open_attr_lo_refleja() {
        let html = r#"<html><body>
            <details open><summary>Tit</summary><p>Contenido</p></details>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut details_attr: Vec<bool> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("details") {
                details_attr.push(b.details_open_attr);
            }
        });
        assert_eq!(details_attr, vec![true]);
    }

    #[test]
    fn details_summary_se_parsean_como_tags() {
        let html = r#"<html><body>
            <details><summary>Tit</summary><p>Contenido</p></details>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut saw_details = false;
        let mut saw_summary = false;
        doc.box_tree.walk(|b| {
            match b.tag.as_deref() {
                Some("details") => saw_details = true,
                Some("summary") => saw_summary = true,
                _ => {}
            }
        });
        assert!(saw_details, "no se encontró <details> en el box tree");
        assert!(saw_summary, "no se encontró <summary> en el box tree");
    }

    #[test]
    fn details_open_attr_es_false_para_nodos_no_details() {
        let html = "<html><body><p>x</p><h1>y</h1></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() != Some("details") {
                assert!(!b.details_open_attr, "{:?} no debería tener details_open_attr=true", b.tag);
            }
        });
    }

    #[test]
    fn ws_entre_blocks_se_filtra() {
        // El "\n  " entre </h1> y <p> produce un Text node " " que NO
        // debería rendear como un row vacío.
        let html = "<html><body><h1>A</h1>\n  <p>B</p>\n  <h2>C</h2></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Walk del body. Esperamos sólo h1, p, h2 como children directos
        // (sin text-leaves de whitespace entre ellos).
        let body = &doc.box_tree.root;
        // Body envuelve un Inline de transición (collapse_whitespace puede
        // dejar uno leading o trailing). Recorremos directamente.
        let mut top_tags: Vec<Option<String>> = body
            .children
            .iter()
            .filter(|c| !super::is_ws_only_inline(c))
            .map(|c| c.tag.clone())
            .collect();
        // Aseguramos que el filtrado sólo dejó tags reales.
        top_tags.retain(|t| t.is_some());
        let names: Vec<&str> = top_tags
            .iter()
            .map(|t| t.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(names, vec!["h1", "p", "h2"]);
        // Y verificamos que NO hay inlines whitespace-only entre ellos en
        // el árbol real (post-strip).
        for c in &body.children {
            assert!(
                !super::is_ws_only_inline(c),
                "el body no debería tener inlines ws-only entre blocks: {:?}",
                c.text
            );
        }
    }

    #[test]
    fn ws_alrededor_de_inline_se_preserva() {
        // El espacio entre "foo " y <strong>bar</strong> y " baz" sí
        // tiene valor — debe quedarse para no pegar tokens.
        let html = "<html><body><p>foo <strong>bar</strong> baz</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        // Encontramos el <p> y verificamos que sus children contengan
        // textos con espacios donde corresponde.
        let mut texts: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("p") {
                for c in &b.children {
                    if let Some(t) = &c.text {
                        texts.push(t.clone());
                    }
                    // Si es <strong>, mirá su hijo
                    if c.tag.as_deref() == Some("strong") {
                        for cc in &c.children {
                            if let Some(t) = &cc.text {
                                texts.push(format!("[strong]{t}"));
                            }
                        }
                    }
                }
            }
        });
        // Esperamos que "foo " conserve el espacio trailing y " baz" el leading.
        assert!(
            texts.iter().any(|t| t.ends_with(' ')),
            "esperaba un text con espacio trailing en {:?}",
            texts
        );
        assert!(
            texts.iter().any(|t| t.starts_with(' ')),
            "esperaba un text con espacio leading en {:?}",
            texts
        );
        assert!(
            texts.iter().any(|t| t == "[strong]bar"),
            "esperaba `bar` dentro de strong en {:?}",
            texts
        );
    }

    #[test]
    fn link_target_blank_marca_link_new_tab() {
        let html = r#"<html><body>
            <a href="https://a.test/" target="_blank">A</a>
            <a href="https://b.test/">B</a>
            <a href="https://c.test/" target="_self">C</a>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut links: Vec<(String, bool)> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") {
                if let Some(target) = &b.link {
                    links.push((target.clone(), b.link_new_tab));
                }
            }
        });
        assert!(links.iter().any(|(u, nt)| u.contains("a.test") && *nt));
        assert!(links.iter().any(|(u, nt)| u.contains("b.test") && !*nt));
        assert!(links.iter().any(|(u, nt)| u.contains("c.test") && !*nt));
    }

    #[test]
    fn link_mailto_y_tel_y_javascript_se_ignoran() {
        let html = r#"<html><body>
            <a href="mailto:foo@bar">M</a>
            <a href="tel:+541112345678">T</a>
            <a href="javascript:alert(1)">J</a>
            <a href="data:text/plain,hi">D</a>
            <a href="ftp://example.com/">F</a>
        </body></html>"#;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut clickable: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("a") {
                if let Some(t) = &b.link {
                    clickable.push(t.clone());
                }
            }
        });
        assert!(clickable.is_empty(), "ningún href no-web debería ser clickable: {clickable:?}");
    }

    #[test]
    fn srcset_elige_la_densidad_mas_alta() {
        let url = super::pick_srcset("foo.png 1x, foo-2x.png 2x, foo-3x.png 3x");
        assert_eq!(url.as_deref(), Some("foo-3x.png"));
    }

    #[test]
    fn srcset_elige_el_ancho_mas_grande() {
        let url = super::pick_srcset("a.png 320w, b.png 800w, c.png 1600w");
        assert_eq!(url.as_deref(), Some("c.png"));
    }

    #[test]
    fn srcset_sin_descriptor_usa_la_primera_con_1x_implicito() {
        // En la práctica un srcset sin descriptor es equivalente a 1x.
        let url = super::pick_srcset("a.png, b.png");
        // No importa el orden interno — basta con que devuelva alguno.
        assert!(url.is_some());
    }

    #[test]
    fn svg_parsea_polygon_y_polyline() {
        let html = r##"<html><body>
            <svg width="100" height="100">
                <polygon points="0,0 50,0 50,50" fill="red"/>
                <polyline points="0,100 100,50 100,0" stroke="blue"/>
            </svg>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut prim_count = 0;
        let mut had_closed = false;
        let mut had_open = false;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.svg {
                for p in &s.prims {
                    if let crate::SvgPrim::Polyline { points, closed, .. } = p {
                        prim_count += 1;
                        if *closed {
                            had_closed = true;
                            assert_eq!(points.len(), 3);
                        } else {
                            had_open = true;
                        }
                    }
                }
            }
        });
        assert_eq!(prim_count, 2);
        assert!(had_closed);
        assert!(had_open);
    }

    #[test]
    fn svg_parsea_path_minimal() {
        let html = r##"<html><body>
            <svg width="100" height="100">
                <path d="M 10 10 L 90 10 L 50 90 Z" fill="green"/>
            </svg>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut cmds_count = 0;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.svg {
                for p in &s.prims {
                    if let crate::SvgPrim::Path { d, .. } = p {
                        cmds_count = d.len();
                    }
                }
            }
        });
        // M, L, L, Z → 4 cmds.
        assert_eq!(cmds_count, 4);
    }

    #[test]
    fn svg_recolecta_rect_circle_y_line() {
        let html = r##"<html><body>
            <svg width="200" height="100" viewBox="0 0 200 100">
                <rect x="10" y="10" width="50" height="30" fill="red" stroke="black" stroke-width="2"/>
                <circle cx="120" cy="50" r="20" fill="blue"/>
                <line x1="0" y1="0" x2="200" y2="100" stroke="green" stroke-width="3"/>
            </svg>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut scene: Option<crate::SvgScene> = None;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.svg {
                scene = Some(s.clone());
            }
        });
        let scene = scene.expect("debería haber un <svg>");
        assert_eq!(scene.width, 200.0);
        assert_eq!(scene.height, 100.0);
        assert_eq!(scene.view_box, Some((0.0, 0.0, 200.0, 100.0)));
        assert_eq!(scene.prims.len(), 3);
        match &scene.prims[0] {
            crate::SvgPrim::Rect { x, y, w, h, fill, stroke, .. } => {
                assert_eq!(*x, 10.0);
                assert_eq!(*y, 10.0);
                assert_eq!(*w, 50.0);
                assert_eq!(*h, 30.0);
                assert!(fill.is_some());
                assert!(stroke.is_some());
            }
            _ => panic!("primera prim debería ser Rect"),
        }
        match &scene.prims[1] {
            crate::SvgPrim::Circle { cx, cy, r, .. } => {
                assert_eq!(*cx, 120.0);
                assert_eq!(*cy, 50.0);
                assert_eq!(*r, 20.0);
            }
            _ => panic!("segunda prim debería ser Circle"),
        }
        match &scene.prims[2] {
            crate::SvgPrim::Line { x1, y2, .. } => {
                assert_eq!(*x1, 0.0);
                assert_eq!(*y2, 100.0);
            }
            _ => panic!("tercera prim debería ser Line"),
        }
    }

    #[test]
    fn select_recolecta_options_y_seleccionado_inicial() {
        let html = r##"<html><body>
            <form action="/p">
                <select name="lang">
                    <option value="es">Español</option>
                    <option value="en" selected>English</option>
                    <option>Otro</option>
                </select>
            </form>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("https://example.com/", html);
        let mut info: Option<crate::SelectInfo> = None;
        doc.box_tree.walk(|b| {
            if let Some(s) = &b.select {
                info = Some(s.clone());
                assert_eq!(b.input_name.as_deref(), Some("lang"));
                assert_eq!(b.form_idx, Some(0));
            }
        });
        let info = info.expect("debería haber un <select>");
        assert_eq!(info.options.len(), 3);
        assert_eq!(info.options[0].value, "es");
        assert_eq!(info.options[0].label, "Español");
        assert_eq!(info.options[2].label, "Otro");
        assert_eq!(info.options[2].value, "Otro"); // fallback al label
        assert_eq!(info.initial, 1); // <option selected> es el segundo
    }

    #[test]
    fn form_asigna_form_idx_a_inputs_que_contiene() {
        let html = r##"<html><body>
            <form action="/search" method="get">
                <input type="text" name="q" value="hola">
                <input type="text" name="lang" value="es">
            </form>
            <input type="text" name="outside">
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("https://example.com/", html);
        assert_eq!(doc.box_tree.forms.len(), 1);
        let mut names_inside: Vec<String> = Vec::new();
        let mut outside_form_idx: Option<usize> = None;
        doc.box_tree.walk(|b| {
            if let Some(name) = &b.input_name {
                if b.form_idx == Some(0) {
                    names_inside.push(name.clone());
                } else if b.input_kind.is_some() && name == "outside" {
                    outside_form_idx = b.form_idx;
                }
            }
        });
        assert_eq!(names_inside, vec!["q".to_string(), "lang".into()]);
        assert_eq!(outside_form_idx, None);
        assert_eq!(
            doc.box_tree.forms[0].action.as_deref(),
            Some("https://example.com/search")
        );
    }

    #[test]
    fn em_y_i_y_cite_son_italic_por_default() {
        let html = "<html><body><em>a</em><i>b</i><cite>c</cite><p>d</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found: Vec<(String, crate::FontStyle)> = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(tag) = &b.tag {
                if matches!(tag.as_str(), "em" | "i" | "cite" | "p") {
                    found.push((tag.clone(), b.font_style));
                }
            }
        });
        let em = found.iter().find(|(t, _)| t == "em").unwrap();
        let i = found.iter().find(|(t, _)| t == "i").unwrap();
        let cite = found.iter().find(|(t, _)| t == "cite").unwrap();
        let p = found.iter().find(|(t, _)| t == "p").unwrap();
        assert_eq!(em.1, crate::FontStyle::Italic);
        assert_eq!(i.1, crate::FontStyle::Italic);
        assert_eq!(cite.1, crate::FontStyle::Italic);
        assert_eq!(p.1, crate::FontStyle::Normal);
    }

    #[test]
    fn font_style_normal_override_padre_italic() {
        let html = r##"<html><head><style>
            .x { font-style: normal }
        </style></head><body><em>fuera<span class="x">dentro</span></em></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut span_style: Option<crate::FontStyle> = None;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("span") {
                span_style = Some(b.font_style);
            }
        });
        assert_eq!(span_style, Some(crate::FontStyle::Normal));
    }

    #[test]
    fn focus_pseudo_aporta_a_focus_background() {
        use crate::StyleEngine;
        let html = r##"<html><head><style>
            input { background: white }
            input:focus { background: #ffeecc }
        </style></head><body><input type="text"></body></html>"##;
        let dom = crate::DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let input = dom.find("input").unwrap();
        let base = styles.compute_with_parent_for_state(&input, None, false, false);
        let focused = styles.compute_with_parent_for_state(&input, None, false, true);
        // base es blanco (255,255,255), focused es #ffeecc (255,238,204).
        assert_eq!(base.background.map(|c| (c.r, c.g, c.b)), Some((255, 255, 255)));
        assert_eq!(focused.background.map(|c| (c.r, c.g, c.b)), Some((255, 238, 204)));
    }

    #[test]
    fn box_tree_expone_focus_background() {
        let html = r##"<html><head><style>
            input:focus { background: #abcdef }
        </style></head><body><input type="text"></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("input") {
                assert_eq!(
                    b.focus_background.map(|c| (c.r, c.g, c.b)),
                    Some((0xab, 0xcd, 0xef))
                );
                found = true;
            }
        });
        assert!(found, "no se encontró <input> en el box tree");
    }

    #[test]
    fn parsea_background_image_url_a_computed_style_y_no_descarga_si_url_no_resuelve() {
        // Sin red, fetch_and_decode falla y background_image queda None.
        // Pero el url SÍ debe quedar capturado en computed.background_image_url
        // (visible al re-parsear el stylesheet).
        use crate::StyleEngine;
        let html = r##"<html><head><style>
            .hero { background-image: url("https://nope.invalid/bg.png") }
        </style></head><body><div class="hero">x</div></body></html>"##;
        let dom = crate::DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let div = dom.find("div").expect("debería encontrar <div>");
        let s = styles.compute_with_parent(&div, None);
        assert_eq!(
            s.background_image_url.as_deref(),
            Some("https://nope.invalid/bg.png")
        );
    }

    #[test]
    fn background_image_none_limpia_url() {
        use crate::StyleEngine;
        let html = r##"<html><head><style>
            .hero { background-image: url(a.png) }
            .hero.off { background-image: none }
        </style></head><body><div class="hero off">x</div></body></html>"##;
        let dom = crate::DomTree::parse(html);
        let styles = StyleEngine::from_dom(&dom);
        let div = dom.find("div").expect("debería encontrar <div>");
        let s = styles.compute_with_parent(&div, None);
        assert!(s.background_image_url.is_none());
    }

    #[test]
    fn link_fragmento_se_resuelve_a_base_mas_frag() {
        // Antes: `#top` se ignoraba (None). Ahora resuelve contra la
        // base — el chrome detecta same-page y scrollea en lugar de
        // recargar la URL.
        let html = r##"<html><body><a href="#top">arriba</a></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("https://example.com/doc", html);
        let mut links: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(l) = &b.link {
                links.push(l.clone());
            }
        });
        assert_eq!(links, vec!["https://example.com/doc#top".to_string()]);
    }

    #[test]
    fn find_y_of_match_devuelve_y_creciente_por_match() {
        let html = r##"<html><body>
            <p>alfa</p><p>beta</p><p>alfa beta</p><p>alfa</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let bt = &doc.box_tree;
        let y1 = bt.find_y_of_match("alfa", 1).expect("match 1");
        let y2 = bt.find_y_of_match("alfa", 2).expect("match 2");
        let y3 = bt.find_y_of_match("alfa", 3).expect("match 3");
        assert!(y2 > y1, "match 2 debe quedar más abajo que match 1");
        assert!(y3 > y2);
        // Sin match para el 4to.
        assert!(bt.find_y_of_match("alfa", 4).is_none());
        // Query vacía o nth=0 devuelven None.
        assert!(bt.find_y_of_match("", 1).is_none());
        assert!(bt.find_y_of_match("alfa", 0).is_none());
    }

    #[test]
    fn input_autofocus_se_marca_solo_para_inputs_con_attr() {
        let html = r##"<html><body>
            <form>
                <input type="text" name="a">
                <input type="text" name="b" autofocus>
                <input type="text" name="c" autofocus>
            </form>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut flags: Vec<bool> = Vec::new();
        doc.box_tree.walk(|b| {
            if b.input_kind.is_some() {
                flags.push(b.input_autofocus);
            }
        });
        assert_eq!(flags, vec![false, true, true]);
    }

    #[test]
    fn element_id_se_extrae_del_attr() {
        let html = r##"<html><body>
            <h2 id="intro">Intro</h2>
            <p id="">vacío no cuenta</p>
            <p>sin id</p>
        </body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut ids: Vec<String> = Vec::new();
        doc.box_tree.walk(|b| {
            if let Some(id) = &b.element_id {
                ids.push(id.clone());
            }
        });
        assert_eq!(ids, vec!["intro".to_string()]);
    }

    #[test]
    fn ws_solo_inline_no_se_dropea_si_padre_es_inline_flow() {
        // <p>foo<span> </span>bar</p> — el espacio dentro de span sí debe
        // quedar porque separa "foo" de "bar".
        let html = "<html><body><p>foo<span> </span>bar</p></body></html>";
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut found_space = false;
        doc.box_tree.walk(|b| {
            if b.tag.as_deref() == Some("span") {
                for c in &b.children {
                    if c.text.as_deref().map(|s| s.contains(' ')).unwrap_or(false) {
                        found_space = true;
                    }
                }
            }
        });
        assert!(found_space, "el espacio dentro de <span> debería preservarse");
    }
}
