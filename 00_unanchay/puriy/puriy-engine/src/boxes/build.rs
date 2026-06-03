//! Construcción del box tree desde el DOM + StyleEngine: `build`/`build_node`
//! (walk recursivo que computa estilos y arma cada `BoxNode`), recolección de
//! forms y de `<svg>` (parseo de prims/paths), texto inline, colapso de
//! márgenes, y prefetch/decodificación de imágenes (`fetch_image_src`).
//! Extraído de `boxes/mod.rs` (regla #1). Comparte tipos del crate vía `use super::*`.
use super::*;

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
    // Prefetch paralelo de imágenes: pre-walk del DOM antes del build
    // recolecta todas las URLs de `<img>`/`<picture>` (resueltas contra
    // base) y las baja en paralelo con un pool de workers. Las bytes
    // quedan en la cache global; el `fetch_and_decode` síncrono dentro
    // de `build_node` después hace cache hit. Esto convierte el parse
    // de una página con 20 imágenes de "20 round-trips serializados"
    // a "ceil(20/N) round-trips". `background-image: url(...)` no
    // entra al pre-walk todavía — vive en CSS y requiere computar
    // styles primero.
    prefetch_image_urls(&dom.document(), base.as_ref());
    // Segundo pass de prefetch: `background-image: url(...)` vive en
    // CSS — necesita styles computados, así que va después del primer
    // pre-walk. Computamos sin parent style (background-image no es
    // heredable, así que el value es independiente del padre). Las
    // URLs descargadas también caen en la cache global.
    prefetch_background_image_urls(&dom.document(), styles, base.as_ref());
    let mut counters: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    let mut root = build_node(&body, styles, base.as_ref(), None, &mut counters)
        .unwrap_or_else(empty_root);
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
    // Identidad estable por nodo (1..N en DFS pre-orden). El chrome la usa
    // para llevar estado por-nodo (tween de `transition` en hover) keyeado
    // por id, sin contar índices en walks paralelos frágiles.
    let mut node_cursor: u32 = 1;
    assign_node_ids(&mut root, &mut node_cursor);
    BoxTree { root, forms, styles: styles.clone() }
}

/// Post-pass: numera cada nodo del árbol en orden DFS pre-orden empezando
/// en `*next`. Determinista y estable mientras la estructura del árbol no
/// cambie — exactamente la garantía que necesita el estado de hover del
/// chrome (keyeado por `node_id`).
pub(crate) fn assign_node_ids(node: &mut BoxNode, next: &mut u32) {
    node.node_id = *next;
    *next += 1;
    for child in &mut node.children {
        assign_node_ids(child, next);
    }
}

pub(crate) fn collect_forms_dom(node: &Handle, base: Option<&url::Url>, out: &mut Vec<FormInfo>) {
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

pub(crate) fn assign_form_idx(b: &mut BoxNode, stack: &mut Vec<usize>, cursor: &mut usize) {
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
pub(crate) fn collect_svg(svg_node: &Handle) -> SvgScene {
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

pub(crate) fn collect_svg_prims(node: &Handle, out: &mut Vec<SvgPrim>) {
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

pub(crate) fn svg_num(node: &Handle, name: &str, default: f32) -> f32 {
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

pub(crate) fn parse_svg_points(s: &str) -> Vec<(f32, f32)> {
    let nums: Vec<f32> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse::<f32>().ok())
        .collect();
    nums.chunks_exact(2).map(|c| (c[0], c[1])).collect()
}

/// Parser de `d=` minimal: soporta M/m, L/l, H/h, V/v, C/c, Q/q, Z/z.
/// No soporta A (arcs), T, S (smooth bezier).
pub(crate) fn parse_svg_path(d: &str) -> Vec<PathCmd> {
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

pub(crate) fn svg_color(node: &Handle, name: &str) -> Option<Color> {
    let v = dom::attr(node, name)?;
    let v = v.trim();
    if v.eq_ignore_ascii_case("none") {
        return None;
    }
    crate::style::parse_color_named_or_hex(v)
}

pub(crate) fn empty_root() -> BoxNode {
    BoxNode {
        display: Display::Block,
        background: None,
        color: Color::BLACK,
        font_size: 16.0,
        font_weight: 400,
        font_style: crate::style::FontStyle::Normal,
        font_family: None,
        margin: Sides::all(0.0),
        margin_left_auto: false,
        margin_right_auto: false,
        padding: Sides::all(0.0),
        width: LengthVal::Auto,
        height: LengthVal::Auto,
        max_width: LengthVal::Auto,
        text_align: TextAlign::Left,
        line_height: None,
        border_widths: Sides::all(0.0),
        border_colors: Sides::all(None),
        border_radii: Corners::all(0.0),
        border_style: BorderLineStyle::Solid,
        hover_background: None,
        focus_background: None,
        box_shadows: Vec::new(),
        z_index: 0,
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::Start,
        align_items: AlignItems::Stretch,
        align_content: AlignContent::Normal,
        justify_items: None,
        justify_self: AlignSelf::Auto,
        flex_wrap: FlexWrap::NoWrap,
        gap_row: 0.0,
        gap_column: 0.0,
        box_sizing: BoxSizing::ContentBox,
        min_width: LengthVal::Auto,
        min_height: LengthVal::Auto,
        max_height: LengthVal::Auto,
        aspect_ratio: None,
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
        object_fit: None,
        caret_color: None,
        accent_color: None,
        cursor: Cursor::Auto,
        object_position: None,
        pointer_events: PointerEvents::Auto,
        text_indent: 0.0,
        word_spacing: 0.0,
        letter_spacing: 0.0,
        text_shadows: Vec::new(),
        transforms: Vec::new(),
        grid_template_columns: Vec::new(),
        grid_template_rows: Vec::new(),
        text_decoration: TextDecorationLine::None,
        text_decoration_color: None,
        text_decoration_style: TextDecorationStyle::Solid,
        text_decoration_thickness: None,
        text_underline_offset: None,
        text: None,
        children: Vec::new(),
        tag: Some("body".into()),
        link: None,
        image: None,
        details_open_attr: false,
        link_new_tab: false,
        link_download: None,
        background_image: None,
        background_size: BackgroundSize::Auto,
        background_position: BackgroundPosition { x: LengthVal::Pct(0.0), y: LengthVal::Pct(0.0) },
        background_repeat: BackgroundRepeat::Repeat,
        background_extra_layers: Vec::new(),
        background_origin: BackgroundOrigin::PaddingBox,
        background_clip: BackgroundClip::BorderBox,
        input_kind: None,
        input_initial: None,
        input_placeholder: None,
        input_name: None,
        input_checked_initial: false,
        input_autofocus: false,
        form_idx: None,
        select: None,
        svg: None,
        canvas: None,
        element_id: None,
        class_list: Vec::new(),
        attributes: Vec::new(),
        animation: None,
        transitions: Vec::new(),
        node_id: 0,
    }
}

pub(crate) fn build_node(
    node: &Handle,
    styles: &StyleEngine,
    base: Option<&url::Url>,
    parent_style: Option<&ComputedStyle>,
    counters: &mut std::collections::HashMap<String, i32>,
) -> Option<BoxNode> {
    match &node.data {
        NodeData::Element { .. } => {
            let style = styles.compute_with_parent(node, parent_style);
            if style.display == Display::None {
                // Distinguimos el `display:none` de RUIDO UA (script/style/
                // option/colgroup/canvas/...) — que se descarta — del puesto
                // por el AUTOR (CSS de la página), que se RETIENE como box
                // oculto con su subárbol, para que un toggle de clase (restyle,
                // Fase 7.184) pueda mostrarlo. El chrome no pinta ni reserva
                // espacio para boxes `Display::None` (TaffyDisplay::None).
                // Fase 7.185.
                let tag = dom::element_name(node).unwrap_or_default();
                if crate::style::tag_defaults_to_none(&tag) {
                    return None;
                }
                // Cae a través: construye el box (display=None) y su subárbol.
            }
            // CSS counters: aplicar reset (sobrescribe) y luego
            // increment al entrar al nodo. Implementación pragmática
            // — un map global que sólo crece. CSS spec dice que reset
            // crea scope nuevo por subárbol, pero eso requiere un
            // stack y rara vez importa para los usos comunes (numbered
            // headings, breadcrumbs); cuando importe se mete el stack.
            for (name, val) in &style.counter_reset {
                counters.insert(name.clone(), *val);
            }
            for (name, delta) in &style.counter_increment {
                *counters.entry(name.clone()).or_insert(0) += *delta;
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
            let link_download = if tag.as_deref() == Some("a") {
                dom::attr(node, "download").map(|s| s.trim().to_string())
            } else {
                None
            };

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
            // `<canvas>`: tamaño intrínseco de los atributos `width`/`height`
            // (px crudos; default 300×150 por spec). El chrome casa el
            // `element_id` con el contexto 2D del runtime y pinta sus comandos.
            let canvas = if tag.as_deref() == Some("canvas") {
                let cw = dom::attr(node, "width")
                    .and_then(|s| s.trim().parse::<f32>().ok())
                    .filter(|v| *v > 0.0)
                    .unwrap_or(300.0);
                let ch = dom::attr(node, "height")
                    .and_then(|s| s.trim().parse::<f32>().ok())
                    .filter(|v| *v > 0.0)
                    .unwrap_or(150.0);
                Some((cw, ch))
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
                src_candidate.and_then(|s| fetch_image_src(base, &s))
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
                chosen.and_then(|s| fetch_image_src(base, &s))
            } else {
                None
            };
            // `background-image: url(...)` — resolver contra base y
            // descargar/decode. Misma cache que `<img>` por la fetch::
            // global. Falla silenciosa → background_image queda None.
            let background_image = style
                .background_image_url
                .as_deref()
                .and_then(|u| fetch_image_src(base, u));
            // Capas de background extra (lista `background: a, b, ...`):
            // resolver cada `url(...)` igual que la capa 0 (misma cache);
            // las que fallan se descartan; los gradientes pasan tal cual.
            let background_extra_layers: Vec<BoxBackgroundLayer> = style
                .background_extra_layers
                .iter()
                .filter_map(|l| {
                    let (image, gradient) = match &l.image {
                        crate::style::BackgroundImage::Url(u) => {
                            match fetch_image_src(base, u) {
                                Some(img) => (Some(img), None),
                                None => return None, // url no resoluble → capa descartada
                            }
                        }
                        crate::style::BackgroundImage::Gradient(g) => (None, Some(g.clone())),
                    };
                    Some(BoxBackgroundLayer {
                        image,
                        gradient,
                        size: l.size,
                        position: l.position,
                        repeat: l.repeat,
                    })
                })
                .collect();
            let mut children = Vec::new();
            // `::before` pseudo-element. Se inyecta ANTES que el marker
            // de `<li>` y que los children reales — matchea spec ("the
            // first thing inside the box").
            if let Some(ps) =
                styles.compute_pseudo(node, crate::style::PseudoElement::Before, Some(&style))
            {
                // Aplicar reset/increment declarados en la regla del
                // pseudo (`h2::before { counter-increment: sec }`).
                // El pseudo es lo "primero adentro" del nodo, así que
                // sus contadores cuentan antes de resolver su content.
                for (name, val) in &ps.counter_reset {
                    counters.insert(name.clone(), *val);
                }
                for (name, delta) in &ps.counter_increment {
                    *counters.entry(name.clone()).or_insert(0) += *delta;
                }
                if let Some(items) = &ps.content {
                    emit_content_items(items, node, counters, &ps, base, &mut children);
                }
            }
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
            // `<iframe>` placeholder: sin engine de sub-página todavía,
            // mostramos un label con la URL para que el lector vea QUE
            // hay contenido embebido y dónde apunta.
            if tag.as_deref() == Some("iframe") {
                let src = dom::attr(node, "src").unwrap_or_default();
                let label = if src.is_empty() {
                    "[iframe sin src]".to_string()
                } else {
                    format!("[iframe: {src}]")
                };
                children.push(inline_text_with_style(label, &style));
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
                if let Some(b) = build_node(child, styles, base, Some(&style), counters) {
                    children.push(b);
                }
            }
            // `::after` pseudo-element. Se appendea al final, después
            // de los children reales. Igual que before, aplicamos
            // reset/increment del pseudo antes de resolver content.
            if let Some(ps) =
                styles.compute_pseudo(node, crate::style::PseudoElement::After, Some(&style))
            {
                for (name, val) in &ps.counter_reset {
                    counters.insert(name.clone(), *val);
                }
                for (name, delta) in &ps.counter_increment {
                    *counters.entry(name.clone()).or_insert(0) += *delta;
                }
                if let Some(items) = &ps.content {
                    emit_content_items(items, node, counters, &ps, base, &mut children);
                }
            }
            let children = strip_block_adjacent_whitespace(children, style.display);
            let children = collapse_vertical_margins(children);
            // Margin collapsing contra el padre. CSS spec: si el padre
            // no tiene border-top ni padding-top, el margin-top del
            // primer hijo block in-flow se promueve al padre (queda
            // como max(parent.margin_top, child.margin_top)). Idem
            // para el último hijo y margin-bottom. Solo aplica si el
            // padre es Block-ish (no Flex/Grid/Inline); en esos casos
            // hay un context distinto que no colapsa.
            let parent_no_top_barrier = style.padding.top == 0.0
                && style.border_widths.top == 0.0;
            let parent_no_bot_barrier = style.padding.bottom == 0.0
                && style.border_widths.bottom == 0.0;
            let parent_is_block_flow = matches!(style.display, Display::Block);
            let mut effective_margin = style.margin;
            let children = if parent_is_block_flow {
                collapse_margins_against_parent(
                    children,
                    &mut effective_margin,
                    parent_no_top_barrier,
                    parent_no_bot_barrier,
                )
            } else {
                children
            };
            Some(BoxNode {
                display: style.display,
                background: style.background,
                color: style.color,
                font_size: style.font_size,
                font_weight: style.font_weight,
                font_style: style.font_style,
                font_family: style.font_family.clone(),
                margin: effective_margin,
                margin_left_auto: style.margin_left_auto,
                margin_right_auto: style.margin_right_auto,
                padding: style.padding,
                width: style.width,
                height: style.height,
                max_width: style.max_width,
                text_align: style.text_align,
                line_height: style.line_height,
                border_widths: style.border_widths,
                border_colors: style.border_colors,
                border_radii: style.border_radii,
                border_style: style.border_style,
                hover_background,
                focus_background,
                box_shadows: style.box_shadows.clone(),
                z_index: style.z_index,
                flex_direction: style.flex_direction,
                justify_content: style.justify_content,
                align_items: style.align_items,
                align_content: style.align_content,
                justify_items: style.justify_items,
                justify_self: style.justify_self,
                flex_wrap: style.flex_wrap,
                gap_row: style.gap_row,
                gap_column: style.gap_column,
                box_sizing: style.box_sizing,
                min_width: style.min_width,
                min_height: style.min_height,
                max_height: style.max_height,
                aspect_ratio: style.aspect_ratio,
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
                object_fit: style.object_fit,
                caret_color: style.caret_color,
                accent_color: style.accent_color,
                cursor: style.cursor,
                object_position: style.object_position,
                pointer_events: style.pointer_events,
                text_indent: style.text_indent,
                word_spacing: style.word_spacing,
                letter_spacing: style.letter_spacing,
                text_shadows: style.text_shadows.clone(),
                transforms: style.transforms.clone(),
                grid_template_columns: style.grid_template_columns.clone(),
                grid_template_rows: style.grid_template_rows.clone(),
                text_decoration: style.text_decoration,
                text_decoration_color: style.text_decoration_color,
                text_decoration_style: style.text_decoration_style,
                text_decoration_thickness: style.text_decoration_thickness,
                text_underline_offset: style.text_underline_offset,
                text: None,
                children,
                tag: tag.clone(),
                link,
                image,
                details_open_attr: tag.as_deref() == Some("details")
                    && dom::attr(node, "open").is_some(),
                link_new_tab,
                link_download,
                background_image,
                background_size: style.background_size,
                background_position: style.background_position,
                background_repeat: style.background_repeat,
                background_extra_layers,
                background_origin: style.background_origin,
                background_clip: style.background_clip,
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
                canvas,
                element_id: dom::attr(node, "id").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
                class_list: dom::attr(node, "class")
                    .map(|s| {
                        s.split_whitespace()
                            .filter(|p| !p.is_empty())
                            .map(|p| p.to_string())
                            .collect()
                    })
                    .unwrap_or_default(),
                attributes: dom::all_attrs(node),
                // Resuelve `animation: <name>` contra la tabla de @keyframes
                // del stylesheet; sólo Some si el nombre matchea.
                animation: style.animation.as_ref().and_then(|b| {
                    styles
                        .keyframes()
                        .get(&b.name)
                        .map(|kf| AnimationInstance {
                            binding: b.clone(),
                            keyframes: kf.clone(),
                        })
                }),
                transitions: style.transitions.clone(),
                node_id: 0,
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
                if let Some(b) = build_node(child, styles, base, parent_style, counters) {
                    children.push(b);
                }
            }
            let children = strip_block_adjacent_whitespace(children, Display::Block);
            let children = collapse_vertical_margins(children);
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
                margin_left_auto: false,
                margin_right_auto: false,
                padding: Sides::all(0.0),
                width: LengthVal::Auto,
                height: LengthVal::Auto,
                max_width: LengthVal::Auto,
                text_align: p.text_align,
                line_height: p.line_height,
                border_widths: Sides::all(0.0),
                border_colors: Sides::all(None),
                border_radii: Corners::all(0.0),
        border_style: BorderLineStyle::Solid,
                hover_background: None,
        focus_background: None,
                box_shadows: Vec::new(),
        z_index: 0,
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::Start,
                align_items: AlignItems::Stretch,
                align_content: AlignContent::Normal,
                justify_items: None,
                justify_self: AlignSelf::Auto,
                flex_wrap: FlexWrap::NoWrap,
                gap_row: 0.0,
                gap_column: 0.0,
                box_sizing: BoxSizing::ContentBox,
                min_width: LengthVal::Auto,
                min_height: LengthVal::Auto,
                max_height: LengthVal::Auto,
                aspect_ratio: None,
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
                object_fit: None,
                caret_color: None,
                accent_color: None,
                cursor: Cursor::Auto,
                object_position: None,
                pointer_events: PointerEvents::Auto,
                text_indent: 0.0,
                word_spacing: 0.0,
                letter_spacing: 0.0,
                text_shadows: Vec::new(),
                transforms: Vec::new(),
                grid_template_columns: Vec::new(),
                grid_template_rows: Vec::new(),
                text_decoration: p.text_decoration,
                text_decoration_color: p.text_decoration_color,
                text_decoration_style: p.text_decoration_style,
                text_decoration_thickness: p.text_decoration_thickness,
                text_underline_offset: p.text_underline_offset,
                text: None,
                children,
                tag: None,
                link: None,
                image: None,
                details_open_attr: false,
                link_new_tab: false,
        link_download: None,
                background_image: None,
                background_size: BackgroundSize::Auto,
                background_position: BackgroundPosition { x: LengthVal::Pct(0.0), y: LengthVal::Pct(0.0) },
                background_repeat: BackgroundRepeat::Repeat,
                background_extra_layers: Vec::new(),
                background_origin: BackgroundOrigin::PaddingBox,
                background_clip: BackgroundClip::BorderBox,
                input_kind: None,
                input_initial: None,
                input_placeholder: None,
        input_name: None,
        input_checked_initial: false,
        input_autofocus: false,
        form_idx: None,
        select: None,
        svg: None,
        canvas: None,
        element_id: None,
        class_list: Vec::new(),
        attributes: Vec::new(),
                animation: None,
                transitions: Vec::new(),
                node_id: 0,
            })
        }
    }
}

/// Construye un nodo Text inline con el color/font/text-align/line-height
/// del estilo dado — usado tanto por hojas Text reales como por los
/// markers sintéticos (`•` de `<li>`, `[img: alt]` de `<img>` roto).
pub(crate) fn inline_text_with_style(s: String, style: &ComputedStyle) -> BoxNode {
    let mut leaf = BoxNode {
        display: Display::Inline,
        background: None,
        color: style.color,
        font_size: style.font_size,
        font_weight: style.font_weight,
        font_style: style.font_style,
        font_family: style.font_family.clone(),
        margin: Sides::all(0.0),
        margin_left_auto: false,
        margin_right_auto: false,
        padding: Sides::all(0.0),
        width: LengthVal::Auto,
        height: LengthVal::Auto,
        max_width: LengthVal::Auto,
        text_align: style.text_align,
        line_height: style.line_height,
        border_widths: Sides::all(0.0),
        border_colors: Sides::all(None),
        border_radii: Corners::all(0.0),
        border_style: BorderLineStyle::Solid,
        hover_background: None,
        focus_background: None,
        box_shadows: Vec::new(),
        z_index: 0,
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::Start,
        align_items: AlignItems::Stretch,
        align_content: AlignContent::Normal,
        justify_items: None,
        justify_self: AlignSelf::Auto,
        flex_wrap: FlexWrap::NoWrap,
        gap_row: 0.0,
        gap_column: 0.0,
        box_sizing: BoxSizing::ContentBox,
        min_width: LengthVal::Auto,
        min_height: LengthVal::Auto,
        max_height: LengthVal::Auto,
        aspect_ratio: None,
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
        object_fit: None,
        caret_color: None,
        accent_color: None,
        cursor: Cursor::Auto,
        object_position: None,
        pointer_events: PointerEvents::Auto,
        text_indent: 0.0,
        word_spacing: 0.0,
        letter_spacing: 0.0,
        text_shadows: Vec::new(),
        transforms: Vec::new(),
        grid_template_columns: Vec::new(),
        grid_template_rows: Vec::new(),
        text_decoration: style.text_decoration,
        text_decoration_color: style.text_decoration_color,
        text_decoration_style: style.text_decoration_style,
        text_decoration_thickness: style.text_decoration_thickness,
        text_underline_offset: style.text_underline_offset,
        text: Some(s),
        children: Vec::new(),
        tag: None,
        link: None,
        image: None,
        details_open_attr: false,
        link_new_tab: false,
        link_download: None,
        background_image: None,
        background_size: BackgroundSize::Auto,
        background_position: BackgroundPosition { x: LengthVal::Pct(0.0), y: LengthVal::Pct(0.0) },
        background_repeat: BackgroundRepeat::Repeat,
        background_extra_layers: Vec::new(),
        background_origin: BackgroundOrigin::PaddingBox,
        background_clip: BackgroundClip::BorderBox,
        input_kind: None,
        input_initial: None,
        input_placeholder: None,
        input_name: None,
        input_checked_initial: false,
        input_autofocus: false,
        form_idx: None,
        select: None,
        svg: None,
        canvas: None,
        element_id: None,
        class_list: Vec::new(),
        attributes: Vec::new(),
        animation: None,
        transitions: Vec::new(),
        node_id: 0,
    };
    // `background-clip: text` (Fase 7.208): el gradiente vive en el elemento
    // estilado (p. ej. el `<h1>`), pero las glifos están en esta hoja de texto
    // hija. Propagamos el clip + el gradiente para rellenar los glifos con él;
    // el `color: transparent` típico del patrón deja ver sólo el gradiente.
    if style.background_clip == BackgroundClip::Text {
        leaf.background_clip = BackgroundClip::Text;
        leaf.background_gradient = style.background_gradient.clone();
    }
    leaf
}

/// `true` si el nodo se comporta como block-level para el flujo (Block,
/// Flex, Grid, None). `Inline*` queda fuera — son del flow inline.
pub(crate) fn is_block_level(b: &BoxNode) -> bool {
    !matches!(
        b.display,
        Display::Inline | Display::InlineBlock | Display::InlineFlex | Display::InlineGrid
    )
}

/// `true` si el nodo es un leaf de texto inline cuyo contenido se reduce
/// a whitespace (incluye el caso post-collapse del CSS, que deja " "
/// como "espacio entre tokens"). `<br>` y otros inlines sin texto no
/// matchean (b.text es None).
pub(crate) fn is_ws_only_inline(b: &BoxNode) -> bool {
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
pub(crate) fn strip_block_adjacent_whitespace(
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
    let ws_only: Vec<bool> = children.iter().map(is_ws_only_inline).collect();
    let n = children.len();
    // Para cada nodo whitespace-only, buscamos el primer vecino no-ws
    // (antes y después). Si ambos son block-level (o son edge), drop —
    // la run entera de whitespace entre dos blocks no aporta nada
    // visual. Antes mirábamos sólo el vecino inmediato, lo que dejaba
    // que runs consecutivas se preservaran al final del body
    // ("<blockquote>X</blockquote>  \n  ").
    let mut out = Vec::with_capacity(n);
    for (i, c) in children.into_iter().enumerate() {
        if ws_only[i] {
            let prev_is_block_or_edge = {
                let mut j = i;
                loop {
                    if j == 0 {
                        break true;
                    }
                    j -= 1;
                    if !ws_only[j] {
                        break block_levels[j];
                    }
                }
            };
            let next_is_block_or_edge = {
                let mut j = i + 1;
                loop {
                    if j >= n {
                        break true;
                    }
                    if !ws_only[j] {
                        break block_levels[j];
                    }
                    j += 1;
                }
            };
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
/// Materializa los `ContentItem`s del `content:` del pseudo en boxes
/// hijos. Strings/counters/attrs adjacentes se concatenan en un
/// `inline_text_with_style`; cada `Url` genera un `<img>` sintético
/// inline-block. Orden preservado. Items que producen string vacía o
/// urls que fallan al decode se omiten silenciosamente.
pub(crate) fn emit_content_items(
    items: &[crate::style::ContentItem],
    node: &Handle,
    counters: &std::collections::HashMap<String, i32>,
    pseudo_style: &ComputedStyle,
    base: Option<&url::Url>,
    out: &mut Vec<BoxNode>,
) {
    use crate::style::ContentItem;
    let mut text_buf = String::new();
    let flush_text = |buf: &mut String, out: &mut Vec<BoxNode>| {
        if !buf.is_empty() {
            out.push(inline_text_with_style(std::mem::take(buf), pseudo_style));
        }
    };
    for it in items {
        match it {
            ContentItem::Text(s) => text_buf.push_str(s),
            ContentItem::Counter(name) => {
                let v = counters.get(name).copied().unwrap_or(0);
                text_buf.push_str(&v.to_string());
            }
            ContentItem::Attr(name) => {
                if let Some(v) = dom::attr(node, name) {
                    text_buf.push_str(&v);
                }
            }
            ContentItem::Url(u) => {
                flush_text(&mut text_buf, out);
                if let Some(abs) = resolve_href(base, u) {
                    if let Some(img) = fetch_and_decode(&abs) {
                        out.push(synthetic_image_box(img, pseudo_style));
                    }
                    // Si fetch/decode falla, lo omitimos (matchea CSS
                    // spec: url() inválido suprime la generación).
                }
            }
        }
    }
    flush_text(&mut text_buf, out);
}

/// Construye un BoxNode inline-block con una imagen ya decodificada,
/// hereda el estilo del pseudo. Se usa para `content: url(...)`.
pub(crate) fn synthetic_image_box(img: ImageData, style: &ComputedStyle) -> BoxNode {
    let mut b = inline_text_with_style(String::new(), style);
    b.display = Display::InlineBlock;
    b.image = Some(img);
    b.text = None;
    b
}

/// Margin collapsing contra el padre. CSS spec:
/// - Si el padre NO tiene border-top ni padding-top, el margin-top
///   del primer hijo block in-flow "se ve" como parte del padre —
///   se promueve y queda en `max(parent.margin_top, child.margin_top)`.
///   El hijo se setea a 0 para evitar doble cuenta.
/// - Idem para el último hijo y bottom.
///
/// Esto destraba el caso típico: `body { margin: 8px }` con un primer
/// `<h1 style="margin: 21px 0">` — sin collapse el body tiene 8px +
/// 21px = 29px arriba; con collapse, max(8, 21) = 21px, que es lo que
/// hacen los browsers reales.
pub(crate) fn collapse_margins_against_parent(
    mut children: Vec<BoxNode>,
    parent_margin: &mut Sides<f32>,
    no_top_barrier: bool,
    no_bot_barrier: bool,
) -> Vec<BoxNode> {
    if no_top_barrier {
        if let Some(first) = children.first_mut() {
            if is_block_level(first) && first.margin.top > 0.0 {
                parent_margin.top = parent_margin.top.max(first.margin.top);
                first.margin.top = 0.0;
            }
        }
    }
    if no_bot_barrier {
        if let Some(last) = children.last_mut() {
            if is_block_level(last) && last.margin.bottom > 0.0 {
                parent_margin.bottom = parent_margin.bottom.max(last.margin.bottom);
                last.margin.bottom = 0.0;
            }
        }
    }
    children
}

/// Margin collapsing CSS — entre hermanos block adyacentes, el gap
/// vertical es `max(prev.margin_bottom, next.margin_top)` (NO la suma).
/// Sin esto, raw HTML pages como motherfucking se ven con gaps el
/// doble entre `<h2>` y `<p>` consecutivos. Implementación simple:
/// para cada par (block, block) consecutivo, restamos del margin_top
/// del segundo el min(prev.margin_bottom, next.margin_top). El total
/// `prev.margin_bottom + next.margin_top_modificado` queda igual a
/// `max(prev.margin_bottom, next.margin_top)`.
///
/// Casos NO cubiertos (queda para una iteración más completa):
/// - Collapse con el padre (cuando primer/último hijo block no tiene
///   padding/border arriba/abajo, su margin colapsa contra el padre).
/// - Negative margins (CSS spec dice que se tratan separadamente).
/// - Through-block collapsing en blocks vacíos.
pub(crate) fn collapse_vertical_margins(children: Vec<BoxNode>) -> Vec<BoxNode> {
    if children.len() < 2 {
        return children;
    }
    let mut out: Vec<BoxNode> = Vec::with_capacity(children.len());
    for c in children {
        if let Some(prev) = out.last() {
            if is_block_level(prev) && is_block_level(&c) {
                let prev_bot = prev.margin.bottom.max(0.0);
                let next_top = c.margin.top.max(0.0);
                let reduction = prev_bot.min(next_top);
                if reduction > 0.0 {
                    let mut adjusted = c;
                    adjusted.margin.top -= reduction;
                    out.push(adjusted);
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

/// Workers paralelos para el prefetch. 6 es un compromiso razonable:
/// alto enough para esconder latencia de TCP/TLS (cada handshake ~50-
/// 200ms), bajo enough para no saturar servidores ni el ulimit de
/// sockets del proceso. Browsers reales usan 6-8 por host.
const PREFETCH_WORKERS: usize = 6;

/// Pre-walk del DOM coleccionando URLs absolutas de `<img src>`,
/// `<img srcset>`, `<picture><source srcset>`, y disparando descargas
/// paralelas. La cache global de bytes guarda los resultados —
/// `fetch_and_decode` en `build_node` después hace cache hit.
pub(crate) fn prefetch_image_urls(root: &Handle, base: Option<&url::Url>) {
    let mut urls: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut push = |u: String| {
        if seen.insert(u.clone()) {
            urls.push(u);
        }
    };
    dom::walk(root, &mut |node| {
        let tag = dom::element_name(node);
        match tag.as_deref() {
            Some("img") => {
                if let Some(src) = pick_srcset(&dom::attr(node, "srcset").unwrap_or_default())
                    .or_else(|| dom::attr(node, "src"))
                {
                    if let Some(abs) = resolve_href(base, &src) {
                        push(abs);
                    }
                }
            }
            Some("picture") => {
                for child in node.children.borrow().iter() {
                    if dom::element_name(child).as_deref() == Some("source") {
                        if let Some(s) = dom::attr(child, "srcset") {
                            if let Some(c) = pick_srcset(&s) {
                                if let Some(abs) = resolve_href(base, &c) {
                                    push(abs);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    });
    if urls.is_empty() {
        return;
    }
    // Cache hits no necesitan fetch; los filtramos para ahorrar threads.
    // Además filtramos schemes no-HTTP (`about:`, `file:`, `data:`) —
    // ureq haría un round-trip al timeout para nada.
    let pending: Vec<String> = urls
        .into_iter()
        .filter(|u| {
            url::Url::parse(u)
                .ok()
                .map(|p| matches!(p.scheme(), "http" | "https"))
                .unwrap_or(false)
        })
        .filter(|u| crate::cache::get(u).is_none())
        .collect();
    if pending.is_empty() {
        return;
    }
    // Pool simple: dividir las URLs en chunks de tamaño ceil(N/W) y un
    // thread por chunk. Más simple que un channel + N workers, y para
    // 6-30 URLs típicas de una página el balance es suficiente.
    let chunk_size = pending.len().div_ceil(PREFETCH_WORKERS).max(1);
    let mut handles = Vec::new();
    for chunk in pending.chunks(chunk_size) {
        let chunk = chunk.to_vec();
        handles.push(std::thread::spawn(move || {
            for url in chunk {
                // Best-effort: errores se ignoran. El build_node
                // posterior los reintentará serializado y muestra el
                // alt del `<img>` si igual falla.
                let _ = crate::fetch::fetch_bytes(&url);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
}

/// Segundo pass de prefetch: recolecta URLs de `background-image:
/// url(...)` después de computar styles. Reusa el mismo pool de
/// workers que `prefetch_image_urls`. Computamos sin parent porque
/// `background-image` no se hereda y los valores son independientes
/// del contexto del padre (cosa que sí valdría para `color` o
/// `font-size`).
pub(crate) fn prefetch_background_image_urls(
    root: &Handle,
    styles: &StyleEngine,
    base: Option<&url::Url>,
) {
    let mut urls: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    dom::walk(root, &mut |node| {
        if !matches!(node.data, markup5ever_rcdom::NodeData::Element { .. }) {
            return;
        }
        let style = styles.compute(node);
        let mut push = |u: &str| {
            if let Some(abs) = resolve_href(base, u) {
                if seen.insert(abs.clone()) {
                    urls.push(abs);
                }
            }
        };
        if let Some(u) = style.background_image_url.as_deref() {
            push(u);
        }
        // Capas extra (lista `background: a, b`): prefetch sus url() también.
        for l in &style.background_extra_layers {
            if let crate::style::BackgroundImage::Url(u) = &l.image {
                push(u);
            }
        }
    });
    if urls.is_empty() {
        return;
    }
    let pending: Vec<String> = urls
        .into_iter()
        .filter(|u| {
            url::Url::parse(u)
                .ok()
                .map(|p| matches!(p.scheme(), "http" | "https"))
                .unwrap_or(false)
        })
        .filter(|u| crate::cache::get(u).is_none())
        .collect();
    if pending.is_empty() {
        return;
    }
    let chunk_size = pending.len().div_ceil(PREFETCH_WORKERS).max(1);
    let mut handles = Vec::new();
    for chunk in pending.chunks(chunk_size) {
        let chunk = chunk.to_vec();
        handles.push(std::thread::spawn(move || {
            for url in chunk {
                let _ = crate::fetch::fetch_bytes(&url);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
}

pub(crate) fn fetch_and_decode(url: &str) -> Option<ImageData> {
    let bytes = crate::fetch::fetch_bytes(url).ok()?;
    decode_image_bytes(&bytes)
}

/// Decodifica bytes de imagen (PNG/JPEG por las features de `image`) a RGBA8.
/// `None` si el formato no está habilitado o el decode falla.
pub(crate) fn decode_image_bytes(bytes: &[u8]) -> Option<ImageData> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.format()?; // formato no habilitado por features → None
    let img = reader.decode().ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    Some(ImageData { rgba: rgba.into_raw(), width, height })
}

/// Resuelve+decodifica la imagen de un `src`/`srcset`/`background-image`.
/// Los `data:` URLs se decodifican inline (RFC 2397) — `resolve_href` los
/// bloquea a propósito (no son navegables como `<a href>`), pero como fuente
/// de un recurso son legítimos. El resto resuelve contra `base` y baja por
/// HTTP/file. `None` si falta src o falla la decodificación.
pub fn fetch_image_src(base: Option<&url::Url>, src: &str) -> Option<ImageData> {
    if crate::fetch::is_data_url(src.trim()) {
        return decode_image_bytes(&crate::fetch::decode_data_url(src.trim())?);
    }
    let abs = resolve_href(base, src)?;
    fetch_and_decode(&abs)
}

/// Colapso de whitespace según `white-space`:
/// - `Normal` / `NoWrap`: runs internos → un espacio, leading/trailing
///   reducidos a uno; newlines colapsan igual.
/// - `Pre`: todo preservado.
/// - `PreWrap`: igual que Pre — el wrap es responsabilidad del layout.
/// - `PreLine`: runs de espacio/tab → un espacio, newlines preservados.
pub(crate) fn collapse_whitespace(s: &str, ws: WhiteSpace) -> String {
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
pub(crate) fn apply_text_transform(s: String, t: TextTransform) -> String {
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
pub(crate) fn li_marker(node: &Handle, kind: ListStyleType) -> Option<String> {
    match kind {
        ListStyleType::None => None,
        ListStyleType::Disc => Some("• ".into()),
        ListStyleType::Circle => Some("◦ ".into()),
        ListStyleType::Square => Some("▪ ".into()),
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
pub(crate) fn ol_item_position(node: &Handle) -> i32 {
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
pub(crate) fn parent_handle(node: &Handle) -> Option<Handle> {
    let weak = node.parent.take();
    let restored = weak.clone();
    node.parent.set(restored);
    weak.and_then(|w| w.upgrade())
}

/// Convierte 1..N a alpha bijectiva base-26 (1=a, 26=z, 27=aa, 28=ab…).
/// Valores `<= 0` caen a `"0"` — el marker numérico igual se imprime.
pub(crate) fn to_alpha(mut n: i32, upper: bool) -> String {
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
pub(crate) fn to_roman(n: i32, upper: bool) -> String {
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

pub(crate) fn resolve_href(base: Option<&url::Url>, href: &str) -> Option<String> {
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
