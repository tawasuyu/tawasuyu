//! Color ramps para heatmaps. Interpolación lineal entre control points.

use pineal_render::Color;

/// Rampa de color. `sample(t)` mapea `t ∈ [0,1]` a un `Color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ramp {
    /// Viridis — perceptualmente uniforme, dark-purple → yellow.
    Viridis,
    /// Escala de grises lineal, negro → blanco.
    Grayscale,
}

impl Ramp {
    /// Mapea `t` (se clampa a `[0,1]`) a un color de la rampa.
    pub fn sample(&self, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ramp::Grayscale => Color::rgb(t, t, t),
            Ramp::Viridis => lerp_stops(t, VIRIDIS),
        }
    }
}

/// Control points de Viridis (aproximación de 5 stops del colormap real).
const VIRIDIS: &[(f32, u32)] = &[
    (0.00, 0x440154),
    (0.25, 0x3b528b),
    (0.50, 0x21918c),
    (0.75, 0x5ec962),
    (1.00, 0xfde725),
];

/// Interpola linealmente entre los stops `(pos, hex)` ordenados por `pos`.
fn lerp_stops(t: f32, stops: &[(f32, u32)]) -> Color {
    if stops.is_empty() {
        return Color::BLACK;
    }
    if t <= stops[0].0 {
        return Color::from_hex(stops[0].1);
    }
    let last = stops[stops.len() - 1];
    if t >= last.0 {
        return Color::from_hex(last.1);
    }
    for w in stops.windows(2) {
        let (p0, c0) = w[0];
        let (p1, c1) = w[1];
        if t >= p0 && t <= p1 {
            let local = (t - p0) / (p1 - p0);
            return lerp_color(Color::from_hex(c0), Color::from_hex(c1), local);
        }
    }
    Color::from_hex(last.1)
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    Color::rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grayscale_endpoints() {
        assert_eq!(Ramp::Grayscale.sample(0.0), Color::BLACK);
        assert_eq!(Ramp::Grayscale.sample(1.0), Color::WHITE);
    }

    #[test]
    fn viridis_endpoints_match_control_points() {
        assert_eq!(Ramp::Viridis.sample(0.0), Color::from_hex(0x440154));
        assert_eq!(Ramp::Viridis.sample(1.0), Color::from_hex(0xfde725));
    }

    #[test]
    fn sample_clamps_out_of_range() {
        assert_eq!(Ramp::Viridis.sample(-5.0), Ramp::Viridis.sample(0.0));
        assert_eq!(Ramp::Viridis.sample(5.0), Ramp::Viridis.sample(1.0));
    }

    #[test]
    fn viridis_midpoint_is_between() {
        let mid = Ramp::Viridis.sample(0.5);
        // El stop de 0.5 es 0x21918c.
        assert_eq!(mid, Color::from_hex(0x21918c));
    }
}
