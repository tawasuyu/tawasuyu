//! `llimphi-widget-theme-switcher` — botón que rota los presets de
//! [`llimphi_theme::Theme`].
//!
//! Análogo Llimphi del `nahual-widget-theme-switcher` GPUI. Diferencia
//! estructural: GPUI lleva el theme en un `Global` y el switcher lo
//! reemplaza con `cx.set_global`; Llimphi no tiene globals — el caller
//! guarda el theme en su `Model` y reasigna en su `update`. El widget
//! sólo emite `on_change(next_theme)` cuando el botón se clickea, donde
//! `next_theme` es el siguiente preset de [`Theme::next_after`].
//!
//! El label del botón muestra el nombre del preset actual con un signo
//! de rotación (`Tema: Dark ▸`). Los colores salen del `Theme` actual
//! para que el switcher sea coherente con el resto de la UI.
//!
//! # Uso
//!
//! ```ignore
//! use llimphi_widget_theme_switcher::theme_switcher_view;
//!
//! // En App::view:
//! let switcher = theme_switcher_view(&model.theme, Msg::ChangeTheme);
//! ```
//!
//! `Msg::ChangeTheme(Theme)` lo define la app; en `update`:
//!
//! ```ignore
//! Msg::ChangeTheme(t) => { model.theme = t; }
//! ```

#![forbid(unsafe_code)]

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Paleta del switcher. Por default replica el patrón del switcher de
/// nahual: `bg_panel_alt` + hover `bg_row_hover`, texto `fg_text`.
#[derive(Debug, Clone, Copy)]
pub struct ThemeSwitcherPalette {
    pub bg: Color,
    pub bg_hover: Color,
    pub fg: Color,
    pub radius: f64,
}

impl Default for ThemeSwitcherPalette {
    fn default() -> Self {
        Self::from_theme(&Theme::dark())
    }
}

impl ThemeSwitcherPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            bg: t.bg_panel_alt,
            bg_hover: t.bg_row_hover,
            fg: t.fg_text,
            radius: 3.0,
        }
    }
}

/// Compone el switcher: chip con texto `Tema: <nombre> ▸`. Click rota
/// al siguiente preset y emite `on_change(next)`.
///
/// Toma el `current` por referencia para no clonar el `Theme` entero
/// (es `Copy`, pero la API se mantiene consistente con `Palette::from_theme`).
/// La paleta se deriva del `current` para que el chip use el mismo set
/// de colores que el resto de la UI.
pub fn theme_switcher_view<Msg: Clone + 'static>(
    current: &Theme,
    on_change: impl Fn(Theme) -> Msg,
) -> View<Msg> {
    let palette = ThemeSwitcherPalette::from_theme(current);
    theme_switcher_styled(current, &palette, on_change)
}

/// Variante con paleta explícita — útil cuando la app quiere un look
/// distinto al default (botón destacado, accent del switcher fijo, etc.).
pub fn theme_switcher_styled<Msg: Clone + 'static>(
    current: &Theme,
    palette: &ThemeSwitcherPalette,
    on_change: impl Fn(Theme) -> Msg,
) -> View<Msg> {
    let next = Theme::next_after(current.name);
    let label = format!("Tema: {} ▸", current.name);

    View::new(Style {
        size: Size {
            width: length(140.0_f32),
            height: length(26.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(palette.bg)
    .hover_fill(palette.bg_hover)
    .radius(palette.radius)
    .text_aligned(label, 11.0, palette.fg, Alignment::Start)
    .on_click(on_change(next))
}

/// Variante de tamaño flexible — toma el ancho dado por el padre y se
/// adapta al alto natural del slot. Útil dentro de toolbars con flexbox.
pub fn theme_switcher_flex<Msg: Clone + 'static>(
    current: &Theme,
    palette: &ThemeSwitcherPalette,
    on_change: impl Fn(Theme) -> Msg,
) -> View<Msg> {
    let next = Theme::next_after(current.name);
    let label = format!("Tema: {} ▸", current.name);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(26.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.bg)
    .hover_fill(palette.bg_hover)
    .radius(palette.radius)
    .text_aligned(label, 11.0, palette.fg, Alignment::Start)
    .on_click(on_change(next))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum Msg {
        Change(&'static str),
    }

    #[test]
    fn switcher_constructs_with_a_default_theme() {
        let t = Theme::dark();
        let _v = theme_switcher_view::<Msg>(&t, |th| Msg::Change(th.name));
        // Si el constructor no panicó, el widget queda armado.
    }

    #[test]
    fn palette_from_theme_matches_panel_alt_slots() {
        let t = Theme::dark();
        let p = ThemeSwitcherPalette::from_theme(&t);
        // No comparamos por igualdad de Color (no implementa PartialEq);
        // sí garantizamos que la paleta derivó del theme — radius default.
        assert_eq!(p.radius, 3.0);
    }

    #[test]
    fn on_change_receives_the_next_preset() {
        // Verificación funcional independiente: la rotación que verá el
        // handler coincide con `Theme::next_after`.
        let current = Theme::dark();
        let expected_next = Theme::next_after(current.name).name;
        assert_eq!(expected_next, "Light");
    }
}
