//! `llimphi-widget-panel` — firma visual transversal de los paneles gioser.
//!
//! Aporta dos detalles que aplicados consistentemente vuelven al sistema
//! reconocible sin que se note "diseñado":
//!
//! 1. **Gradiente vertical casi imperceptible** — el fondo del panel no
//!    es un color sólido sino una interpolación lineal entre una versión
//!    ligeramente más clara (top) y una ligeramente más oscura (bot) del
//!    color base. La diferencia es ~4% en valor — invisible al primer
//!    vistazo pero el ojo lo registra como "tallado" en vez de "pintado".
//!
//! 2. **Hairline accent en el top edge** — una línea horizontal de 1px
//!    en el color accent del theme, al ~30% de alpha, justo en el borde
//!    superior del panel. Funciona como "hilo de identidad" que cose
//!    todos los paneles del sistema: aparece en modales, dropdowns,
//!    cards, sidebars; siempre el mismo grosor, siempre el mismo color.
//!
//! ## API
//!
//! - [`PanelStyle`] — bundle de tokens (color base, accent, radio,
//!   alpha del hairline, fuerza del gradiente).
//! - [`panel_signature_painter`] — `Fn` para `View::paint_with`. Útil si
//!   ya tenés un View configurado y querés sumarle la firma sin envolver.
//! - [`panel_view`] — convenience: arma el View completo con la firma
//!   aplicada, recibe los hijos como `Vec<View<Msg>>`.
//!
//! ## Cuándo usarlo
//!
//! - SÍ: modales, dropdowns, cards prominentes, columnas de layout,
//!   shortcuts-help, paneles flotantes.
//! - NO: chips, badges, toasts, items de lista (la firma es para
//!   superficies grandes; en piezas chiquitas es ruido).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, Rect as KurboRect, RoundedRect};
use llimphi_ui::llimphi_raster::peniko::{color::AlphaColor, Color, Fill, Gradient};
use llimphi_ui::{PaintRect, View};
use llimphi_theme::{alpha, radius, Theme};

/// Token bundle de la firma visual.
#[derive(Debug, Clone, Copy)]
pub struct PanelStyle {
    /// Color base del panel (típico: `theme.bg_panel`).
    pub bg_base: Color,
    /// Color del hairline (típico: `theme.accent`).
    pub accent: Color,
    /// Radio de las esquinas (típico: `radius::MD` para cards, `radius::LG`
    /// para modales/overlays).
    pub radius: f64,
    /// Alpha del hairline (0.0–1.0). Por debajo de 0.20 se pierde; por
    /// encima de 0.45 se vuelve dominante. Default 0.30.
    pub hairline_alpha: f32,
    /// Fuerza del gradiente — cada componente RGB se desplaza ±gradient
    /// (en escala 0.0–1.0). 0.04 = 4% = imperceptible-pero-presente.
    /// Subir más sólo si el theme es muy claro y el efecto no llega.
    pub gradient_strength: f32,
}

impl PanelStyle {
    /// Estilo estándar para cards / sidebars / paneles medianos.
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            bg_base: t.bg_panel,
            accent: t.accent,
            radius: radius::MD,
            hairline_alpha: alpha::SCRIM as f32 / 255.0 * 1.2, // ~0.30
            gradient_strength: 0.04,
        }
    }

    /// Variante para superficies grandes — modales, splash, overlays.
    /// Esquinas más generosas, gradiente y hairline un toque más marcados.
    pub fn from_theme_large(t: &Theme) -> Self {
        Self {
            bg_base: t.bg_panel,
            accent: t.accent,
            radius: radius::LG,
            hairline_alpha: 0.35,
            gradient_strength: 0.05,
        }
    }

    /// Variante neutra — sin hairline (panels que no deben llevar la
    /// "firma" porque son piezas auxiliares). Mantiene el gradiente.
    pub fn neutral(t: &Theme) -> Self {
        Self {
            bg_base: t.bg_panel,
            accent: t.accent,
            radius: radius::MD,
            hairline_alpha: 0.0,
            gradient_strength: 0.03,
        }
    }

    /// Color del top del gradiente: base aclarada.
    pub fn bg_top(&self) -> Color {
        shift(self.bg_base, self.gradient_strength)
    }

    /// Color del bottom del gradiente: base oscurecida.
    pub fn bg_bot(&self) -> Color {
        shift(self.bg_base, -self.gradient_strength)
    }
}

/// Devuelve la closure de pintura que aplica la firma sobre el rect del
/// nodo. Pasarla a `View::paint_with` para sumar la firma a un View
/// existente. El View NO debe tener `.fill(...)` setteado — el gradient
/// reemplaza el fill sólido.
///
/// Nota: el View debe llamar `.radius(style.radius)` en sí mismo si quiere
/// que clip/hit-test/borders respeten las esquinas. La firma pinta el
/// gradiente como `RoundedRect` con el mismo `radius`, así que la
/// silueta visual es consistente independientemente del clipping.
pub fn panel_signature_painter(
    style: PanelStyle,
) -> impl Fn(&mut llimphi_ui::llimphi_raster::vello::Scene, &mut llimphi_ui::llimphi_text::Typesetter, PaintRect)
       + Send
       + Sync
       + 'static {
    move |scene, _ts, rect| {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }

        // === 1) Gradiente vertical en RoundedRect ===
        let x0 = rect.x as f64;
        let y0 = rect.y as f64;
        let x1 = (rect.x + rect.w) as f64;
        let y1 = (rect.y + rect.h) as f64;
        let rr = RoundedRect::new(x0, y0, x1, y1, style.radius);
        let gradient = Gradient::new_linear(
            Point::new(x0, y0),
            Point::new(x0, y1),
        )
        .with_stops([style.bg_top(), style.bg_bot()].as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &gradient, None, &rr);

        // === 2) Hairline accent en el top edge ===
        // Se acorta horizontalmente para no chocar con las esquinas
        // redondeadas — queda inscrito en el "techo recto" del panel.
        if style.hairline_alpha > 0.0 && rect.w > style.radius as f32 * 2.0 + 4.0 {
            let hairline_color = with_alpha_mul(style.accent, style.hairline_alpha);
            let hairline = KurboRect::new(
                x0 + style.radius,
                y0,
                x1 - style.radius,
                y0 + 1.0,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, hairline_color, None, &hairline);
        }
    }
}

/// Convenience: arma un `View` con la firma aplicada y los `children`
/// adentro. Equivalente a:
///
/// ```ignore
/// View::new(Style { size: full, ..Default::default() })
///     .paint_with(panel_signature_painter(style))
///     .radius(style.radius)
///     .clip(true)
///     .children(children)
/// ```
///
/// Para layouts custom (size específico, padding, flex direction), usar
/// `panel_signature_painter` directamente y construir el View a mano.
pub fn panel_view<Msg: Clone + 'static>(
    children: Vec<View<Msg>>,
    style: PanelStyle,
) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with(panel_signature_painter(style))
    .radius(style.radius)
    .clip(true)
    .children(children)
}

// =====================================================================
// Helpers internos
// =====================================================================

/// Desplaza cada componente RGB de `c` por `delta` (positivo aclara,
/// negativo oscurece). Clampea en [0,1]. El alpha queda intacto.
fn shift(c: Color, delta: f32) -> Color {
    let [r, g, b, a] = c.components;
    AlphaColor::new([
        (r + delta).clamp(0.0, 1.0),
        (g + delta).clamp(0.0, 1.0),
        (b + delta).clamp(0.0, 1.0),
        a,
    ])
}

fn with_alpha_mul(c: Color, mult: f32) -> Color {
    let [r, g, b, a] = c.components;
    AlphaColor::new([r, g, b, a * mult])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bg_top_is_brighter_than_bg_bot() {
        let t = Theme::dark();
        let s = PanelStyle::from_theme(&t);
        let top = s.bg_top();
        let bot = s.bg_bot();
        // El top debe tener cada canal RGB ≥ al del bot (es más claro).
        for i in 0..3 {
            assert!(top.components[i] >= bot.components[i],
                "canal {i}: top {} < bot {}", top.components[i], bot.components[i]);
        }
    }

    #[test]
    fn neutral_style_has_no_hairline() {
        let t = Theme::dark();
        let s = PanelStyle::neutral(&t);
        assert_eq!(s.hairline_alpha, 0.0);
    }

    #[test]
    fn shift_clamps_to_unit() {
        let c = Color::from_rgba8(250, 250, 250, 255);
        let bright = shift(c, 0.5);
        assert!(bright.components[0] <= 1.0);
        assert!(bright.components[1] <= 1.0);
    }
}
