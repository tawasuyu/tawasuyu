//! `llimphi-widget-wawa-mark` — sello vectorial del SO wawa.
//!
//! ## Spec (2026-05-29)
//!
//! Identidad nominal **implícita**: el rombo de fondo lleva la paleta
//! oficial (Azul Índigo / Púrpura Profundo) y un único trazo blanco
//! continuo, fino y angular dibuja una 'W' geométrica perfectamente
//! simétrica usando las aristas del grafo. No hay tipografía superpuesta
//! ni letras pintadas — sólo geometría.
//!
//! ### Composición
//!
//! 1. **Rombo de fondo** — degradado vertical inmaculado, sin sutura
//!    visible: índigo arriba, púrpura abajo. El degradado lineal cubre
//!    toda la altura del rombo (no sólo la mitad), de modo que el cambio
//!    de tono es continuo.
//! 2. **Trazo de 'W' implícita** — un solo `BezPath` que arranca en la
//!    zona media-izquierda (cuadrante azul), baja al valle del cuadrante
//!    púrpura inferior-izquierdo, sube hasta tocar el centro exacto de
//!    la sutura azul/púrpura (el ecuador `y = mid`), baja simétricamente
//!    al valle púrpura inferior-derecho y sube a morir en la zona
//!    media-derecha del cuadrante azul. Cinco vértices, cuatro segmentos.
//! 3. **Merkle Core** — punto luminoso con halo en el pico central de
//!    la 'W' (sobre la sutura). Es el nodo raíz que amarra el sistema.
//!
//! ### Geometría (en coords normalizadas `[0, 1] × [0, 1]` del rect)
//!
//! ```text
//!                      Top
//!                       ◇
//!                      / \
//!                     /   \           ← azul índigo
//!                P0  ●     ●  P4      ← y ≈ 0.46  (media-izq / media-der)
//!                   /\     /\
//!                  /  \   /  \
//!                 /    ★ /    \       ← P2 = pico medio (sutura, y = 0.50)
//!                /    /│ \     \         + Merkle Core
//!     Left  ◇──/───/──┼──\──\───◇  Right
//!              /  /   │   \  \
//!             ●─/     │     \─●        ← púrpura profundo
//!             P1      │      P3       ← y ≈ 0.78  (valles)
//!               \     │     /
//!                \    │    /
//!                 \   │   /
//!                  \  │  /
//!                   \ │ /
//!                    \│/
//!                     ◇
//!                   Bottom
//! ```
//!
//! Las coords están elegidas para que (a) los segmentos sean diagonales
//! con pendiente ~constante, dando una 'W' visualmente simétrica, y (b)
//! todos los puntos queden bien dentro del rombo (sin sangrar al borde).
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

    // === 2) 'W' implícita ===
    //
    // Coords en porcentaje del rombo (origen = esquina top-left del bbox
    // del rombo = (cx-half, cy-half), unidad = side). Los valores fueron
    // ajustados para que la 'W' quede inscrita con holgura y los
    // segmentos sean visualmente equilibrados.
    //
    // - P0/P4 = (0.22, 0.46) y (0.78, 0.46)  → media-izq / media-der
    //   en zona azul, ligeramente arriba de la sutura.
    // - P1/P3 = (0.34, 0.78) y (0.66, 0.78)  → valles en cuadrante púrpura.
    // - P2 = (0.50, 0.50)                     → pico central sobre la sutura.
    let coord = |fx: f64, fy: f64| -> Point {
        Point::new(
            cx - half + fx * side,
            cy - half + fy * side,
        )
    };
    let p0 = coord(0.22, 0.46);
    let p1 = coord(0.34, 0.78);
    let p2 = coord(0.50, 0.50);
    let p3 = coord(0.66, 0.78);
    let p4 = coord(0.78, 0.46);

    let mut w_path = BezPath::new();
    w_path.move_to(p0);
    w_path.line_to(p1);
    w_path.line_to(p2);
    w_path.line_to(p3);
    w_path.line_to(p4);

    // Espesor escalable: ~2.3% del lado del rombo. A 128px = ~3px;
    // a 256px = ~6px. Mantiene nitidez sin engordar.
    let stroke_w = (side * 0.023).max(1.0);
    let stroke = Stroke::new(stroke_w)
        .with_join(llimphi_ui::llimphi_raster::kurbo::Join::Miter)
        .with_caps(llimphi_ui::llimphi_raster::kurbo::Cap::Butt);

    scene.stroke(
        &stroke,
        Affine::IDENTITY,
        palette.stroke,
        None,
        &w_path,
    );

    // === 3) Merkle Core ===
    //
    // Punto luminoso sobre P2 — el pico central de la 'W', en la sutura
    // exacta entre azul y púrpura. Renderizamos un halo (círculo grande
    // semi-transparente) + un núcleo (círculo pequeño opaco) para
    // sensación de glow sin necesidad de blur de verdad.
    let core_r = (side * 0.018).max(1.2);
    let halo_r = core_r * 2.6;

    // Halo: mismo color que el core pero con alpha bajo, encima del
    // trazo de la W para que tape la intersección. Usamos add mode no:
    // mejor un solo fill blando, que es lo que pide la spec ("punto
    // luminoso", no "destello de cámara").
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

    // Core opaco.
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
