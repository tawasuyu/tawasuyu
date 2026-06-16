//! Construcción recursiva del árbol: `empty_root` y `build_node` (walk DOM→BoxNode).
//! Extraído de `boxes/build.rs` (regla #1). Sin cambios de lógica.
use super::*;

/// `LengthVal` → `(px, pct)` para el centro de un clip-path elíptico. Sólo
/// `Px`/`Pct` portan valor; el resto (Auto/content) cae a `(0, 0)` ⇒ borde
/// izquierdo/superior. El compositor resuelve `px + pct/100·dim`. Fase 7.1220.
fn clip_len_pair(v: LengthVal) -> (f32, f32) {
    match v {
        LengthVal::Px(p) => (p, 0.0),
        LengthVal::Pct(p) => (0.0, p),
        _ => (0.0, 0.0),
    }
}

/// Base contra la que resuelve un `%` de radio de clip-path: ancho, alto o
/// la diagonal `√(w²+h²)/√2` (circle). Fase 7.1221.
enum RadBasis {
    Width,
    Height,
    Diag,
}

/// `LengthVal` → cuádruple `[px, pct_w, pct_h, pct_diag]` para un radio de
/// clip-path. Un `%` cae en la ranura de su base; un px en `px`. Exactamente
/// una ranura de pct queda no-cero. El compositor suma las cuatro
/// contribuciones contra el rect. Fase 7.1221.
fn clip_radius_quad(v: LengthVal, basis: RadBasis) -> [f32; 4] {
    let (px, pct) = clip_len_pair(v);
    match basis {
        RadBasis::Width => [px, pct, 0.0, 0.0],
        RadBasis::Height => [px, 0.0, pct, 0.0],
        RadBasis::Diag => [px, 0.0, 0.0, pct],
    }
}

/// `ClipRadius` → quíntuple `[px, pct_w, pct_h, pct_diag, side]` para un radio
/// de clip-path. `Len` delega en `clip_radius_quad` con `side = 0`. Un keyword
/// de lado pone px/pct en 0 y codifica `side`: `circle` ⇒ `1`=closest /
/// `2`=farthest (base 4 lados); `ellipse` ⇒ `3`=closest / `4`=farthest (base
/// eje). El compositor decide la geometría según `side`. Fase 7.1222.
fn clip_radius_quint(r: crate::style::ClipRadius, basis: RadBasis, circle: bool) -> [f32; 5] {
    use crate::style::ClipRadius;
    match r {
        ClipRadius::Len(v) => {
            let q = clip_radius_quad(v, basis);
            [q[0], q[1], q[2], q[3], 0.0]
        }
        ClipRadius::ClosestSide => [0.0, 0.0, 0.0, 0.0, if circle { 1.0 } else { 3.0 }],
        ClipRadius::FarthestSide => [0.0, 0.0, 0.0, 0.0, if circle { 2.0 } else { 4.0 }],
    }
}

/// `true` si el padre establece un contexto flex/grid — único caso en que
/// `margin-top/bottom: auto` centra (en block flow CSS lo computa a 0).
fn parent_is_flex_grid(p: Option<&ComputedStyle>) -> bool {
    matches!(
        p.map(|s| s.display),
        Some(Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid)
    )
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
        margin_top_auto: false,
        margin_bottom_auto: false,
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
        clip_inset: None,
        clip_ellipse: None,
        clip_polygon: None,
        clip_path_svg: None,
        clip_ref_inset: None,
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
        text_overflow: TextOverflow::Clip,
        scroll_behavior: ScrollBehavior::Auto,
        tab_size: TabSize::Chars(8),
        user_select: UserSelect::Auto,
        overflow_wrap: OverflowWrap::Normal,
        word_break: WordBreak::Normal,
        hyphens: Hyphens::Manual,
        resize: Resize::None,
        writing_mode: WritingMode::HorizontalTb,
        direction: Direction::Ltr,
        unicode_bidi: UnicodeBidi::Normal,
        font_stretch: 1.0,
        image_rendering: ImageRendering::Auto,
        mix_blend_mode: BlendMode::Normal,
        background_blend_mode: Vec::new(),
        isolation: Isolation::Auto,
        will_change: Vec::new(),
        appearance: Appearance::Auto,
        font_kerning: FontKerning::Auto,
        font_feature_settings: Vec::new(),
        font_variation_settings: Vec::new(),
        font_language_override: None,
        text_rendering: TextRendering::Auto,
        filter: Vec::new(),
        backdrop_filter: Vec::new(),
        text_orientation: TextOrientation::Mixed,
        overscroll_behavior_x: OverscrollBehavior::Auto,
        overscroll_behavior_y: OverscrollBehavior::Auto,
        scroll_snap_type: ScrollSnapType(None),
        object_position: None,
        pointer_events: PointerEvents::Auto,
        text_indent: 0.0,
        word_spacing: 0.0,
        letter_spacing: 0.0,
        text_shadows: Vec::new(),
        transforms: Vec::new(),
        grid_template_columns: Vec::new(),
        grid_template_rows: Vec::new(),
        grid_auto_flow: GridAutoFlow::Row,
        grid_auto_columns: Vec::new(),
        grid_auto_rows: Vec::new(),
        grid_template_areas: None,
        grid_row_start: None,
        grid_row_end: None,
        grid_column_start: None,
        grid_column_end: None,
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
        mask_image: None,
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
            // `mask-image: url(...)` — misma cache/decoder que background-image.
            // El compositor la aplica como máscara de luminancia sobre el
            // subárbol, resolviendo size/position/repeat contra el rect igual
            // que background. El encaje viaja con la imagen. Falla silenciosa →
            // mask_image queda None. Fase 7.1226 (pintado), 7.1227 (encaje).
            let mask_image = match &style.mask_image {
                Some(crate::style::MaskImage::Url(u)) => fetch_image_src(base, u).map(|img| {
                    (
                        img,
                        style.mask_size,
                        style.mask_position,
                        style.mask_repeat,
                        style.mask_mode,
                    )
                }),
                None => None,
            };
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
                if let Some(marker) =
                    li_marker(node, &style.list_style_type, styles.counter_styles())
                {
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
                // Resolución contra el contexto: el auto vertical sólo centra
                // si el padre es flex/grid (block flow → 0).
                margin_top_auto: style.margin_top_auto && parent_is_flex_grid(parent_style),
                margin_bottom_auto: style.margin_bottom_auto && parent_is_flex_grid(parent_style),
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
                // Fase 7.1219 — clip-path: inset(...) → insets px desde el
                // border box. Formas no rectangulares no se modelan acá.
                clip_inset: match &style.clip_path {
                    Some(crate::style::ClipPath::Inset { top, right, bottom, left, .. }) => {
                        Some([*top, *right, *bottom, *left])
                    }
                    _ => None,
                },
                // Fase 7.1220/7.1221/7.1222 — clip-path: circle()/ellipse() →
                // spec elíptico de 14 floats: centro [cx_px, cx_pct, cy_px,
                // cy_pct] + dos radios [px, pct_w, pct_h, pct_diag, side]. El %
                // y los lados se difieren al compositor (dependen del rect).
                // circle ⇒ rx == ry sobre base diagonal/4-lados; ellipse rx
                // sobre ancho/eje-x, ry sobre alto/eje-y.
                clip_ellipse: match &style.clip_path {
                    Some(crate::style::ClipPath::Circle { radius, cx, cy }) => {
                        let (cxp, cxc) = clip_len_pair(*cx);
                        let (cyp, cyc) = clip_len_pair(*cy);
                        let r = clip_radius_quint(*radius, RadBasis::Diag, true);
                        Some([
                            cxp, cxc, cyp, cyc, r[0], r[1], r[2], r[3], r[4], r[0], r[1], r[2],
                            r[3], r[4],
                        ])
                    }
                    Some(crate::style::ClipPath::Ellipse { rx, ry, cx, cy }) => {
                        let (cxp, cxc) = clip_len_pair(*cx);
                        let (cyp, cyc) = clip_len_pair(*cy);
                        let rx = clip_radius_quint(*rx, RadBasis::Width, false);
                        let ry = clip_radius_quint(*ry, RadBasis::Height, false);
                        Some([
                            cxp, cxc, cyp, cyc, rx[0], rx[1], rx[2], rx[3], rx[4], ry[0], ry[1],
                            ry[2], ry[3], ry[4],
                        ])
                    }
                    _ => None,
                },
                // Fase 7.1223 — clip-path: polygon(...) → (evenodd, puntos) con
                // cada punto [x_px, x_pct, y_px, y_pct]; el % se difiere al
                // compositor (depende del rect).
                clip_polygon: match &style.clip_path {
                    Some(crate::style::ClipPath::Polygon { evenodd, points }) => {
                        let pts = points
                            .iter()
                            .map(|(x, y)| {
                                let (xp, xc) = clip_len_pair(*x);
                                let (yp, yc) = clip_len_pair(*y);
                                [xp, xc, yp, yc]
                            })
                            .collect();
                        Some((*evenodd, pts))
                    }
                    _ => None,
                },
                // Fase 7.1224 — clip-path: path(...) → (evenodd, d) con el
                // string SVG crudo (lo parsea el compositor).
                clip_path_svg: match &style.clip_path {
                    Some(crate::style::ClipPath::Path { evenodd, d }) => {
                        Some((*evenodd, d.clone()))
                    }
                    _ => None,
                },
                // Fase 7.1225 — caja de referencia de clip-path → insets del
                // border-box. padding-box = border; content-box = border +
                // padding; border/margin-box = None (sin cambio).
                clip_ref_inset: match style.clip_geometry_box {
                    crate::style::GeometryBox::PaddingBox => Some([
                        style.border_widths.top,
                        style.border_widths.right,
                        style.border_widths.bottom,
                        style.border_widths.left,
                    ]),
                    crate::style::GeometryBox::ContentBox => Some([
                        style.border_widths.top + style.padding.top,
                        style.border_widths.right + style.padding.right,
                        style.border_widths.bottom + style.padding.bottom,
                        style.border_widths.left + style.padding.left,
                    ]),
                    _ => None,
                },
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
                text_overflow: style.text_overflow,
                scroll_behavior: style.scroll_behavior,
                tab_size: style.tab_size,
                user_select: style.user_select,
                overflow_wrap: style.overflow_wrap,
                word_break: style.word_break,
                hyphens: style.hyphens,
                resize: style.resize,
                writing_mode: style.writing_mode,
                direction: style.direction,
                unicode_bidi: style.unicode_bidi,
                font_stretch: style.font_stretch,
                image_rendering: style.image_rendering,
                mix_blend_mode: style.mix_blend_mode,
                background_blend_mode: style.background_blend_mode.clone(),
                isolation: style.isolation,
                will_change: style.will_change.clone(),
                appearance: style.appearance,
                font_kerning: style.font_kerning,
                font_feature_settings: style.font_feature_settings.clone(),
                font_variation_settings: style.font_variation_settings.clone(),
                font_language_override: style.font_language_override.clone(),
                text_rendering: style.text_rendering,
                filter: style.filter.clone(),
                backdrop_filter: style.backdrop_filter.clone(),
                text_orientation: style.text_orientation,
                overscroll_behavior_x: style.overscroll_behavior_x,
                overscroll_behavior_y: style.overscroll_behavior_y,
                scroll_snap_type: style.scroll_snap_type,
                object_position: style.object_position,
                pointer_events: style.pointer_events,
                text_indent: style.text_indent,
                word_spacing: style.word_spacing,
                letter_spacing: style.letter_spacing,
                text_shadows: style.text_shadows.clone(),
                transforms: style.transforms.clone(),
                grid_template_columns: style.grid_template_columns.clone(),
                grid_template_rows: style.grid_template_rows.clone(),
                grid_auto_flow: style.grid_auto_flow,
                grid_auto_columns: style.grid_auto_columns.clone(),
                grid_auto_rows: style.grid_auto_rows.clone(),
                grid_template_areas: style.grid_template_areas.clone(),
                grid_row_start: style.grid_row_start.clone(),
                grid_row_end: style.grid_row_end.clone(),
                grid_column_start: style.grid_column_start.clone(),
                grid_column_end: style.grid_column_end.clone(),
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
                mask_image,
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
                margin_top_auto: false,
                margin_bottom_auto: false,
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
                clip_inset: None,
                clip_ellipse: None,
                clip_polygon: None,
                clip_path_svg: None,
                clip_ref_inset: None,
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
                text_overflow: TextOverflow::Clip,
                scroll_behavior: ScrollBehavior::Auto,
                tab_size: TabSize::Chars(8),
                user_select: UserSelect::Auto,
                overflow_wrap: OverflowWrap::Normal,
                word_break: WordBreak::Normal,
                hyphens: Hyphens::Manual,
                resize: Resize::None,
                writing_mode: WritingMode::HorizontalTb,
                direction: Direction::Ltr,
                unicode_bidi: UnicodeBidi::Normal,
                font_stretch: 1.0,
                image_rendering: ImageRendering::Auto,
                mix_blend_mode: BlendMode::Normal,
                background_blend_mode: Vec::new(),
                isolation: Isolation::Auto,
                will_change: Vec::new(),
                appearance: Appearance::Auto,
                font_kerning: FontKerning::Auto,
                font_feature_settings: Vec::new(),
                font_variation_settings: Vec::new(),
                font_language_override: None,
                text_rendering: TextRendering::Auto,
                filter: Vec::new(),
                backdrop_filter: Vec::new(),
                text_orientation: TextOrientation::Mixed,
                overscroll_behavior_x: OverscrollBehavior::Auto,
                overscroll_behavior_y: OverscrollBehavior::Auto,
                scroll_snap_type: ScrollSnapType(None),
                object_position: None,
                pointer_events: PointerEvents::Auto,
                text_indent: 0.0,
                word_spacing: 0.0,
                letter_spacing: 0.0,
                text_shadows: Vec::new(),
                transforms: Vec::new(),
                grid_template_columns: Vec::new(),
                grid_template_rows: Vec::new(),
                grid_auto_flow: GridAutoFlow::Row,
                grid_auto_columns: Vec::new(),
                grid_auto_rows: Vec::new(),
                grid_template_areas: None,
                grid_row_start: None,
                grid_row_end: None,
                grid_column_start: None,
                grid_column_end: None,
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
                mask_image: None,
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

