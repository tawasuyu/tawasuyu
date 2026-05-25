//! `pluma-editor-llimphi` — el backend Llimphi del editor DAG.
//!
//! Consume un [`RenderPlan`] de `pluma-render-plan` y lo vuelca a un árbol
//! `llimphi-ui::View`: los bloques de átomo y las marcas del osciloscopio
//! son nodos absolutamente posicionados (taffy `Position::Absolute`); los
//! conectores de dependencia van como triplas de rectángulos delgados que
//! dibujan el codo en S.
//!
//! Es el único crate de pluma visual que toca `llimphi-ui` — el resto de
//! la cadena (`core`, `graph`, `render-plan`) es agnóstico.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, Position, Rect, Size, Style},
    FlexDirection,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use pluma_render_plan::{AtomBlock, CoherenceTone, Edge, RenderPlan, SidepaneMark};

/// Paleta del editor — los colores que cambia el tema, separados del
/// `Color` semántico de las tonalidades (rojo conflicto, ámbar pendiente).
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg_app: Color,
    pub bg_panel: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub border_strong: Color,
}

/// Tema oscuro por defecto — análogo al `nahual-theme` dark default.
impl Default for Palette {
    fn default() -> Self {
        Self {
            bg_app: Color::from_rgba8(14, 16, 22, 255),
            bg_panel: Color::from_rgba8(22, 26, 36, 255),
            fg_text: Color::from_rgba8(214, 222, 232, 255),
            fg_muted: Color::from_rgba8(140, 152, 170, 255),
            border_strong: Color::from_rgba8(70, 84, 110, 255),
        }
    }
}

/// Color semántico de un estado de coherencia. Fijo, no temático: el rojo
/// de "conflicto" y el ámbar de "pendiente" son señales, no estilo.
pub fn tone_color(tone: CoherenceTone) -> Color {
    match tone {
        // hsl(145°, 42%, 55%) ≈ rgb(94, 184, 124) — verde coherencia
        CoherenceTone::Valid => Color::from_rgba8(94, 184, 124, 255),
        // hsl(42°, 82%, 58%) ≈ rgb(238, 178, 53) — ámbar pendiente
        CoherenceTone::Pending => Color::from_rgba8(238, 178, 53, 255),
        // hsl(2°, 70%, 58%) ≈ rgb(225, 84, 75) — rojo conflicto
        CoherenceTone::Conflict => Color::from_rgba8(225, 84, 75, 255),
    }
}

/// Etiqueta corta de un tono — para leyendas.
pub fn tone_label(tone: CoherenceTone) -> &'static str {
    match tone {
        CoherenceTone::Valid => "coherente",
        CoherenceTone::Pending => "por evaluar",
        CoherenceTone::Conflict => "en conflicto",
    }
}

/// Compone el plan completo en un árbol `View`: capa de conectores al
/// fondo, bloques y marcas encima. El nodo raíz mide exactamente el
/// contenido — envolverlo en un contenedor con clipping para documentos
/// largos (Llimphi todavía no implementa scroll; los bloques fuera del
/// viewport quedan recortados por la superficie).
pub fn editor_view<Msg: Clone + 'static>(plan: &RenderPlan, palette: &Palette) -> View<Msg> {
    let cfg = plan.config;
    let content_w = plan
        .blocks
        .iter()
        .map(|b| b.x + b.w)
        .fold(0.0f32, f32::max)
        + cfg.margin;
    let content_h = plan.content_height.max(cfg.margin * 2.0);

    let mut children: Vec<View<Msg>> = Vec::new();
    // Aristas al fondo: las pinta primero, los bloques las tapan al cruzarlas.
    for e in &plan.edges {
        children.extend(edge_segments::<Msg>(e, palette.border_strong));
    }
    for b in &plan.blocks {
        children.push(block_view::<Msg>(b, palette));
    }
    for m in &plan.sidepane {
        children.push(mark_view::<Msg>(m, &cfg));
    }

    View::new(Style {
        position: Position::Relative,
        size: Size {
            width: length(content_w.max(cfg.margin * 2.0)),
            height: length(content_h),
        },
        ..Default::default()
    })
    .children(children)
}

// ---------------------------------------------------------------------
// Bloques y marcas
// ---------------------------------------------------------------------

/// Caja absoluta de un átomo: borde tonal + interior con meta + preview.
fn block_view<Msg: Clone + 'static>(b: &AtomBlock, palette: &Palette) -> View<Msg> {
    let meta = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        format!(
            "{}  ·  profundidad {}  ·  {}",
            b.branch,
            b.depth,
            tone_label(b.tone)
        ),
        10.0,
        palette.fg_muted,
        Alignment::Start,
    );

    let preview = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(b.preview.clone(), 13.0, palette.fg_text, Alignment::Start);

    // Interior: bg del panel, padding, dos filas de texto.
    let inner = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(3.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .radius(3.0)
    .children(vec![meta, preview]);

    // Exterior: borde tonal (2 px) absolutamente posicionado.
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(b.x),
            top: length(b.y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(b.w),
            height: length(b.h),
        },
        padding: Rect {
            left: length(2.0_f32),
            right: length(2.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(tone_color(b.tone))
    .radius(5.0)
    .children(vec![inner])
}

/// Marca del osciloscopio de coherencia en el sidepane.
fn mark_view<Msg: Clone + 'static>(
    m: &SidepaneMark,
    cfg: &pluma_render_plan::LayoutConfig,
) -> View<Msg> {
    let usable = (cfg.sidepane_width - 8.0).max(4.0);
    let w = (m.intensity * usable).max(3.0);
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(cfg.margin),
            top: length(m.y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(w),
            height: length(m.h),
        },
        ..Default::default()
    })
    .fill(tone_color(m.tone))
    .radius(3.0)
}

// ---------------------------------------------------------------------
// Conectores
// ---------------------------------------------------------------------

/// Devuelve los tres rectángulos que dibujan el codo en S de una arista
/// del prerrequisito al dependiente: vertical baja, horizontal cruza,
/// vertical baja. Si origen y destino están alineados verticalmente, el
/// tramo horizontal degenera a un punto — se omite y queda un único
/// segmento vertical.
fn edge_segments<Msg: Clone + 'static>(e: &Edge, color: Color) -> Vec<View<Msg>> {
    let stroke = 1.6f32;
    let half = stroke * 0.5;
    let mid_y = (e.y1 + e.y2) * 0.5;
    let mut out = Vec::with_capacity(3);

    // Tramo 1: vertical desde (x1, y1) hasta (x1, mid_y).
    out.push(line_view::<Msg>(
        e.x1 - half,
        e.y1,
        stroke,
        (mid_y - e.y1).abs().max(stroke),
        color,
    ));
    // Tramo 2: horizontal a la altura `mid_y` cruzando entre x1 y x2.
    if (e.x2 - e.x1).abs() > stroke {
        let (x_l, x_r) = if e.x1 < e.x2 {
            (e.x1, e.x2)
        } else {
            (e.x2, e.x1)
        };
        out.push(line_view::<Msg>(
            x_l - half,
            mid_y - half,
            (x_r - x_l) + stroke,
            stroke,
            color,
        ));
    }
    // Tramo 3: vertical desde (x2, mid_y) hasta (x2, y2).
    out.push(line_view::<Msg>(
        e.x2 - half,
        mid_y,
        stroke,
        (e.y2 - mid_y).abs().max(stroke),
        color,
    ));
    out
}

/// Rectángulo delgado absolutamente posicionado — el "pincel" de un tramo.
fn line_view<Msg: Clone + 'static>(x: f32, y: f32, w: f32, h: f32, color: Color) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(w),
            height: length(h),
        },
        ..Default::default()
    })
    .fill(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tones_have_distinct_colors() {
        let v = tone_color(CoherenceTone::Valid);
        let p = tone_color(CoherenceTone::Pending);
        let c = tone_color(CoherenceTone::Conflict);
        assert!(v.components != p.components);
        assert!(p.components != c.components);
        assert!(v.components != c.components);
    }

    #[test]
    fn tone_labels_are_set() {
        assert_eq!(tone_label(CoherenceTone::Conflict), "en conflicto");
    }
}
