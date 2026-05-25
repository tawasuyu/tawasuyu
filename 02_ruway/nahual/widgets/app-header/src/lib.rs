//! `nahual-widget-app-header` — tira superior estándar de las apps
//! del repo.
//!
//! Compone:
//! - Label dinámico a la izquierda (flex_grow).
//! - [`theme_switcher`] a la derecha.
//! - bg = `theme.bg_panel`, text = `theme.fg_text`,
//!   border-bottom = `theme.border`.
//! - Padding 16/12, text_size 14.
//!
//! Patrón emergente: `nakui-explorer`, `chasqui-explorer`,
//! `minga-explorer`, `brahman-broker-explorer` declaran headers
//! idénticos sólo cambiando el label. Ahora es 1 línea.
//!
//! # Ejemplo
//!
//! ```ignore
//! use nahual_widget_app_header::app_header;
//!
//! let header = app_header(cx, format!("Log: {}  ·  {} entries", path, n));
//! div().child(header).child(body)
//! ```

#![forbid(unsafe_code)]

use gpui::{div, prelude::*, px, App, IntoElement, SharedString};
use nahual_theme::Theme;
use nahual_widget_theme_switcher::theme_switcher;

/// Construye el header standard. Lee `Theme::global(cx)` para los
/// colors; falla si no hay theme instalado (panic propagado de
/// `Theme::global`).
///
/// `label` es texto plano. Para labels más ricos (ej. icon + text,
/// múltiples spans), usar [`app_header_with`] que acepta
/// cualquier child element.
pub fn app_header(cx: &mut App, label: impl Into<SharedString>) -> impl IntoElement {
    let label: SharedString = label.into();
    app_header_with(cx, div().child(label))
}

/// Variante de [`app_header`] que acepta cualquier `IntoElement`
/// como contenido del lado izquierdo. El widget envuelve el child
/// en un `div().flex_grow()` para que el switcher quede pegado a
/// la derecha.
pub fn app_header_with(cx: &mut App, label_child: impl IntoElement) -> impl IntoElement {
    let theme = Theme::global(cx).clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .px(px(16.))
        .py(px(12.))
        .bg(theme.bg_panel.clone())
        .border_b_1()
        .border_color(theme.border)
        .text_color(theme.fg_text)
        .text_size(px(14.))
        .child(div().flex_grow().child(label_child))
        .child(theme_switcher(cx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn app_header_constructs_with_string_label(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Theme::install_default(cx);
            let _h = app_header(cx, "Test header");
        });
    }

    #[gpui::test]
    fn app_header_with_accepts_arbitrary_child(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Theme::install_default(cx);
            let _h = app_header_with(
                cx,
                div().child(SharedString::from("Custom child")),
            );
        });
    }

    #[gpui::test]
    fn app_header_label_accepts_owned_or_borrowed(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Theme::install_default(cx);
            let _ = app_header(cx, "literal");
            let _ = app_header(cx, "owned".to_string());
            let _ = app_header(cx, format!("formatted {}", 42));
        });
    }
}
