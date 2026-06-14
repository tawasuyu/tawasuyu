//! Texto inline, niveles de bloque, recorte de whitespace, content-items, colapso de márgenes.
//! Extraído de `boxes/build.rs` (regla #1). Sin cambios de lógica.
use super::*;

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
        grid_auto_flow: style.grid_auto_flow,
        grid_auto_columns: style.grid_auto_columns.clone(),
        grid_auto_rows: style.grid_auto_rows.clone(),
        grid_row_start: style.grid_row_start.clone(),
        grid_row_end: style.grid_row_end.clone(),
        grid_column_start: style.grid_column_start.clone(),
        grid_column_end: style.grid_column_end.clone(),
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

