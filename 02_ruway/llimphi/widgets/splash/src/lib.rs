//! `llimphi-widget-splash` — splash de arranque gioser.
//!
//! Identidad visual del SO al boot: cuatro cuadrantes ordenados como
//! una cruz andina, cada uno con su nombre quechua y color simbólico,
//! que **entran en secuencia** con un tween de fade+escala.
//!
//! Los cuadrantes (en orden de entrada):
//! 1. `unanchay` — PERCIBIR  — cyan (índigo claro)
//! 2. `yachay`   — CONOCER   — verde aurora
//! 3. `ruway`    — HACER     — naranja sunset
//! 4. `ukupacha` — RAÍZ      — púrpura profundo
//!
//! Cada cuadrante hace fade-in + slight scale-up, con un offset de
//! `motion::NORMAL / 2` entre uno y el siguiente. La app pasa un
//! `Instant` de inicio y el splash calcula las fases relativas — no
//! requiere ningún tween del modelo.
//!
//! Cuando el splash termina (todos visibles), la app puede:
//! - mantenerlo unos segundos más como pantalla de carga,
//! - hacer un fade-out completo cuando el sistema esté listo,
//! - o reemplazarlo por la UI principal.

#![forbid(unsafe_code)]

use std::time::Instant;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_motion::motion;

/// Datos de un cuadrante: nombre quechua, glosa breve y color.
#[derive(Debug, Clone, Copy)]
pub struct Quadrant {
    pub name: &'static str,
    pub gloss: &'static str,
    pub color: Color,
}

/// Los cuatro cuadrantes canónicos, en orden de entrada al splash.
pub fn quadrants() -> [Quadrant; 4] {
    [
        Quadrant {
            name: "unanchay",
            gloss: "PERCIBIR",
            color: Color::from_rgba8(110, 160, 230, 255),
        },
        Quadrant {
            name: "yachay",
            gloss: "CONOCER",
            color: Color::from_rgba8(110, 220, 180, 255),
        },
        Quadrant {
            name: "ruway",
            gloss: "HACER",
            color: Color::from_rgba8(232, 160, 90, 255),
        },
        Quadrant {
            name: "ukupacha",
            gloss: "RAÍZ",
            color: Color::from_rgba8(160, 110, 220, 255),
        },
    ]
}

/// Construye el splash. `started_at` es el `Instant` de origen — el
/// splash calcula las fases relativas. La app puede llamar `animate(handle,
/// motion::SLOW * 3, …)` para forzar repaints durante la animación.
///
/// `bg`: color de fondo (típico: `theme.bg_app`).
/// `fg_text`: color del título/glosa.
pub fn splash_view<Msg: Clone + 'static>(
    started_at: Instant,
    bg: Color,
    fg_text: Color,
) -> View<Msg> {
    let elapsed = started_at.elapsed().as_secs_f32();
    let stagger = motion::NORMAL.as_secs_f32() * 0.45;
    let per_quad = motion::NORMAL.as_secs_f32();
    let quads = quadrants();

    let cells: Vec<View<Msg>> = quads
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let local_t = ((elapsed - i as f32 * stagger) / per_quad).clamp(0.0, 1.0);
            let eased = motion::ease_out_cubic(local_t);
            quadrant_cell(q, eased, fg_text)
        })
        .collect();

    // 2×2 grid: row 0 = unanchay + yachay; row 1 = ruway + ukupacha.
    let row = |a: View<Msg>, b: View<Msg>| -> View<Msg> {
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: percent(0.5_f32),
            },
            gap: Size {
                width: length(12.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![a, b])
    };
    let mut iter = cells.into_iter();
    let r0 = row(iter.next().unwrap(), iter.next().unwrap());
    let r1 = row(iter.next().unwrap(), iter.next().unwrap());

    let grid = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(420.0_f32),
            height: length(280.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(12.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![r0, r1]);

    // Título "gioser" debajo, también fade-in pero al final.
    let title_t = ((elapsed - 4.0 * stagger) / per_quad).clamp(0.0, 1.0);
    let title_alpha = motion::ease_out_cubic(title_t);
    let title = View::new(Style {
        size: Size {
            width: length(420.0_f32),
            height: length(32.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned("gioser", 22.0, fg_text, Alignment::Center)
    .alpha(title_alpha);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(0.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .children(vec![grid, title])
}

fn quadrant_cell<Msg: Clone + 'static>(
    quad: &Quadrant,
    progress: f32,
    fg_text: Color,
) -> View<Msg> {
    // El cuadrante "entra" con fade y un leve drift desde abajo (10px).
    // El drift lo representamos con un padding-top que tiende a cero;
    // como llimphi no expone translate por nodo (sólo position absolute),
    // metemos el contenido en un wrapper con padding decreciente.
    let drift = (1.0 - progress) * 10.0;

    let name = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(quad.name, 16.0, fg_text, Alignment::Center);

    let gloss = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(quad.gloss, 10.0, quad.color, Alignment::Center);

    let inner = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(drift),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![name, gloss]);

    // Fondo del cuadrante con gradient vertical en el color semántico:
    // alpha 50 arriba → alpha 12 abajo. Da volumen al cuadrante (más
    // intenso cerca del accent strip del top) y un efecto "halo descendente"
    // que ayuda a leer la cruz andina como cuatro luces que emergen del
    // centro. Antes: alpha 30 uniforme.
    let border = with_alpha8(quad.color, 90);
    let bg_top = with_alpha8(quad.color, 50);
    let bg_bot = with_alpha8(quad.color, 12);

    let cell_radius = llimphi_theme::radius::MD;

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, RoundedRect};
        use llimphi_ui::llimphi_raster::peniko::{Fill, Gradient};

        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let x0 = rect.x as f64;
        let y0 = rect.y as f64;
        let x1 = (rect.x + rect.w) as f64;
        let y1 = (rect.y + rect.h) as f64;
        let rr = RoundedRect::new(x0, y0, x1, y1, cell_radius);
        let gradient = Gradient::new_linear(Point::new(x0, y0), Point::new(x0, y1))
            .with_stops([bg_top, bg_bot].as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &gradient, None, &rr);
    })
    .radius(cell_radius)
    .clip(true)
    .alpha(progress)
    .children(vec![
        // Línea accent superior — 2px del color del cuadrante a alta
        // intensidad, ancla del gradiente que cae.
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(2.0_f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .fill(border),
        inner,
    ])
}

fn with_alpha8(c: Color, a: u8) -> Color {
    let [r, g, b, _] = c.components;
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    AlphaColor::new([r, g, b, a as f32 / 255.0])
}
