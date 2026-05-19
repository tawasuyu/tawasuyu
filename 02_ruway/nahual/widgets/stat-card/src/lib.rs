//! `nahual-widget-stat-card` — tarjeta de dashboard con accent.
//!
//! Compone:
//! - **`card_themed(cx)`** del [`nahual_widget_card`] como contenedor.
//! - **Border-l-4** con un color de accent que el caller decide
//!   (verde = OK, rojo = error, etc.).
//! - **Label** chico arriba en el color del accent.
//! - **Value** grande (`px(28)`) en el color principal del text.
//! - **Description** chica en el color tenue.
//! - **Listing opcional** de items recientes con sub-header
//!   `"recent (N de TOTAL):"`.
//!
//! El patrón emerge en dashboards estilo `minga-explorer` (counts
//! del repo + sample) y `brahman-broker-explorer` (estado del
//! probe). Cada consumer aporta sus propios accents semánticos.
//!
//! El widget no asume valor numérico — `value` es
//! `Into<SharedString>`, así que sirve igual para counts (`"3"`),
//! status text (`"UP / PROVIDER"`) o cualquier label corto.
//!
//! # Ejemplo
//!
//! ```ignore
//! use nahual_widget_stat_card::stat_card;
//! use gpui::{rgb, Hsla};
//!
//! let cell = stat_card(
//!     cx,
//!     "Nodos AST",
//!     "247",
//!     "fragments parseados del código",
//!     rgb(0x88c0d0),
//!     theme.fg_text,
//!     theme.fg_muted,
//!     &["abc123  fn_decl".into(), "def456  expr".into()],
//! );
//! ```

#![forbid(unsafe_code)]

use gpui::{div, prelude::*, px, App, IntoElement, SharedString};
use nahual_widget_card::card_themed;

/// Construye una stat card. Devuelve `impl IntoElement` para que el
/// caller pueda meterla directo como child de cualquier
/// `flex_col`/`gap` parent.
///
/// Args:
/// - `cx` — `&App` (acepta `&Context<T>` por deref). El widget lee
///   el theme global para el bg de la card.
/// - `label` — header chico, en el color del accent.
/// - `value` — texto principal, render grande (`px(28)`).
/// - `description` — texto chico tenue debajo del value.
/// - `accent` — color del border-l y del label.
/// - `text` — color principal (para el value).
/// - `text_dim` — color tenue (para description y sub-header de
///   recent).
/// - `recent_items` — slice de strings; si no vacío, se renderea
///   como sub-listing con header `"recent (N de TOTAL):"`. Cada
///   item ocupa una linea.
#[allow(clippy::too_many_arguments)]
pub fn stat_card(
    cx: &App,
    label: &str,
    value: impl Into<SharedString>,
    description: &str,
    accent: gpui::Rgba,
    text: gpui::Hsla,
    text_dim: gpui::Hsla,
    recent_items: &[String],
) -> impl IntoElement {
    let value: SharedString = value.into();
    let total_for_header = recent_items.len();

    let mut card = card_themed(cx)
        .border_l_4()
        .border_color(accent)
        .child(
            div()
                .text_color(accent)
                .text_size(px(11.))
                .child(SharedString::from(label.to_string())),
        )
        .child(
            div()
                .text_color(text)
                .text_size(px(28.))
                .child(value),
        )
        .child(
            div()
                .text_color(text_dim)
                .text_size(px(11.))
                .child(SharedString::from(description.to_string())),
        );

    if !recent_items.is_empty() {
        // Sub-header indicando cuántos items se muestran.
        // El "TOTAL" es el len del slice porque el caller ya lo
        // truncó — no tenemos acceso al total original. Si el
        // caller quiere "5 de 247", debe formatear el label/value
        // con el total.
        card = card.child(
            div()
                .mt(px(6.))
                .text_color(text_dim)
                .text_size(px(10.))
                .child(SharedString::from(format!("recent ({total_for_header}):"))),
        );
        for it in recent_items {
            card = card.child(
                div()
                    .text_color(text)
                    .text_size(px(11.))
                    .child(SharedString::from(it.clone())),
            );
        }
    }

    card
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use nahual_theme::Theme;

    /// Smoke test: el constructor lee el theme global y devuelve un
    /// IntoElement. Sin TestAppContext no podemos asertar render
    /// pixels — esto valida wireup + type-check.
    #[gpui::test]
    fn stat_card_constructs_with_theme(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Theme::install_default(cx);
            let theme = Theme::global(cx);
            let _el = stat_card(
                cx,
                "Test",
                "42",
                "una descripción",
                gpui::rgb(0x88c0d0),
                theme.fg_text,
                theme.fg_muted,
                &[],
            );
        });
    }

    #[gpui::test]
    fn stat_card_with_recent_items_works(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Theme::install_default(cx);
            let theme = Theme::global(cx);
            let _el = stat_card(
                cx,
                "Items",
                "3",
                "items recientes:",
                gpui::rgb(0xa3be8c),
                theme.fg_text,
                theme.fg_muted,
                &["a1b2c3  foo".into(), "d4e5f6  bar".into(), "789012  baz".into()],
            );
        });
    }

    #[gpui::test]
    fn stat_card_value_accepts_string_or_number_repr(cx: &mut TestAppContext) {
        // Type-check: value es Into<SharedString>. Tanto literal
        // string como `format!()` deberían funcionar.
        cx.update(|cx| {
            Theme::install_default(cx);
            let theme = Theme::global(cx);
            let _ = stat_card(cx, "L", "literal", "d", gpui::rgb(0), theme.fg_text, theme.fg_muted, &[]);
            let _ = stat_card(cx, "L", format!("{}", 42), "d", gpui::rgb(0), theme.fg_text, theme.fg_muted, &[]);
            let _ = stat_card(cx, "L", "owned".to_string(), "d", gpui::rgb(0), theme.fg_text, theme.fg_muted, &[]);
        });
    }
}
