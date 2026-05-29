//! `llimphi-widget-wawa-mark` — sello vectorial del SO wawa.
//!
//! ## Spec (revisión 2026-05-29)
//!
//! Identidad nominal **implícita**: el rombo de fondo lleva la paleta
//! oficial (Azul Índigo / Púrpura Profundo) y los trazos blancos forman
//! las letras **"WA"** pero geométricamente — no son tipografía, son
//! aristas internas que rebotan en los mismos 45° del rombo, así dan
//! sensación de facetas talladas dentro del diamante.
//!
//! ### Composición
//!
//! 1. **Rombo de fondo** — degradado vertical inmaculado, sin sutura
//!    visible: índigo arriba, púrpura abajo. El degradado lineal cubre
//!    toda la altura del rombo (no sólo la mitad), de modo que el cambio
//!    de tono es continuo.
//! 2. **Trazo "WA"** — un único `BezPath` con dos subtrazos:
//!    - **W** (izquierda): zigzag de 4 segmentos, todos a 45° (matching
//!      las aristas del rombo). Picos en la sutura azul/púrpura
//!      (y = 0.50), valles en y = 0.60. Cinco vértices, cuatro segmentos.
//!    - **A** (derecha): triángulo abierto formado por dos legs a 45°
//!      + un crossbar horizontal a mitad de altura. Tres segmentos.
//!    Las strokes diagonales (6 de las 7) son paralelas a las aristas
//!    del rombo, por eso "leen" como filos cortados del diamante en vez
//!    de letras pintadas encima.
//! 3. **Merkle Core** — punto luminoso con halo en el pico central de
//!    la W (sobre la sutura, donde azul y púrpura se encuentran). Es el
//!    nodo raíz que amarra el sistema.
//!
//! ### Geometría (en coords normalizadas `[0, 1] × [0, 1]` del rect)
//!
//! ```text
//!                            Top
//!                             ◇
//!                            / \
//!                           /   \              ← azul índigo
//!                          /     \
//!                P0    P2★    P4 A1
//!                  ●─.   ●  .─●  ●─.  .─●     ← y = 0.50 (sutura)
//!                     ╲ ╱ ╲ ╱     ╲      ╱
//!                      ╳   ╳       ╲────╱     ← crossbar A (y=0.55)
//!                     ╱ ╲ ╱ ╲     ╱      ╲
//!                  ●─'   ●  '─●  ●─'    '─●  ← y = 0.60 (valles/pies)
//!                P1    P3    A0    A2
//!                          ↑
//!                          gap entre W y A
//!     Left  ◇─────────────────────────────────◇  Right
//!                          (sutura, y = 0.50)
//!                          /
//!                         /
//!                        /                     ← púrpura profundo
//!                       /
//!                      ◇
//!                    Bottom
//! ```
//!
//! Las strokes diagonales todas a slope ±1, igual que las aristas del
//! rombo. El crossbar de la A es la única horizontal — concesión mínima
//! a la legibilidad de la letra, queda subordinado al patrón diamante.
//!
//! ## Uso
//!
//! ```ignore
//! use llimphi_widget_wawa_mark::{wawa_mark_view, WawaMarkPalette};
//!
//! // En un view:
//! View::new(Style { size: Size { width: length(128.0), height: length(128.0) }, ..Default::default() })
//!     .children(vec![wawa_mark_view(&WawaMarkPalette::default())])
//! ```
//!
//! El widget rellena el rect del padre — pasarle un tamaño cuadrado para
//! que el rombo no se distorsione (lo respeta igual, pero queda mejor
//! cuadrado).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{percent, Size, Style},
    Position,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Point, Stroke};
use llimphi_ui::llimphi_raster::peniko::{color::AlphaColor, Color, Fill, Gradient, Mix};
use llimphi_ui::View;

/// Paleta del sello. Los defaults corresponden a la especificación
/// oficial (Azul Índigo + Púrpura Profundo + trazo blanco + acento
/// cyan-eléctrico para el Merkle Core).
#[derive(Debug, Clone, Copy)]
pub struct WawaMarkPalette {
    /// Color superior del degradado (tope del rombo).
    pub indigo: Color,
    /// Color inferior del degradado (base del rombo).
    pub purple: Color,
    /// Color del trazo de la 'W' implícita.
    pub stroke: Color,
    /// Color del Merkle Core (nodo central). Halo se deriva con alpha
    /// reducido del mismo color.
    pub core: Color,
}

impl Default for WawaMarkPalette {
    fn default() -> Self {
        Self {
            // Azul Índigo profundo — saturación alta, valor medio.
            indigo: Color::from_rgba8(46, 56, 168, 255),
            // Púrpura Profundo — más violeta, valor menor.
            purple: Color::from_rgba8(76, 32, 122, 255),
            // Blanco con leve calidez para no quemar contra el púrpura.
            stroke: Color::from_rgba8(240, 240, 248, 255),
            // Cyan eléctrico — el "color del cursor del osciloscopio".
            core: Color::from_rgba8(120, 240, 255, 255),
        }
    }
}

/// Construye el `View` que pinta el sello dentro del rect del padre.
/// El widget se posiciona absolute al 100% del padre — pasarle un
/// contenedor con tamaño cuadrado para evitar distorsión.
pub fn wawa_mark_view<Msg: Clone + 'static>(palette: &WawaMarkPalette) -> View<Msg> {
    let p = *palette;
    View::new(Style {
        position: Position::Absolute,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| paint_mark(scene, rect, &p))
}

/// Pintor puro — recibe el `Scene`, el rect de pintura y la paleta.
/// Expuesto por separado para que apps avanzadas puedan reusar el
/// painter dentro de canvas custom (splash de boot, about box, etc.)
/// sin pasar por la fachada `View`.
pub fn paint_mark(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: llimphi_ui::PaintRect,
    palette: &WawaMarkPalette,
) {
    // Encajamos el rombo en el menor de los lados del rect, centrado.
    // Así el sello mantiene su proporción incluso si el rect no es
    // cuadrado (pero degrada gracilmente).
    let side = rect.w.min(rect.h) as f64;
    let cx = rect.x as f64 + rect.w as f64 * 0.5;
    let cy = rect.y as f64 + rect.h as f64 * 0.5;
    let half = side * 0.5;

    // === 1) Rombo de fondo con degradado vertical ===
    //
    // Construimos el rombo como BezPath (4 segmentos rectos) en coords
    // absolutas. El degradado lineal va de (cx, top) a (cx, bot) — toda
    // la altura del rombo — para que el cambio de tono sea continuo y
    // sin sutura visible.
    let top = Point::new(cx, cy - half);
    let right = Point::new(cx + half, cy);
    let bot = Point::new(cx, cy + half);
    let left = Point::new(cx - half, cy);

    let mut rhombus = BezPath::new();
    rhombus.move_to(top);
    rhombus.line_to(right);
    rhombus.line_to(bot);
    rhombus.line_to(left);
    rhombus.close_path();

    let gradient = Gradient::new_linear(top, bot)
        .with_stops([palette.indigo, palette.purple].as_slice());

    scene.fill(Fill::NonZero, Affine::IDENTITY, &gradient, None, &rhombus);

    // === 2) "WA" implícita ===
    //
    // Coords en porcentaje del rombo (origen = esquina top-left del bbox
    // del rombo = (cx-half, cy-half), unidad = side). Toda stroke diagonal
    // tiene |dy/dx| = 1 (paralela a las aristas del rombo) — por eso lee
    // como faceta del diamante en vez de letra dibujada encima.
    let coord = |fx: f64, fy: f64| -> Point {
        Point::new(
            cx - half + fx * side,
            cy - half + fy * side,
        )
    };

    // Unidad de escala: span vertical de las letras. dx==dy en cada leg
    // hace que las strokes corran a 45° exactos (mismo ángulo que las
    // aristas del rombo). Probado para que WA quede inscrita con holgura
    // en el rombo a cualquier escala — al achicar (32px) sigue legible,
    // al ampliar (300px) no se ve disperso.
    let unit: f64 = 0.10;
    // Línea de picos en la sutura azul/púrpura.
    let top_y = 0.50;
    // Línea de valles/pies en el cuadrante púrpura inferior.
    let bot_y = top_y + unit;

    // ---- W (zigzag de 4 segmentos) ----
    // Centramos la composición WA: span total ≈ 0.61 (W 0.36 + gap 0.03
    // + A 0.18 + holgura). Empezamos en x = 0.19 para que el centro
    // óptico de WA caiga cerca de x = 0.50.
    let w_left = 0.20;
    let p0 = coord(w_left + 0.0 * unit, top_y);
    let p1 = coord(w_left + 1.0 * unit, bot_y);
    let p2 = coord(w_left + 2.0 * unit, top_y);
    let p3 = coord(w_left + 3.0 * unit, bot_y);
    let p4 = coord(w_left + 4.0 * unit, top_y);

    // ---- A (legs + crossbar) ----
    // Gap entre W y A — apenas un respiro para que no se confundan en
    // un solo zigzag.
    let gap = 0.04;
    let a_left = w_left + 4.0 * unit + gap;
    let a0 = coord(a_left + 0.0 * unit, bot_y);
    let a1 = coord(a_left + 1.0 * unit, top_y);
    let a2 = coord(a_left + 2.0 * unit, bot_y);
    // Crossbar a mitad de altura, en el tercio interno de cada leg para
    // que no toque las puntas (queda más A que H).
    let cross_y = (top_y + bot_y) * 0.5 + 0.005; // un toque debajo del medio óptico
    let c_offset = 0.30 * unit;
    let cb0 = coord(a_left + 0.0 * unit + c_offset, cross_y);
    let cb1 = coord(a_left + 2.0 * unit - c_offset, cross_y);

    // Un único BezPath con cuatro subtrazos (move_to abre subtrazo nuevo).
    let mut wa = BezPath::new();
    // W
    wa.move_to(p0);
    wa.line_to(p1);
    wa.line_to(p2);
    wa.line_to(p3);
    wa.line_to(p4);
    // A — legs.
    wa.move_to(a0);
    wa.line_to(a1);
    wa.line_to(a2);
    // A — crossbar (horizontal, único trazo no diagonal).
    wa.move_to(cb0);
    wa.line_to(cb1);

    // Espesor escalable: ~2.0% del lado del rombo. Levemente más fino
    // que la W sola, porque ahora hay 7 strokes en vez de 4 y conviene
    // bajar densidad.
    let stroke_w = (side * 0.020).max(1.0);
    let stroke = Stroke::new(stroke_w)
        .with_join(llimphi_ui::llimphi_raster::kurbo::Join::Miter)
        .with_caps(llimphi_ui::llimphi_raster::kurbo::Cap::Butt);

    scene.stroke(
        &stroke,
        Affine::IDENTITY,
        palette.stroke,
        None,
        &wa,
    );

    // === 3) Merkle Core ===
    //
    // Sobre P2 — pico central de la W, en la sutura exacta entre azul y
    // púrpura. Halo amplio semi-transparente + núcleo opaco compacto
    // dan sensación de glow sin blur real.
    let core_r = (side * 0.018).max(1.2);
    let halo_r = core_r * 2.6;
    let halo_color = with_alpha(palette.core, 0.30);
    scene.push_layer(Mix::Normal, 1.0, Affine::IDENTITY, &Circle::new(p2, halo_r));
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        halo_color,
        None,
        &Circle::new(p2, halo_r),
    );
    scene.pop_layer();
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        palette.core,
        None,
        &Circle::new(p2, core_r),
    );
}

/// Devuelve `color` con su alpha multiplicado por `mult` (no reemplazado).
/// Mantenemos la cromaticidad intacta.
fn with_alpha(color: Color, mult: f32) -> Color {
    let [r, g, b, a] = color.components;
    AlphaColor::new([r, g, b, a * mult])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_has_distinct_indigo_and_purple() {
        let p = WawaMarkPalette::default();
        assert_ne!(p.indigo.components, p.purple.components);
        assert_ne!(p.stroke.components, p.core.components);
    }

    #[test]
    fn with_alpha_multiplies_not_replaces() {
        let c = Color::from_rgba8(100, 100, 100, 255);
        let halved = with_alpha(c, 0.5);
        assert!((halved.components[3] - 0.5).abs() < 1e-3);
        // RGB intactos.
        assert!((halved.components[0] - c.components[0]).abs() < 1e-3);
    }
}
