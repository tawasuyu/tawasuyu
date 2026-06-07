//! La **vista espacial** (el "Prezi" de mirada): un mosaico por escritorio en
//! una grilla, con sus ventanas a escala, y una cámara que vuela del escritorio
//! activo a la grilla (al abrir) o aterriza en el destino elegido (al saltar).
//!
//! Vive en la librería —separada del binario— para que el render sea
//! reutilizable y, sobre todo, **verificable headless** (`examples/dump_overview`
//! lo pinta a PNG sin levantar el compositor). Es agnóstico del `Msg` de la app:
//! el llamante pasa un `on_pick: Fn(usize) -> Msg` para el click en una celda.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, JustifyContent, Position, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

use mirada_brain::Desktop;

/// Estado de cámara de la vista espacial, leído por el render cada frame.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// Progreso `0..1`. `1` = grilla completa visible; `0` = la celda
    /// [`focus`](Self::focus) llena la pantalla. Interpolar `zoom` da el vuelo.
    pub zoom: f32,
    /// La celda sobre la que se centra la cámara (origen del zoom).
    pub focus: usize,
}

/// Pinta la vista espacial sobre `desktop`, con la cámara `cam`. `screen` es el
/// tamaño del lienzo en px (relación de aspecto del mosaico = la de la salida).
/// `on_pick(i)` produce el `Msg` que dispara el salto al escritorio `i`.
///
/// La geometría base se calcula con la grilla a pantalla completa (zoom `t=1`);
/// la cámara es la transformación afín `scr(p) = O(t) + s(t)·p`, identidad a
/// `t=1` y, a `t=0`, agranda la celda con foco hasta llenar la pantalla.
pub fn overview_view<M, F>(
    desktop: &Desktop,
    theme: &Theme,
    on_accent: Color,
    win_bg: Color,
    canvas_bg: Color,
    cam: Camera,
    screen: (i32, i32),
    on_pick: F,
) -> View<M>
where
    M: Clone + 'static,
    F: Fn(usize) -> M,
{
    let cfg = desktop.config();
    let loads = desktop.workspace_loads();
    let count = loads.len().max(1);
    let active = desktop.active_index();
    let focus = cam.focus.min(count - 1);

    // Rect de referencia (relación de aspecto) y la geometría teselada de cada
    // escritorio, normalizada luego a [0,1] de ese rect.
    let (sw_i, sh_i) = screen;
    let wr = desktop.overview_rect(mirada_brain::Rect::new(0, 0, sw_i, sh_i));
    let wr_w = wr.w.max(1) as f32;
    let wr_h = wr.h.max(1) as f32;
    let aspect = wr_w / wr_h;
    let layouts = desktop.workspace_layouts(wr);

    // --- Grilla base (a zoom = 1, centrada en el lienzo) ----------------
    let cw = sw_i as f32;
    let ch = sh_i as f32;
    let cols = cfg.overview_grid_columns(count).max(1);
    let rows = count.div_ceil(cols);
    const MARGIN: f32 = 28.0;
    const GAP: f32 = 16.0;
    let avail_w = (cw - 2.0 * MARGIN - GAP * (cols as f32 - 1.0)).max(1.0);
    let avail_h = (ch - 2.0 * MARGIN - GAP * (rows as f32 - 1.0)).max(1.0);
    let cell_w_max = avail_w / cols as f32;
    let cell_h_max = avail_h / rows as f32;
    // Preserva el aspecto del escritorio: la celda entra en su hueco.
    let cell_w = cell_w_max.min(cell_h_max * aspect);
    let cell_h = cell_w / aspect;
    let grid_w = cell_w * cols as f32 + GAP * (cols as f32 - 1.0);
    let grid_h = cell_h * rows as f32 + GAP * (rows as f32 - 1.0);
    let gx = (cw - grid_w) / 2.0;
    let gy = (ch - grid_h) / 2.0;
    // Rect base de la celda `i` (a zoom = 1).
    let base_rect = |i: usize| -> (f32, f32, f32, f32) {
        let c = (i % cols) as f32;
        let r = (i / cols) as f32;
        (gx + c * (cell_w + GAP), gy + r * (cell_h + GAP), cell_w, cell_h)
    };

    // --- Cámara: scr(p) = O(t) + s(t)·p ---------------------------------
    let t = cam.zoom.clamp(0.0, 1.0);
    let (fx, fy, fw, _fh) = base_rect(focus);
    // Escala a t=0 para que la celda con foco llene el ancho del lienzo.
    let s0 = (cw / fw.max(1.0)).max(1.0);
    let s = s0 + (1.0 - s0) * t;
    let ox = -s0 * fx * (1.0 - t);
    let oy = -s0 * fy * (1.0 - t);

    let mut children: Vec<View<M>> = Vec::with_capacity(count);
    for i in 0..count {
        let (bx, by, bw, bh) = base_rect(i);
        let sx = ox + s * bx;
        let sy = oy + s * by;
        let sw = s * bw;
        let sh = s * bh;
        // Descartá celdas fuera de pantalla (durante el zoom-in se van).
        if sx + sw < 0.0 || sy + sh < 0.0 || sx > cw || sy > ch {
            continue;
        }

        let is_active = i == active;
        let border = if is_active { theme.accent } else { theme.border };

        // Ventanas del escritorio `i`, a escala dentro de la celda.
        let mut cell_children: Vec<View<M>> = Vec::new();
        for p in layouts.get(i).into_iter().flatten().filter(|p| p.visible) {
            let nx = ((p.rect.x - wr.x) as f32 / wr_w).clamp(0.0, 1.0);
            let ny = ((p.rect.y - wr.y) as f32 / wr_h).clamp(0.0, 1.0);
            let nw = (p.rect.w as f32 / wr_w).clamp(0.0, 1.0);
            let nh = (p.rect.h as f32 / wr_h).clamp(0.0, 1.0);
            let wb = if p.focused { theme.accent } else { theme.border };

            // Cuerpo de la miniatura. El título (si la config lo pide y hay
            // lugar) va en este nodo interior, que no tiene hijos —pintar texto
            // y children en el mismo View no compone.
            let mut body = View::new(Style {
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                padding: Rect {
                    left: length(4.0_f32),
                    right: length(4.0_f32),
                    top: length(3.0_f32),
                    bottom: length(3.0_f32),
                },
                ..Default::default()
            })
            .fill(win_bg)
            .radius(2.0);
            if cfg.overview_show_titles && sw * nw > 110.0 {
                if let Some(info) = desktop.window_info(p.id) {
                    body = body.text_aligned(info.title.clone(), 10.0, theme.fg_muted, Alignment::Start);
                }
            }

            let win = View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: percent(nx),
                    top: percent(ny),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size { width: percent(nw), height: percent(nh) },
                padding: Rect {
                    left: length(1.5_f32),
                    right: length(1.5_f32),
                    top: length(1.5_f32),
                    bottom: length(1.5_f32),
                },
                ..Default::default()
            })
            .fill(wb)
            .radius(3.0)
            .children(vec![body]);
            cell_children.push(win);
        }

        // Badge con el número de escritorio (arriba-izquierda de la celda).
        let badge_bg = if is_active { theme.accent } else { theme.bg_row_hover };
        let badge_fg = if is_active { on_accent } else { theme.fg_muted };
        cell_children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(6.0_f32),
                    top: length(6.0_f32),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size { width: length(20.0_f32), height: length(18.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .fill(badge_bg)
            .radius(4.0)
            .text_aligned(format!("{}", i + 1), 11.0, badge_fg, Alignment::Center),
        );

        children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(sx),
                    top: length(sy),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size { width: length(sw.max(1.0)), height: length(sh.max(1.0)) },
                padding: Rect {
                    left: length(2.0_f32),
                    right: length(2.0_f32),
                    top: length(2.0_f32),
                    bottom: length(2.0_f32),
                },
                ..Default::default()
            })
            .fill(border)
            .radius(6.0)
            .on_click(on_pick(i))
            .children(vec![View::new(Style {
                position: Position::Relative,
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .fill(canvas_bg)
            .radius(5.0)
            .children(cell_children)]),
        );
    }

    View::new(Style {
        position: Position::Relative,
        size: Size { width: length(cw), height: length(ch) },
        ..Default::default()
    })
    .fill(Color::from_rgba8(6, 8, 12, 255))
    .children(children)
}
