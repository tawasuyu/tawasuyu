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
    AlignItems, AlignSelf, BoxShadow, BoxSizing, ComputedStyle, FlexDirection, FlexWrap,
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
    /// Ancho del border en px.
    pub border_width: f32,
    /// Color del border. `None` = no se dibuja.
    pub border_color: Option<Color>,
    /// Radio corner-radius en px.
    pub border_radius: f32,
    /// Background a aplicar cuando el nodo está bajo el mouse. `None` =
    /// no hay regla `:hover` que cambie el background del nodo. El
    /// chrome lo plug-ea vía `View::hover_fill`. Restyle completo en
    /// hover (cambios de color/border) queda fuera de scope por ahora.
    pub hover_background: Option<Color>,
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
    let base = url::Url::parse(base_url).ok();
    let body = dom.find("body").unwrap_or_else(|| dom.document());
    let root = build_node(&body, styles, base.as_ref(), None).unwrap_or_else(empty_root);
    BoxTree { root }
}

fn empty_root() -> BoxNode {
    BoxNode {
        display: Display::Block,
        background: None,
        color: Color::BLACK,
        font_size: 16.0,
        font_weight: 400,
        margin: Sides::all(0.0),
        padding: Sides::all(0.0),
        width: LengthVal::Auto,
        max_width: LengthVal::Auto,
        text_align: TextAlign::Left,
        line_height: None,
        border_width: 0.0,
        border_color: None,
        border_radius: 0.0,
        hover_background: None,
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
            // Hover style: recomputamos con hover_active=true y vemos si
            // alguna pseudo-clase `:hover` cambió el background. Si sí,
            // exponemos el delta al chrome para que use hover_fill().
            // Resto del diff (color/border/etc.) queda fuera por ahora —
            // restyle completo en hover requeriría re-mount del tree.
            let hover_style = styles.compute_with_parent_in_state(node, parent_style, true);
            let hover_background = if hover_style.background != style.background {
                hover_style.background
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
            // <img>: descarga + decode sync. Si falla, el campo queda
            // None y el chrome muestra placeholder con el alt.
            let image = if tag.as_deref() == Some("img") {
                dom::attr(node, "src")
                    .and_then(|s| resolve_href(base, &s))
                    .and_then(|abs| fetch_and_decode(&abs))
            } else {
                None
            };
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
                margin: style.margin,
                padding: style.padding,
                width: style.width,
                max_width: style.max_width,
                text_align: style.text_align,
                line_height: style.line_height,
                border_width: style.border_width,
                border_color: style.border_color,
                border_radius: style.border_radius,
                hover_background,
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
                margin: Sides::all(0.0),
                padding: Sides::all(0.0),
                width: LengthVal::Auto,
                max_width: LengthVal::Auto,
                text_align: p.text_align,
                line_height: p.line_height,
                border_width: 0.0,
                border_color: None,
                border_radius: 0.0,
                hover_background: None,
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
        margin: Sides::all(0.0),
        padding: Sides::all(0.0),
        width: LengthVal::Auto,
        max_width: LengthVal::Auto,
        text_align: style.text_align,
        line_height: style.line_height,
        border_width: 0.0,
        border_color: None,
        border_radius: 0.0,
        hover_background: None,
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
    // Fragmentos puros (`#foo`) — no navegan a una URL distinta. Por
    // ahora los descartamos; en el futuro deberían scrollear al
    // elemento con id="foo".
    if href.starts_with('#') {
        return None;
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
    fn link_fragmento_se_ignora() {
        let html = r##"<html><body><a href="#top">arriba</a></body></html>"##;
        let eng = Engine::new();
        let doc = eng.load_html("about:test", html);
        let mut any_link = false;
        doc.box_tree.walk(|b| {
            if b.link.is_some() {
                any_link = true;
            }
        });
        assert!(!any_link, "fragmento puro #top no debería resolverse a una URL navegable");
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
