//! `nahual-widget-theme-switcher` — botón clickable para ciclar
//! entre los presets de `Theme`.
//!
//! El botón muestra el nombre del theme actual; al click avanza al
//! siguiente preset según [`Theme::next_after`] (rotación circular
//! sobre [`Theme::all`]).
//!
//! El cambio se aplica con `Theme::set(cx, ...)` que invalida el
//! global y dispara redraws en todos los widgets que observan el
//! theme via `cx.observe_global::<Theme>()`. Para widgets que NO
//! observan el theme (ej. los themed wrappers de banner/card en su
//! versión actual, que leen el theme dentro de `render`), basta con
//! que el render se vuelva a invocar — esto sucede automáticamente
//! tras `cx.set_global` que marca todos los views como dirty.
//!
//! # Uso
//!
//! ```ignore
//! use nahual_widget_theme_switcher::theme_switcher;
//!
//! // Adentro de Render::render:
//! let switcher = theme_switcher(cx);
//! header.child(switcher)
//! ```

#![forbid(unsafe_code)]

use gpui::{div, prelude::*, px, App, ClickEvent, IntoElement, SharedString, Window};
use nahual_theme::Theme;

/// Construye el switcher: una `Div` clickable con el nombre del
/// theme actual + flecha indicadora. Al click rota al siguiente
/// preset.
///
/// Estilo: padding consistente con el resto de los chrome controls
/// del repo (`px(8/4)`), `bg(theme.bg_panel_alt)`, `text_color(fg_text)`.
/// Sin border, hover sutil con `bg_row_hover`.
///
/// El handler del click usa `cx.update_global::<Theme>` para
/// reemplazar el theme global; los widgets que leen
/// `Theme::global` en su próximo render verán el nuevo.
pub fn theme_switcher(cx: &mut App) -> impl IntoElement {
    let theme = Theme::global(cx).clone();
    let label = format!("Tema: {} ▸", theme.name);

    div()
        .id("nahual-theme-switcher")
        .px(px(8.))
        .py(px(4.))
        .bg(theme.bg_panel_alt.clone())
        .text_color(theme.fg_text)
        .text_size(px(11.))
        .rounded(px(3.))
        .hover(move |d| d.bg(theme.bg_row_hover))
        .child(SharedString::from(label))
        .on_click(|_event: &ClickEvent, _window: &mut Window, cx: &mut App| {
            let current_name = Theme::global(cx).name;
            let next = Theme::next_after(current_name);
            Theme::set(cx, next);
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn switcher_constructs_with_theme_installed(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Theme::install_default(cx);
            let _div = theme_switcher(cx);
            // Smoke: si llegamos aquí sin panic, el constructor lee
            // el global, deriva colors, y construye un Div.
        });
    }

    #[gpui::test]
    fn theme_set_changes_global(cx: &mut TestAppContext) {
        cx.update(|cx| {
            Theme::install_default(cx);
            let initial_name = Theme::global(cx).name;
            // Ciclo manual sin pasar por el handler del click.
            let next = Theme::next_after(initial_name);
            Theme::set(cx, next.clone());
            let after = Theme::global(cx).name;
            assert_eq!(after, next.name);
            assert_ne!(after, initial_name, "el ciclo debe cambiar el name");
        });
    }
}
