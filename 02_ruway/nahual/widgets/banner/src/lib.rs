//! `nahual-widget-banner` — tiras horizontales de status.
//!
//! Cuatro variants con paleta consistente entre apps:
//!
//! - [`Banner::Info`] — azul tenue, mensajes neutros.
//! - [`Banner::Success`] — verde, confirmaciones de op exitosa
//!   (toasts típicos).
//! - [`Banner::Warning`] — amber, llamadas de atención (modales
//!   de confirmación, condiciones de "por las dudas").
//! - [`Banner::Error`] — rojo, errores fatales o de carga.
//!
//! Diseño: una `Div` GPUI con paddings + colors hardcoded por
//! variant. El caller añade niños via el builder de div (`.child(...)`,
//! `.flex()`, etc.) para customizar más allá del default.
//!
//! # Ejemplo
//!
//! ```ignore
//! use nahual_widget_banner::{banner, Banner};
//!
//! // Toast simple (success):
//! let toast = banner(Banner::Success, "guardado");
//!
//! // Banner de error con extra child:
//! let err = banner(Banner::Error, "no pude leer log").child(
//!     div().text_size(px(10.)).child("(timeout 3s)")
//! );
//! ```

#![forbid(unsafe_code)]

use gpui::{div, hsla, prelude::*, px, App, Background, Div, Hsla, Rgba, SharedString};
use nahual_theme::Theme;

/// Severidad / tono del banner. Determina los colores del fondo,
/// texto y border (si aplica). El caller no debería mezclar
/// kinds en un mismo banner — usar la composición de divs si
/// hace falta una vista híbrida.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Banner {
    Info,
    Success,
    Warning,
    Error,
}

impl Banner {
    /// Color de fondo del banner (sin alpha).
    pub fn bg(self) -> Rgba {
        match self {
            Banner::Info => gpui::rgb(0x1d2a3a),
            Banner::Success => gpui::rgb(0x2d3a2a),
            Banner::Warning => gpui::rgb(0x4a3a1a),
            Banner::Error => gpui::rgb(0x4a2020),
        }
    }

    /// Color del texto principal del banner.
    pub fn fg(self) -> Rgba {
        match self {
            Banner::Info => gpui::rgb(0xc0d0e0),
            Banner::Success => gpui::rgb(0xc0e0a0),
            Banner::Warning => gpui::rgb(0xf0e0a0),
            Banner::Error => gpui::rgb(0xffd0d0),
        }
    }
}

/// Construye un banner con el `kind` indicado y `message` como
/// texto principal. Devuelve un [`Div`] al que el caller puede
/// agregar children, `id`, handlers, etc.
///
/// Padding y text_size son los defaults estándar del repo
/// (`px(12./6.)` en cada axis, `px(11.)` para el texto). Para un
/// banner más grande/chico, llamar `.text_size(...)` / `.px(...)`
/// sobre el resultado.
pub fn banner(kind: Banner, message: impl Into<SharedString>) -> Div {
    div()
        .px(px(12.))
        .py(px(6.))
        .bg(kind.bg())
        .text_color(kind.fg())
        .text_size(px(11.))
        .child(message.into())
}

/// Variante themed de [`banner`]: deriva colores siguiendo el
/// `Theme::global(cx).is_dark` (lightness flip dark ↔ light) +
/// hue fijo por kind (verde para Success, amber para Warning,
/// rojo para Error). Info usa `theme.bg_panel_alt` + `theme.accent`
/// para integrarse al chrome del app.
///
/// Beneficio sobre [`banner`]: cuando el usuario cambia de theme
/// claro a oscuro, los banners ajustan contraste sin esfuerzo.
///
/// Si la app no instaló un `Theme`, panicea (`Theme::global` lo
/// requiere). Para apps sin theme, usar [`banner`] directo.
pub fn banner_themed(cx: &App, kind: Banner, message: impl Into<SharedString>) -> Div {
    let theme = Theme::global(cx);
    let (bg, fg) = themed_colors(kind, theme);
    div()
        .px(px(12.))
        .py(px(6.))
        .bg(bg)
        .text_color(fg)
        .text_size(px(11.))
        .child(message.into())
}

/// Deriva el par `(bg, fg)` para un kind dado contra el theme.
/// Public para tests + para que los consumers puedan computar el
/// par sin construir el div (ej. para custom layouts).
pub fn themed_colors(kind: Banner, theme: &Theme) -> (Background, Hsla) {
    match kind {
        Banner::Info => (theme.bg_panel_alt.clone(), theme.accent),
        Banner::Success => derive_pair(120.0 / 360.0, theme.is_dark),
        Banner::Warning => derive_pair(40.0 / 360.0, theme.is_dark),
        Banner::Error => derive_pair(0.0 / 360.0, theme.is_dark),
    }
}

/// Computa `(bg, fg)` para un hue fijo respetando dark/light mode:
/// dark → bg low-lightness, fg high-lightness; light → invertido.
fn derive_pair(hue: f32, is_dark: bool) -> (Background, Hsla) {
    let (bg_l, fg_l) = if is_dark { (0.18, 0.85) } else { (0.92, 0.20) };
    let bg_hsla = hsla(hue, 0.40, bg_l, 1.0);
    let fg_hsla = hsla(hue, 0.40, fg_l, 1.0);
    (bg_hsla.into(), fg_hsla)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_has_distinct_bg_color() {
        // Sanity: ningún kind comparte bg con otro. Si emerge una
        // versión "low-contrast" de algún kind, abrir en otro
        // variant en vez de re-usar el color.
        let bgs = [
            Banner::Info.bg(),
            Banner::Success.bg(),
            Banner::Warning.bg(),
            Banner::Error.bg(),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for b in &bgs {
            assert!(
                seen.insert((b.r * 1000.0) as u32 + (b.g * 1000.0) as u32 * 1000),
                "bg colors collision"
            );
        }
    }

    #[test]
    fn derive_pair_dark_uses_low_bg_and_high_fg() {
        let (_bg, fg) = derive_pair(0.0, true);
        // En dark mode, fg lightness es alta para contraste.
        assert!(
            fg.l > 0.7,
            "fg lightness debería ser alta en dark, got {}",
            fg.l
        );
    }

    #[test]
    fn derive_pair_light_uses_high_bg_and_low_fg() {
        let (_bg, fg) = derive_pair(0.0, false);
        // En light mode, fg lightness es baja para contraste.
        assert!(
            fg.l < 0.3,
            "fg lightness debería ser baja en light, got {}",
            fg.l
        );
    }

    #[test]
    fn derive_pair_distinguishes_kinds_by_hue() {
        // Success/Warning/Error tienen hue distinto; bg lightness
        // sigue al is_dark de igual forma cross-kind. Así verificar
        // que cambiar el hue cambia bg.h (no la lightness).
        let (_, fg_success) = derive_pair(120.0 / 360.0, true);
        let (_, fg_warning) = derive_pair(40.0 / 360.0, true);
        let (_, fg_error) = derive_pair(0.0, true);
        assert!(
            fg_success.h != fg_warning.h,
            "success y warning deben diferir en hue"
        );
        assert!(fg_warning.h != fg_error.h);
    }

    #[test]
    fn each_kind_has_distinct_fg_color() {
        let fgs = [
            Banner::Info.fg(),
            Banner::Success.fg(),
            Banner::Warning.fg(),
            Banner::Error.fg(),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for f in &fgs {
            assert!(
                seen.insert((f.r * 1000.0) as u32 + (f.g * 1000.0) as u32 * 1000)
            );
        }
    }
}
