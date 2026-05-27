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
    AlignItems, BoxShadow, ComputedStyle, FlexDirection, FlexWrap, JustifyContent, LengthVal,
    ListStyleType, Sides, StyleEngine, TextAlign, TextDecorationLine,
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
        text_decoration: TextDecorationLine::None,
        text: None,
        children: Vec::new(),
        tag: Some("body".into()),
        link: None,
        image: None,
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
                text_decoration: style.text_decoration,
                text: None,
                children,
                tag,
                link,
                image,
            })
        }
        NodeData::Text { contents } => {
            let raw = contents.borrow().to_string();
            // CSS whitespace collapse: colapsa runs internos a un solo
            // espacio, preserva un espacio al inicio o fin si lo había
            // (caso clásico: `foo <a>bar</a> baz` debe rendear "foo bar
            // baz" — sin el espacio adyacente al link los tokens se
            // pegan al renderizarse en views vecinas).
            let collapsed = collapse_whitespace(&raw);
            if collapsed.is_empty() {
                return None;
            }
            // El leaf de texto hereda las propiedades inheritables del
            // padre (color, font-size, font-weight, text-align,
            // line-height). Sin esto, todo texto sale negro 16px aunque
            // el `<p>` padre indique color rojo.
            Some(inline_text_with_style(collapsed, parent_style.unwrap_or(&ComputedStyle::default())))
        }
        _ => {
            // Document / Doctype / Comment → recurrir sólo en hijos.
            let mut children = Vec::new();
            for child in node.children.borrow().iter() {
                if let Some(b) = build_node(child, styles, base, parent_style) {
                    children.push(b);
                }
            }
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
                text_decoration: p.text_decoration,
                text: None,
                children,
                tag: None,
                link: None,
                image: None,
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
        text_decoration: style.text_decoration,
        text: Some(s),
        children: Vec::new(),
        tag: None,
        link: None,
        image: None,
    }
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

/// Colapso de whitespace al estilo CSS (sin `white-space: pre`):
/// - todo run de whitespace interno → un solo espacio
/// - preserva un espacio leading/trailing si existía
/// - vacío puro → `""` (el caller decide skipear)
fn collapse_whitespace(s: &str) -> String {
    let leading = s.chars().next().is_some_and(|c| c.is_whitespace());
    let trailing = s.chars().last().is_some_and(|c| c.is_whitespace());
    let core: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if core.is_empty() {
        // Sólo whitespace: si el padre lo coloca entre dos inlines
        // (caso `foo <a>bar</a>`), el espacio entre <a> y "foo" llega
        // como un Text-node de un solo espacio. Lo conservamos como
        // un solo " " para no perder la separación.
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
    if href.is_empty() || href.starts_with('#') || href.starts_with("javascript:") {
        return None;
    }
    if let Ok(abs) = url::Url::parse(href) {
        return Some(abs.into());
    }
    base.and_then(|b| b.join(href).ok()).map(Into::into)
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
}
