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
use llimphi_ui::llimphi_raster::kurbo::Affine;
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
pub fn overview_view<M, F, D>(
    desktop: &Desktop,
    theme: &Theme,
    on_accent: Color,
    win_bg: Color,
    canvas_bg: Color,
    cam: Camera,
    screen: (i32, i32),
    on_pick: F,
    // Modo editor de geometría: `Some(sel)` resalta el escritorio `sel` y hace
    // sus celdas ARRASTRABLES (drag para reacomodar el plano 2D); `None` = vista
    // normal (clic para saltar).
    edit_sel: Option<usize>,
    // En modo editor, arrastrar la celda `i`: `(i, fase, dcol, dfila)` donde
    // dcol/dfila son el delta EN CELDAS (ya convertido del px por el pitch).
    on_drag: D,
) -> View<M>
where
    M: Clone + Send + Sync + 'static,
    F: Fn(usize) -> M,
    D: Fn(usize, llimphi_ui::DragPhase, f32, f32) -> M + Clone + Send + Sync + 'static,
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
    // Plano rico del Prezi: posición libre + tamaño + **giro** por escritorio (en
    // unidades de celda). Por defecto deriva de la grilla; el editor de recorrido
    // del panel lo puede colocar/rotar a voluntad. El overview LO RESPETA.
    let places = cfg.overview_places_for(count);
    // Extensión en unidades de celda (posición + tamaño) → cuántas celdas span.
    let span_cols = places.iter().map(|p| p.x + p.w).fold(1.0_f32, f32::max).max(1.0);
    let span_rows = places.iter().map(|p| p.y + p.h).fold(1.0_f32, f32::max).max(1.0);
    let cols = span_cols.ceil().max(1.0);
    let rows = span_rows.ceil().max(1.0);
    const MARGIN: f32 = 28.0;
    const GAP: f32 = 16.0;
    let avail_w = (cw - 2.0 * MARGIN - GAP * (cols - 1.0)).max(1.0);
    let avail_h = (ch - 2.0 * MARGIN - GAP * (rows - 1.0)).max(1.0);
    // Preserva el aspecto del escritorio: una celda 1×1 entra en su hueco.
    let cell_w = (avail_w / cols).min(avail_h / rows * aspect);
    let cell_h = cell_w / aspect;
    let grid_w = cell_w * cols + GAP * (cols - 1.0);
    let grid_h = cell_h * rows + GAP * (rows - 1.0);
    let gx = (cw - grid_w) / 2.0;
    let gy = (ch - grid_h) / 2.0;
    // Rect base del escritorio `i` (a zoom = 1) desde su colocación rica: la
    // posición usa el "pitch" (celda+gap) para que la grilla por defecto quede
    // pixel-idéntica; el tamaño usa la celda (w/h en unidades de celda).
    let base_rect = |i: usize| -> (f32, f32, f32, f32) {
        let p = places.get(i).copied().unwrap_or_default();
        (
            gx + p.x * (cell_w + GAP),
            gy + p.y * (cell_h + GAP),
            p.w * cell_w,
            p.h * cell_h,
        )
    };
    // Giro propio del tile (rad), respetado vía `View::transform` (gira el tile y
    // sus miniaturas alrededor del centro de su rect).
    let rot_of = |i: usize| -> f64 { places.get(i).map(|p| p.rot as f64).unwrap_or(0.0) };

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
        // En modo editor, la celda seleccionada (la que mueven las flechas) va
        // en ámbar para distinguirla del escritorio activo.
        let border = if edit_sel == Some(i) {
            Color::from_rgba8(245, 180, 50, 255)
        } else if is_active {
            theme.accent
        } else {
            theme.border
        };

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

        // Badge con el número de escritorio. Va arriba-DERECHA: los títulos de
        // las miniaturas se alinean a la izquierda, así que la esquina izquierda
        // está siempre ocupada por el título de la primera ventana — el badge en
        // la derecha no lo pisa (era el solape "badge encima del título").
        let badge_bg = if is_active { theme.accent } else { theme.bg_row_hover };
        let badge_fg = if is_active { on_accent } else { theme.fg_muted };
        cell_children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: auto(),
                    top: length(6.0_f32),
                    right: length(6.0_f32),
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

        let cell = View::new(Style {
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
        .children(cell_children)]);

        // Giro propio del escritorio: rota el tile entero (borde + miniaturas)
        // alrededor del centro de su rect. `rotate(0)` ≡ identidad.
        let cell = cell.transform(Affine::rotate(rot_of(i)));

        // En modo editor la celda se ARRASTRA para reacomodar el plano 2D: el px
        // del drag se convierte a delta-en-celdas con el pitch en pantalla.
        let cell = if edit_sel.is_some() {
            let pitch_x = (s * (bw + GAP)).max(1.0);
            let pitch_y = (s * (bh + GAP)).max(1.0);
            let on_drag = on_drag.clone();
            cell.draggable(move |phase, dx, dy| {
                Some(on_drag(i, phase, dx / pitch_x, dy / pitch_y))
            })
        } else {
            cell
        };
        children.push(cell);
    }

    // Banner del editor de geometría.
    if edit_sel.is_some() {
        children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(0.0_f32),
                    top: length(10.0_f32),
                    right: length(0.0_f32),
                    bottom: auto(),
                },
                size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text_aligned(
                "Editor de Prezi · flechas: mover · 1-9: elegir escritorio · g: salir"
                    .to_string(),
                12.0,
                Color::from_rgba8(245, 180, 50, 255),
                Alignment::Center,
            ),
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
