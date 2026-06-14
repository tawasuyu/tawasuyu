use super::*;

pub(crate) fn box_style(b: &BoxNode, zoom: f32) -> Style {
    // Las hojas de texto se miden con parley en el runtime
    // (`compute_with_measure`): taffy reserva el alto real del texto
    // envuelto (N líneas) en lugar de una sola. Por eso dejamos su height
    // en `auto` — si lo fijáramos a una línea, los párrafos que envuelven
    // se aplastarían unos sobre otros. Mantenemos `line_h` como piso
    // (min_height) para que un nodo de texto vacío no colapse a cero.
    let is_text_leaf = b.text.is_some();
    let lh_mult = b.line_height.unwrap_or(1.2);
    let line_h = b.font_size * lh_mult * zoom;

    let is_flex = matches!(b.display, Display::Flex | Display::InlineFlex);

    let is_grid = matches!(b.display, Display::Grid | Display::InlineGrid);

    // Defaults según display: Block fila completa columnar, Inline en row
    // con altura auto, Flex toma sus props del nodo. None: cero.
    let (default_direction, mut width, mut height) = match b.display {
        Display::Block => (FlexDirection::Column, percent(1.0_f32), auto()),
        Display::Flex => (map_flex_direction(b.flex_direction), percent(1.0_f32), auto()),
        Display::InlineFlex => (map_flex_direction(b.flex_direction), auto(), auto()),
        Display::Grid => (FlexDirection::Column, percent(1.0_f32), auto()),
        Display::InlineGrid => (FlexDirection::Column, auto(), auto()),
        Display::InlineBlock | Display::Inline => {
            // Texto: height auto → lo dimensiona la medición con parley.
            (FlexDirection::Row, auto(), auto())
        }
        Display::None => (FlexDirection::Column, length(0.0_f32), length(0.0_f32)),
    };

    // Para bloques con hijos inline conmutamos a Row + Wrap (igual que
    // antes — el hack original que hace que `<p>` flowee tokens). Para
    // Flex respetamos las props del autor sin tocar.
    let block_inline_wrap =
        matches!(b.display, Display::Block) && has_inline_children(b);

    let flex_wrap = if is_flex {
        map_flex_wrap(b.flex_wrap)
    } else if block_inline_wrap {
        FlexWrap::Wrap
    } else {
        FlexWrap::NoWrap
    };

    let (flex_direction, w_base) = if block_inline_wrap {
        (FlexDirection::Row, percent(1.0_f32))
    } else {
        (default_direction, width)
    };
    width = w_base;

    // CSS `width` explícito gana sobre el default de display.
    if let Some(explicit) = length_to_taffy(b.width, zoom) {
        width = explicit;
    }
    // CSS `height` explícito gana sobre el default (auto = lo dimensiona el
    // contenido). Los % de height sólo resuelven si el padre tiene altura
    // definida — taffy lo maneja igual que en un browser.
    if let Some(explicit) = length_to_taffy(b.height, zoom) {
        height = explicit;
    }
    let max_size = Size {
        width: length_to_taffy(b.max_width, zoom).unwrap_or_else(auto),
        height: length_to_taffy(b.max_height, zoom).unwrap_or_else(auto),
    };
    let min_size = Size {
        width: length_to_taffy(b.min_width, zoom).unwrap_or_else(|| length(0.0_f32)),
        height: length_to_taffy(b.min_height, zoom).unwrap_or_else(|| {
            // Piso de una línea para hojas de texto (el resto: 0).
            if is_text_leaf { length(line_h) } else { length(0.0_f32) }
        }),
    };

    // justify/align: flex y grid los toman del autor. En grid, `justify-content`
    // distribuye las PISTAS en el eje inline cuando no llenan el contenedor, y
    // `align-items` fija la alineación default de los ítems dentro de su celda
    // (eje de bloque). Para bloques con inlines derivamos `justify_content` de
    // `text-align` (comportamiento heredado). Los defaults del engine
    // (Start/Stretch) coinciden con los de taffy, así que esto sólo altera el
    // layout cuando el autor pone un valor explícito.
    let justify_content = if is_flex || is_grid {
        Some(map_justify(b.justify_content))
    } else if block_inline_wrap {
        match b.text_align {
            TextAlign::Left | TextAlign::Justify => None,
            TextAlign::Center => Some(JustifyContent::Center),
            TextAlign::Right => Some(JustifyContent::End),
        }
    } else {
        None
    };

    let align_items = if is_flex || is_grid {
        Some(map_align(b.align_items))
    } else {
        None
    };

    // align-content: distribución de líneas (flex multilínea) / pistas
    // (grid) en el eje cruzado. Aplica tanto a flex como a grid; `Normal`
    // deja el default de taffy (None ≈ stretch).
    let align_content = if is_flex || is_grid {
        map_align_content(b.align_content)
    } else {
        None
    };

    // gap: aplica a flex Y a grid. Taffy lo expone como
    // `Size { width: column-gap, height: row-gap }`.
    let gap = if is_flex || is_grid {
        Size {
            width: length(b.gap_column * zoom),
            height: length(b.gap_row * zoom),
        }
    } else {
        Size { width: length(0.0_f32), height: length(0.0_f32) }
    };

    // box-sizing default CSS = ContentBox; los resets modernos lo
    // fuerzan a BorderBox. Taffy 0.9 default es BorderBox así que
    // mapeamos explícito en ambos sentidos.
    let box_sizing = match b.box_sizing {
        CssBoxSizing::ContentBox => BoxSizing::ContentBox,
        CssBoxSizing::BorderBox => BoxSizing::BorderBox,
    };
    // vertical-align mapea a align_self (con prioridad sobre el de
    // align-self CSS) cuando es inline/inline-block — no es lo mismo en
    // CSS spec pero alcanza para el subset que nos importa.
    let mut align_self = match b.vertical_align {
        VerticalAlign::Baseline => map_align_self(b.align_self),
        VerticalAlign::Top => Some(AlignSelf::Start),
        VerticalAlign::Middle => Some(AlignSelf::Center),
        VerticalAlign::Bottom | VerticalAlign::Sub => Some(AlignSelf::End),
        VerticalAlign::Super => Some(AlignSelf::Start),
    };
    // Fase 7.849 — `width` intrínseca (min/max/fit-content): el width ya cayó
    // a `auto`, pero el padre (flex column/row) estira al hijo en el eje
    // cruzado por su `align-items: stretch` default y la caja volvería a
    // ocupar el ancho completo. Forzamos `align_self: Start` para que se
    // encoja al contenido — salvo que el autor pidiera un alineamiento
    // explícito (align-self / vertical-align), que respetamos. `align_self`
    // gobierna cómo se ubica ESTE nodo en SU padre, así que aplica por igual
    // sea el nodo bloque o contenedor flex.
    if is_intrinsic_size(b.width)
        && matches!(b.align_self, CssAlignSelf::Auto)
        && matches!(b.vertical_align, VerticalAlign::Baseline)
    {
        align_self = Some(AlignSelf::Start);
    }
    let flex_basis: Dimension = length_to_taffy(b.flex_basis, zoom).unwrap_or_else(auto);

    // Position + insets (top/right/bottom/left).
    let position_kind = match b.position {
        CssPosition::Static => TaffyPosition::Relative, // = layout normal
        CssPosition::Relative | CssPosition::Sticky => TaffyPosition::Relative,
        CssPosition::Absolute | CssPosition::Fixed => TaffyPosition::Absolute,
    };
    let inset = Rect {
        top: length_to_inset(b.inset_top, zoom),
        right: length_to_inset(b.inset_right, zoom),
        bottom: length_to_inset(b.inset_bottom, zoom),
        left: length_to_inset(b.inset_left, zoom),
    };

    // Taffy Display: Block/Flex/Grid/None. Inline/InlineBlock las
    // tratamos como Flex (row) por las hacks de inlines.
    let taffy_display = match b.display {
        Display::None => TaffyDisplay::None,
        Display::Grid | Display::InlineGrid => TaffyDisplay::Grid,
        _ => TaffyDisplay::Flex,
    };

    // Grid templates — sólo se aplican si display es grid. Las pistas Px
    // se escalan con zoom; fr/auto/pct quedan intactas.
    let grid_template_columns: Vec<GridTemplateComponent<String>> =
        if is_grid { b.grid_template_columns.iter().map(|t| map_grid_track(t, zoom)).collect() } else { Vec::new() };
    let grid_template_rows: Vec<GridTemplateComponent<String>> =
        if is_grid { b.grid_template_rows.iter().map(|t| map_grid_track(t, zoom)).collect() } else { Vec::new() };

    // Pistas implícitas (auto-rows/columns) y dirección de auto-colocación —
    // sólo relevantes para un contenedor grid.
    let grid_auto_rows: Vec<TrackSizingFunction> =
        if is_grid { b.grid_auto_rows.iter().map(|t| map_grid_track_sizing(t, zoom)).collect() } else { Vec::new() };
    let grid_auto_columns: Vec<TrackSizingFunction> =
        if is_grid { b.grid_auto_columns.iter().map(|t| map_grid_track_sizing(t, zoom)).collect() } else { Vec::new() };
    let grid_auto_flow = map_grid_auto_flow(b.grid_auto_flow);
    let grid_template_areas: Vec<GridTemplateArea<String>> = match (&b.grid_template_areas, is_grid) {
        (Some(s), true) => parse_grid_template_areas(s),
        _ => Vec::new(),
    };

    // Colocación del ítem en su grilla padre (`grid-row`/`grid-column`). Vale
    // para cualquier nodo (es el padre quien decide si es grid item); taffy lo
    // ignora si el padre no es grid. `auto`/None → sin colocación explícita.
    let grid_row = TaffyLine {
        start: map_grid_line(&b.grid_row_start),
        end: map_grid_line(&b.grid_row_end),
    };
    let grid_column = TaffyLine {
        start: map_grid_line(&b.grid_column_start),
        end: map_grid_line(&b.grid_column_end),
    };

    Style {
        display: taffy_display,
        flex_direction,
        flex_wrap,
        justify_content,
        align_items,
        align_content,
        // justify-items / justify-self: taffy sólo los usa en grid (los
        // ignora en flex). `None`/`Auto` → default de taffy.
        justify_items: b.justify_items.map(map_align),
        justify_self: map_align_self(b.justify_self),
        align_self,
        flex_grow: b.flex_grow,
        flex_shrink: b.flex_shrink,
        flex_basis,
        box_sizing,
        position: position_kind,
        inset,
        gap,
        size: Size { width, height },
        min_size,
        max_size,
        // CSS aspect-ratio: taffy dimensiona el eje `auto` a partir del otro
        // usando esta relación. `None` = sin relación.
        aspect_ratio: b.aspect_ratio,
        margin: Rect {
            left: margin_left_lpa(b, zoom),
            right: margin_right_lpa(b, zoom, 0.0),
            top: margin_top_lpa(b, zoom),
            bottom: margin_bottom_lpa(b, zoom),
        },
        // border: el ancho del borde reserva espacio en el box model (igual
        // que padding). Sin esto el contenido quedaba a `padding` del borde
        // externo mientras `decorations.rs` insetea fondo/contenido por
        // `border+padding` → el borde se pintaba ENCIMA del contenido. Con
        // `box-sizing: border-box` taffy además descuenta el borde del size.
        border: Rect {
            left: length(b.border_widths.left * zoom),
            right: length(b.border_widths.right * zoom),
            top: length(b.border_widths.top * zoom),
            bottom: length(b.border_widths.bottom * zoom),
        },
        padding: Rect {
            left: length(b.padding.left * zoom),
            right: length(b.padding.right * zoom),
            top: length(b.padding.top * zoom),
            bottom: length(b.padding.bottom * zoom),
        },
        grid_template_columns: grid_template_columns.into(),
        grid_template_rows: grid_template_rows.into(),
        grid_auto_rows: grid_auto_rows.into(),
        grid_auto_columns: grid_auto_columns.into(),
        grid_auto_flow,
        grid_template_areas: grid_template_areas.into(),
        grid_row,
        grid_column,
        ..Default::default()
    }
}

pub(crate) fn map_grid_track(t: &GridTrackSize, zoom: f32) -> GridTemplateComponent<String> {
    GridTemplateComponent::Single(map_grid_track_sizing(t, zoom))
}

/// Una pista de grid como `TrackSizingFunction` (sin envolver en
/// `GridTemplateComponent`). Lo usan las pistas implícitas
/// `grid-auto-{rows,columns}`, que taffy modela como `Vec<TrackSizingFunction>`
/// y no admiten `repeat()`/named lines.
pub(crate) fn map_grid_track_sizing(t: &GridTrackSize, zoom: f32) -> TrackSizingFunction {
    match t {
        GridTrackSize::Auto => auto(),
        GridTrackSize::Px(v) => length(*v * zoom),
        GridTrackSize::Pct(v) => percent(*v / 100.0),
        GridTrackSize::Fr(v) => fr(*v),
        GridTrackSize::MinContent => min_content(),
        GridTrackSize::MaxContent => max_content(),
        // `fit-content(<len>)`: helper genérico de taffy con el length como tope.
        GridTrackSize::FitContent(px) => {
            let lp: TaffyLengthPercentage = length(*px * zoom);
            fit_content(lp)
        }
        GridTrackSize::Minmax(min, max) => minmax(breadth_to_min(min, zoom), breadth_to_max(max, zoom)),
    }
}

/// `<track-breadth>` → componente `min` de un track de taffy. `Fr` no es válido
/// como mínimo en CSS → degrada a `auto`. Los helpers genéricos infieren el
/// tipo `MinTrackSizingFunction` por el retorno.
fn breadth_to_min(b: &GridTrackBreadth, zoom: f32) -> TaffyMinTrack {
    match b {
        GridTrackBreadth::Auto | GridTrackBreadth::Fr(_) => auto(),
        GridTrackBreadth::Px(v) => length(*v * zoom),
        GridTrackBreadth::Pct(v) => percent(*v / 100.0),
        GridTrackBreadth::MinContent => min_content(),
        GridTrackBreadth::MaxContent => max_content(),
    }
}

/// `<track-breadth>` → componente `max` de un track de taffy (admite `fr`).
fn breadth_to_max(b: &GridTrackBreadth, zoom: f32) -> TaffyMaxTrack {
    match b {
        GridTrackBreadth::Auto => auto(),
        GridTrackBreadth::Px(v) => length(*v * zoom),
        GridTrackBreadth::Pct(v) => percent(*v / 100.0),
        GridTrackBreadth::Fr(v) => fr(*v),
        GridTrackBreadth::MinContent => min_content(),
        GridTrackBreadth::MaxContent => max_content(),
    }
}

/// `grid-auto-flow` CSS → taffy. Las variantes calzan 1:1.
pub(crate) fn map_grid_auto_flow(f: GridAutoFlow) -> TaffyGridAutoFlow {
    match f {
        GridAutoFlow::Row => TaffyGridAutoFlow::Row,
        GridAutoFlow::Column => TaffyGridAutoFlow::Column,
        GridAutoFlow::RowDense => TaffyGridAutoFlow::RowDense,
        GridAutoFlow::ColumnDense => TaffyGridAutoFlow::ColumnDense,
    }
}

/// Resuelve un `<grid-line>` opaco (`"2"`, `"-1"`, `"span 3"`, `auto`/None,
/// nombre de línea o de área) a un `GridPlacement` de taffy. Un nombre suelto
/// se emite como `NamedLine(name, 1)`: taffy resuelve el sufijo implícito
/// `-start`/`-end` según el lado del `Line` en que cae (ver
/// `NamedLineResolver` de taffy), así que `grid-area: header` aterriza en el
/// área nombrada del template.
pub(crate) fn map_grid_line(s: &Option<String>) -> GridPlacement<String> {
    let raw = match s {
        Some(v) => v.trim(),
        None => return GridPlacement::Auto,
    };
    if raw.is_empty() || raw.eq_ignore_ascii_case("auto") {
        return GridPlacement::Auto;
    }
    // `span <n>` o `span <name>` → span de n pistas (nombre → 1).
    if let Some(rest) = raw.strip_prefix("span").filter(|r| {
        r.is_empty() || r.starts_with(char::is_whitespace)
    }) {
        let n = rest.trim().parse::<u16>().unwrap_or(1).max(1);
        return GridPlacement::Span(n);
    }
    // Índice de línea entero (puede ser negativo: cuenta desde el final).
    if let Ok(idx) = raw.parse::<i16>() {
        if idx != 0 {
            return GridPlacement::Line(idx.into());
        }
        return GridPlacement::Auto;
    }
    // Nombre de línea/área. `<custom-ident> <integer>` (p.ej. `col 2`) toma
    // el ident; un nombre solo cuenta como la 1ª línea de ese nombre.
    let mut it = raw.split_whitespace();
    let name = it.next().unwrap_or("").to_string();
    let nth = it.next().and_then(|t| t.parse::<i16>().ok()).unwrap_or(1);
    if name.is_empty() {
        return GridPlacement::Auto;
    }
    GridPlacement::NamedLine(name, nth)
}

/// Parsea `grid-template-areas` (`"a a" "b c"`) a la lista de áreas nombradas
/// de taffy con coordenadas de línea 1-based. Cada nombre toma el rectángulo
/// que lo encierra; `.` (celda nula) se ignora. Filas con distinto número de
/// columnas o áreas no-rectangulares se toleran (bounding box).
pub(crate) fn parse_grid_template_areas(s: &str) -> Vec<GridTemplateArea<String>> {
    // Extrae cada fila entre comillas; tolera comillas simples o dobles.
    let mut rows: Vec<Vec<String>> = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let q = bytes[i];
        if q == b'"' || q == b'\'' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != q {
                j += 1;
            }
            let row = &s[start..j.min(s.len())];
            rows.push(row.split_whitespace().map(|t| t.to_string()).collect());
            i = j + 1;
        } else {
            i += 1;
        }
    }
    // name → (min_row, max_row, min_col, max_col), 0-based.
    use std::collections::HashMap;
    let mut bounds: HashMap<String, (usize, usize, usize, usize)> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if cell == "." {
                continue;
            }
            bounds
                .entry(cell.clone())
                .and_modify(|b| {
                    b.0 = b.0.min(r);
                    b.1 = b.1.max(r);
                    b.2 = b.2.min(c);
                    b.3 = b.3.max(c);
                })
                .or_insert_with(|| {
                    order.push(cell.clone());
                    (r, r, c, c)
                });
        }
    }
    order
        .into_iter()
        .map(|name| {
            let (r0, r1, c0, c1) = bounds[&name];
            GridTemplateArea {
                name,
                // 1-based; el final es la línea DESPUÉS de la última celda.
                row_start: (r0 + 1) as u16,
                row_end: (r1 + 2) as u16,
                column_start: (c0 + 1) as u16,
                column_end: (c1 + 2) as u16,
            }
        })
        .collect()
}

/// `length-percentage-auto`: para insets (top/right/bottom/left) que
/// aceptan `auto` además de px/%. `zoom` escala sólo el valor Px;
/// los porcentajes se resuelven contra el contenedor (que también escala).
/// Margen izquierdo del box como `LengthPercentageAuto`, respetando
/// `margin-left: auto` (centrado horizontal → taffy `auto()`).
pub(crate) fn margin_left_lpa(b: &BoxNode, zoom: f32) -> LengthPercentageAuto {
    if b.margin_left_auto {
        auto()
    } else {
        length(b.margin.left * zoom)
    }
}

/// Margen derecho del box; `extra` suma px fijos (algunos sitios lo
/// necesitan) sólo cuando el lado NO es `auto`.
pub(crate) fn margin_right_lpa(b: &BoxNode, zoom: f32, extra: f32) -> LengthPercentageAuto {
    if b.margin_right_auto {
        auto()
    } else {
        length(b.margin.right * zoom + extra)
    }
}

/// Margen superior/inferior; `auto()` cuando el flag (ya resuelto contra el
/// padre flex/grid en el build) está puesto → taffy absorbe el espacio libre
/// para centrar/empujar en el eje de bloque.
pub(crate) fn margin_top_lpa(b: &BoxNode, zoom: f32) -> LengthPercentageAuto {
    if b.margin_top_auto {
        auto()
    } else {
        length(b.margin.top * zoom)
    }
}

pub(crate) fn margin_bottom_lpa(b: &BoxNode, zoom: f32) -> LengthPercentageAuto {
    if b.margin_bottom_auto {
        auto()
    } else {
        length(b.margin.bottom * zoom)
    }
}

pub(crate) fn length_to_inset(v: LengthVal, zoom: f32) -> LengthPercentageAuto {
    match v {
        LengthVal::Px(px) => length(px * zoom),
        LengthVal::Pct(pct) => percent(pct / 100.0),
        // Keywords intrínsecos no son válidos en `inset`: tratamos como auto.
        _ => auto(),
    }
}

pub(crate) fn map_align_self(a: CssAlignSelf) -> Option<AlignSelf> {
    match a {
        CssAlignSelf::Auto => None,
        CssAlignSelf::Start => Some(AlignSelf::Start),
        CssAlignSelf::Center => Some(AlignSelf::Center),
        CssAlignSelf::End => Some(AlignSelf::End),
        CssAlignSelf::Stretch => Some(AlignSelf::Stretch),
        CssAlignSelf::Baseline => Some(AlignSelf::Baseline),
    }
}

/// Arma un `peniko::Gradient` desde un gradiente CSS contra el rect, según
/// `g.geometry`: lineal (segmento en la dirección CSS), radial (círculo
/// centrado en `at <pos>` con radio por `<size>`) o cónico (sweep `from
/// <angle> at <pos>`). Aplica `alpha_mul` (opacity) a cada stop. Devuelve
/// None si los stops no se pueden representar.
pub(crate) fn build_linear_gradient_brush(
    g: &LinearGradient,
    rect: llimphi_ui::PaintRect,
    alpha_mul: f32,
) -> Option<Gradient> {
    use llimphi_raster::peniko::Extend;
    use puriy_engine::style::{GradientGeometry, LengthVal, RadialSize};
    if g.stops.len() < 2 {
        return None;
    }
    let w = rect.w as f64;
    let h = rect.h as f64;
    // Resuelve una posición CSS (`Pct` contra el span, `Px` crudo) a px de
    // pantalla. Compartido por el centro de radial y conic.
    let resolve_pos = |v: LengthVal, span: f64, origin: f64| -> f64 {
        match v {
            LengthVal::Pct(p) => origin + span * (p as f64) / 100.0,
            LengthVal::Px(px) => origin + px as f64,
            // Auto y keywords intrínsecos (inválidos acá) → centro.
            _ => origin + span * 0.5,
        }
    };

    // Geometría base: dirección/longitud del eje del gradiente, contra el cual
    // se resuelven las posiciones de los stops a fracción 0..1.
    enum Base {
        Linear { start: Point, dir: (f64, f64), len: f64 },
        Radial { center: Point, radius: f64 },
        Conic { center: Point, base_start: f64 },
    }
    let base = match &g.geometry {
        GradientGeometry::Radial(spec) => {
            let cxp = resolve_pos(spec.cx, w, rect.x as f64);
            let cyp = resolve_pos(spec.cy, h, rect.y as f64);
            let (dl, dr) = ((cxp - rect.x as f64).abs(), (rect.x as f64 + w - cxp).abs());
            let (dt, db) = ((cyp - rect.y as f64).abs(), (rect.y as f64 + h - cyp).abs());
            let corner = |x: f64, y: f64| ((cxp - x).powi(2) + (cyp - y).powi(2)).sqrt();
            let corners = [
                corner(rect.x as f64, rect.y as f64),
                corner(rect.x as f64 + w, rect.y as f64),
                corner(rect.x as f64, rect.y as f64 + h),
                corner(rect.x as f64 + w, rect.y as f64 + h),
            ];
            let fmin = |a: f64, b: f64| a.min(b);
            let fmax = |a: f64, b: f64| a.max(b);
            let radius = match spec.size {
                RadialSize::ClosestSide => dl.min(dr).min(dt).min(db),
                RadialSize::FarthestSide => dl.max(dr).max(dt).max(db),
                RadialSize::ClosestCorner => corners.iter().copied().fold(f64::MAX, fmin),
                RadialSize::FarthestCorner => corners.iter().copied().fold(0.0, fmax),
            }
            .max(1.0);
            Base::Radial { center: Point::new(cxp, cyp), radius }
        }
        GradientGeometry::Conic { from_deg, cx, cy } => {
            let center = Point::new(
                resolve_pos(*cx, w, rect.x as f64),
                resolve_pos(*cy, h, rect.y as f64),
            );
            // peniko Sweep: 0 rad = +x (derecha). CSS `from`: 0 = up → -90°.
            Base::Conic { center, base_start: (*from_deg - 90.0).to_radians() as f64 }
        }
        GradientGeometry::Linear { angle_deg } => {
            let theta = (*angle_deg as f64).to_radians();
            let dx = theta.sin();
            let dy = -theta.cos();
            let cx = rect.x as f64 + w * 0.5;
            let cy = rect.y as f64 + h * 0.5;
            let len = dx.abs() * w + dy.abs() * h;
            let start = Point::new(cx - dx * len * 0.5, cy - dy * len * 0.5);
            Base::Linear { start, dir: (dx, dy), len }
        }
    };

    // Longitud del eje contra la cual un stop en px se vuelve fracción.
    // Para cónico el eje es angular: 360° (un giro), así que un stop en px se
    // interpreta como grados.
    let axis_len = match &base {
        Base::Linear { len, .. } => *len,
        Base::Radial { radius, .. } => *radius,
        Base::Conic { .. } => 360.0,
    };
    let frac = |v: LengthVal| -> f64 {
        match v {
            LengthVal::Pct(p) => (p as f64) / 100.0,
            LengthVal::Px(px) => {
                if axis_len > 0.0 { (px as f64) / axis_len } else { 0.0 }
            }
            // Auto y keywords intrínsecos (inválidos en un stop) → 0.
            _ => 0.0,
        }
    };

    // Posiciones de los stops como fracción del eje, aplicando la
    // interpolación CSS: primero/último sin posición → 0/1, runs intermedios
    // sin posición se reparten linealmente, y la secuencia se fuerza no
    // decreciente (hard stops como `red 50%, blue 50%`).
    let n = g.stops.len();
    let mut fr: Vec<Option<f64>> = g.stops.iter().map(|s| s.pos.map(frac)).collect();
    if fr[0].is_none() {
        fr[0] = Some(0.0);
    }
    if fr[n - 1].is_none() {
        fr[n - 1] = Some(1.0);
    }
    let mut last_def = 0usize;
    for j in 1..n {
        if fr[j].is_some() {
            let gap = j - last_def;
            if gap > 1 {
                let a = fr[last_def].unwrap();
                let b = fr[j].unwrap();
                for k in 1..gap {
                    fr[last_def + k] = Some(a + (b - a) * (k as f64 / gap as f64));
                }
            }
            last_def = j;
        }
    }
    let mut pos: Vec<f64> = fr.iter().map(|x| x.unwrap()).collect();
    let mut run = pos[0];
    for v in pos.iter_mut() {
        if *v < run {
            *v = run;
        } else {
            run = *v;
        }
    }

    // Periodo del patrón. `repeating-*` tilea [first..last]; si el patrón ya
    // cubre todo el eje (o es degenerado) cae a no-repetido.
    let period = pos[n - 1] - pos[0];
    let repeating = g.repeating && period > 1e-4 && period < 1.0 - 1e-4;

    // Remapea cada posición a [0,1] de una unidad del patrón (si repite) o
    // simplemente clampa a [0,1] (si no).
    let (offset, span) = if repeating { (pos[0], period) } else { (0.0, 1.0) };
    let mut peniko_stops: Vec<ColorStop> = Vec::with_capacity(n);
    for (i, s) in g.stops.iter().enumerate() {
        let p = if repeating {
            ((pos[i] - offset) / span) as f32
        } else {
            pos[i].clamp(0.0, 1.0) as f32
        };
        let a = ((s.color.a as f32) * alpha_mul) as u8;
        let c = Color::from_rgba8(s.color.r, s.color.g, s.color.b, a);
        peniko_stops.push(ColorStop::from((p, c)));
    }

    // La geometría final abarca exactamente una unidad del patrón cuando
    // repite (peniko `Extend::Repeat` tilea ese [0,1] a lo largo del eje).
    let kind = match base {
        Base::Linear { start, dir, len } => {
            let (dx, dy) = dir;
            let s = Point::new(start.x + dx * len * offset, start.y + dy * len * offset);
            let e = Point::new(
                start.x + dx * len * (offset + span),
                start.y + dy * len * (offset + span),
            );
            GradientKind::Linear(llimphi_raster::peniko::LinearGradientPosition { start: s, end: e })
        }
        Base::Radial { center, radius } => GradientKind::Radial(llimphi_raster::peniko::RadialGradientPosition {
            start_center: center,
            start_radius: (radius * offset) as f32,
            end_center: center,
            end_radius: (radius * (offset + span)) as f32,
        }),
        Base::Conic { center, base_start } => {
            let s = base_start + offset * std::f64::consts::TAU;
            GradientKind::Sweep(llimphi_raster::peniko::SweepGradientPosition {
                center,
                start_angle: s as f32,
                end_angle: (s + span * std::f64::consts::TAU) as f32,
            })
        }
    };

    Some(Gradient {
        kind,
        extend: if repeating { Extend::Repeat } else { Extend::Pad },
        stops: ColorStops(peniko_stops.into()),
        ..Default::default()
    })
}

pub(crate) fn map_flex_direction(d: CssFlexDirection) -> FlexDirection {
    match d {
        CssFlexDirection::Row => FlexDirection::Row,
        CssFlexDirection::RowReverse => FlexDirection::RowReverse,
        CssFlexDirection::Column => FlexDirection::Column,
        CssFlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
    }
}

pub(crate) fn map_flex_wrap(w: CssFlexWrap) -> FlexWrap {
    match w {
        CssFlexWrap::NoWrap => FlexWrap::NoWrap,
        CssFlexWrap::Wrap => FlexWrap::Wrap,
        CssFlexWrap::WrapReverse => FlexWrap::WrapReverse,
    }
}

pub(crate) fn map_justify(j: CssJustifyContent) -> JustifyContent {
    match j {
        CssJustifyContent::Start => JustifyContent::Start,
        CssJustifyContent::Center => JustifyContent::Center,
        CssJustifyContent::End => JustifyContent::End,
        CssJustifyContent::SpaceBetween => JustifyContent::SpaceBetween,
        CssJustifyContent::SpaceAround => JustifyContent::SpaceAround,
        CssJustifyContent::SpaceEvenly => JustifyContent::SpaceEvenly,
    }
}

pub(crate) fn map_align(a: CssAlignItems) -> AlignItems {
    match a {
        CssAlignItems::Start => AlignItems::Start,
        CssAlignItems::Center => AlignItems::Center,
        CssAlignItems::End => AlignItems::End,
        CssAlignItems::Stretch => AlignItems::Stretch,
        CssAlignItems::Baseline => AlignItems::Baseline,
    }
}

/// `align-content` CSS → taffy. `Normal` ⇒ `None` (taffy aplica su default,
/// ≈ stretch para flex). `Start`/`End` mapean a `FlexStart`/`FlexEnd` para
/// que respeten la dirección flex (row-reverse, etc.).
pub(crate) fn map_align_content(a: CssAlignContent) -> Option<AlignContent> {
    match a {
        CssAlignContent::Normal => None,
        CssAlignContent::Start => Some(AlignContent::Start),
        CssAlignContent::Center => Some(AlignContent::Center),
        CssAlignContent::End => Some(AlignContent::End),
        CssAlignContent::Stretch => Some(AlignContent::Stretch),
        CssAlignContent::SpaceBetween => Some(AlignContent::SpaceBetween),
        CssAlignContent::SpaceAround => Some(AlignContent::SpaceAround),
        CssAlignContent::SpaceEvenly => Some(AlignContent::SpaceEvenly),
    }
}

/// Traduce un `LengthVal` CSS al tipo de longitud que taffy entiende.
/// `Auto` queda como `None` (caller lo reemplaza con el default según
/// display o `auto()` para max-size).
pub(crate) fn length_to_taffy(v: LengthVal, zoom: f32) -> Option<llimphi_layout::taffy::style::Dimension> {
    match v {
        LengthVal::Auto => None,
        LengthVal::Px(px) => Some(length(px * zoom)),
        LengthVal::Pct(pct) => Some(percent(pct / 100.0)),
        // Fase 7.849 — tamaño intrínseco. taffy 0.9 no lo modela en
        // `Dimension`; lo mapeamos a `auto` (= dimensiona por contenido).
        // Devolvemos `Some` —no `None`— a propósito: así SOBREESCRIBE el
        // default de display (un bloque vale `percent(1.0)` = ancho lleno; con
        // `auto` se encoge al contenido). La supresión del stretch del padre
        // la hace `box_style` vía `align_self: Start`.
        LengthVal::MinContent | LengthVal::MaxContent | LengthVal::FitContent => Some(auto()),
    }
}

/// `true` si el `LengthVal` es un keyword de tamaño intrínseco (Fase 7.849).
pub(crate) fn is_intrinsic_size(v: LengthVal) -> bool {
    matches!(
        v,
        LengthVal::MinContent | LengthVal::MaxContent | LengthVal::FitContent
    )
}

/// `true` si todos los hijos directos son inline o inline-block. Si los
/// hijos son block, el bloque sigue siendo column.
pub(crate) fn has_inline_children(b: &BoxNode) -> bool {
    !b.children.is_empty()
        && b.children
            .iter()
            .all(|c| matches!(c.display, Display::Inline | Display::InlineBlock))
}
