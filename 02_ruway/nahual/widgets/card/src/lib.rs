//! `nahual-widget-card` — container card-shape para entries de
//! timeline, info cards y similares.
//!
//! Aporta la **forma**: padding consistente (12/8), `rounded(4)`,
//! `flex_col` con `gap(2)`. NO aporta colores — el caller decide
//! `bg`, `border_color`, etc. via builder calls. Esto permite que
//! distintos consumers (timeline con accent por kind, info card
//! con bg uniforme) compartan la misma proporción visual sin
//! acoplarse a una paleta fija.
//!
//! # Ejemplo
//!
//! ```ignore
//! use nahual_widget_card::card;
//! use gpui::{rgb, prelude::*, px};
//!
//! // Card con accent border-l (típico timeline entry):
//! let entry = card()
//!     .bg(rgb(0x1d2128))
//!     .border_l_4()
//!     .border_color(rgb(0x88c0d0))
//!     .child(div().child("header"))
//!     .child(div().child("body"));
//!
//! // Card sin border (info card uniforme):
//! let info = card()
//!     .bg(rgb(0x1d2128))
//!     .child("contenido");
//! ```

#![forbid(unsafe_code)]

use gpui::{div, prelude::*, px, App, Div};
use nahual_theme::Theme;

/// Container card-shape: `flex_col` con padding `12/8`, `rounded(4)`,
/// `gap(2)` interno entre children y `mb(4)` para separación
/// vertical de cards apiladas.
///
/// Sin colores aplicados — el caller agrega `.bg(...)`,
/// `.border_color(...)`, `.border_l_4()`, etc. según necesite.
///
/// El return es un `Div` GPUI — todas las builder methods de div
/// están disponibles (children, hover, on_click, ids, etc.).
pub fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .px(px(12.))
        .py(px(8.))
        .mb(px(4.))
        .rounded(px(4.))
        .gap(px(2.))
}

/// Variante themed: igual que [`card`] pero pre-aplica `bg(panel)`
/// del [`Theme`] global. El caller no necesita conocer la paleta —
/// el bg sigue al theme actual cuando éste cambia.
///
/// Si la app no instaló un Theme, esta función panicea (gpui's
/// `cx.global::<Theme>()` requiere el global instalado). Para apps
/// sin theme, usar [`card`] directo.
pub fn card_themed(cx: &App) -> Div {
    let theme = Theme::global(cx);
    card().bg(theme.bg_panel.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity smoke: el constructor devuelve un Div sin panic. No
    /// podemos asertar las property de styling sin renderear (que
    /// requiere TestAppContext + window). Si la signature cambia,
    /// el código no compila — eso es la real garantía.
    #[test]
    fn card_returns_div_without_panic() {
        let _d = card();
    }
}
