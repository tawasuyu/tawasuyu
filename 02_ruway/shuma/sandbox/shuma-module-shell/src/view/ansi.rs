use super::*;

/// Convierte un `vt100::Color` a un `peniko::Color`, respetando el tema
/// del shell (los 16 índices ANSI se mapean a una paleta consistente).
pub(crate) fn vt_color(
    c: vt100::Color,
    theme: Theme,
    is_bg: bool,
) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    match c {
        vt100::Color::Default => {
            if is_bg {
                // Transparent — el panel ya tiene su propio fill.
                Color::from_rgba8(0, 0, 0, 0)
            } else {
                theme.fg_text
            }
        }
        vt100::Color::Rgb(r, g, b) => Color::from_rgba8(r, g, b, 255),
        vt100::Color::Idx(i) => ansi_idx_to_color(i),
    }
}

/// Empaca un `peniko::Color` a un u32 RGBA8 little-endian listo para el
/// `CellInstance` del pipeline GPU (Fase 4 del SDD-TERMINAL). Espeja
/// `llimphi_widget_terminal::pack_rgba` pero parte del color del runtime
/// (componentes f32 0..1).
pub(crate) fn pack_peniko(c: llimphi_ui::llimphi_raster::peniko::Color) -> u32 {
    let r = (c.components[0].clamp(0.0, 1.0) * 255.0) as u8;
    let g = (c.components[1].clamp(0.0, 1.0) * 255.0) as u8;
    let b = (c.components[2].clamp(0.0, 1.0) * 255.0) as u8;
    let a = (c.components[3].clamp(0.0, 1.0) * 255.0) as u8;
    llimphi_widget_terminal::pack_rgba(r, g, b, a)
}

/// Construye las `CellInstance`s a dibujar para un snapshot vt100 sobre el
/// rect del panel del TUI (Fase 4 del SDD-TERMINAL). Itera fila×col, mira
/// el char + colores fg/bg, rasteriza el glifo si todavía no está en el
/// atlas y arma un instance por celda. Las celdas con char vacío o sólo
/// espacio Y bg default se saltan (el fondo del panel cubre).
///
/// `render_cell_w`/`render_cell_h` son el tamaño de celda en el viewport
/// (deriva del rect / cols×rows); pueden diferir del cell size natural del
/// atlas — la diferencia se absorbe en el shader (el sampler lineal estira
/// el glifo al cell de salida).
pub(crate) fn build_cell_instances(
    snap: &TuiSnapshot,
    atlas: &mut llimphi_widget_terminal::GlyphAtlas,
    theme: Theme,
    rect: llimphi_ui::PaintRect,
) -> Vec<llimphi_widget_terminal::CellInstance> {
    use llimphi_widget_terminal::CellInstance;
    if snap.rows == 0 || snap.cols == 0 {
        return Vec::new();
    }
    let pad = 6.0_f32;
    let avail_w = (rect.w - pad * 2.0).max(0.0);
    let avail_h = (rect.h - pad * 2.0).max(0.0);
    let render_cell_w = (avail_w / snap.cols as f32).max(1.0);
    let render_cell_h = (avail_h / snap.rows as f32).max(1.0);
    let origin_x = rect.x + pad;
    let origin_y = rect.y + pad;
    let (atlas_cell_w, atlas_cell_h) = atlas.cell_size();

    let mut out: Vec<CellInstance> = Vec::with_capacity((snap.rows * snap.cols) as usize);
    for (r, row) in snap.cells.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            let bg = vt_color(cell.bg, theme, true);
            let fg = vt_color(cell.fg, theme, false);
            let ch = cell.ch.chars().next().unwrap_or(' ');
            let is_blank = ch == ' ' || ch == '\0';
            // Salta celdas vacías con fondo default — el panel ya pinta su
            // bg, no hay nada que cubrir ni que pintar.
            if is_blank && bg.components[3] <= 0.001 {
                continue;
            }
            // Pide el slot del glifo. Si el atlas está lleno, intenta
            // crecer una vez; si tampoco entra (raro), salta el char.
            let slot = match atlas.glyph_for(ch) {
                Some(s) => s,
                None => {
                    atlas.grow();
                    match atlas.glyph_for(ch) {
                        Some(s) => s,
                        None => continue,
                    }
                }
            };
            out.push(CellInstance {
                cell_x: origin_x + c as f32 * render_cell_w,
                cell_y: origin_y + r as f32 * render_cell_h,
                uv_x: slot.px as f32,
                uv_y: slot.py as f32,
                uv_w: atlas_cell_w as f32,
                uv_h: atlas_cell_h as f32,
                fg_rgba: pack_peniko(fg),
                bg_rgba: pack_peniko(bg),
            });
        }
    }
    out
}

/// Mapeo 256 → RGB usando la paleta xterm estándar. Cubre los 16
/// básicos, el cubo 6×6×6 y la rampa de grises.
pub(crate) fn ansi_idx_to_color(i: u8) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    const BASIC: [[u8; 3]; 16] = [
        [0, 0, 0],
        [205, 49, 49],
        [13, 188, 121],
        [229, 229, 16],
        [36, 114, 200],
        [188, 63, 188],
        [17, 168, 205],
        [229, 229, 229],
        [102, 102, 102],
        [241, 76, 76],
        [35, 209, 139],
        [245, 245, 67],
        [59, 142, 234],
        [214, 112, 214],
        [41, 184, 219],
        [255, 255, 255],
    ];
    if i < 16 {
        let [r, g, b] = BASIC[i as usize];
        return Color::from_rgba8(r, g, b, 255);
    }
    if i >= 232 {
        let v = 8 + (i - 232) * 10;
        return Color::from_rgba8(v, v, v, 255);
    }
    let i = i - 16;
    let r = i / 36;
    let g = (i / 6) % 6;
    let b = i % 6;
    let to_byte = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
    Color::from_rgba8(to_byte(r), to_byte(g), to_byte(b), 255)
}
