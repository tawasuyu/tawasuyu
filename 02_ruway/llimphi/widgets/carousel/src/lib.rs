//! `llimphi-widget-carousel` — pager paginado.
//!
//! Una vista que muestra N páginas, una a la vez, con **dots indicadores**
//! abajo (clickeables para saltar a la página i) y **flechas opcionales**
//! a los costados. El caller mantiene un único `current_index: usize` en
//! su modelo y recibe `on_change(i)` cuando el usuario cambia de página.
//!
//! v1 **sin swipe** — la navegación va por dots y por flechas. Una v2
//! puede agregar swipe horizontal usando `View::draggable_velocity` +
//! `fling_step` para snap-on-release (el seam ya existe; sumarlo es
//! composición).
//!
//! Helpers puros para wrap/clamp del índice ([`wrap_index`],
//! [`clamp_index`]) — útiles para la lógica del `update` del caller (las
//! flechas en los extremos pueden envolver o quedarse).

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Position, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_theme::Theme;

const DOT_SIZE: f32 = 8.0;
const DOT_GAP: f32 = 8.0;
const DOT_ROW_H: f32 = 28.0;
const ARROW_W: f32 = 36.0;

/// Paleta del carousel.
#[derive(Debug, Clone, Copy)]
pub struct CarouselPalette {
    /// Fondo del dot inactivo.
    pub dot_idle: Color,
    /// Fondo del dot activo (página actual).
    pub dot_active: Color,
    /// Fondo del dot al hover.
    pub dot_hover: Color,
    /// Color del glifo de la flecha (‹ / ›).
    pub arrow_fg: Color,
    /// Fondo de la flecha al hover.
    pub arrow_hover_bg: Color,
}

impl CarouselPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            dot_idle: t.border,
            dot_active: t.accent,
            dot_hover: t.fg_muted,
            arrow_fg: t.fg_muted,
            arrow_hover_bg: t.bg_row_hover,
        }
    }
}

impl Default for CarouselPalette {
    fn default() -> Self {
        Self::from_theme(&Theme::dark())
    }
}

/// Estrategia para los extremos cuando el usuario aprieta `‹` en la
/// primera página o `›` en la última.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarouselWrap {
    /// La página anterior a la 0 es la última; la posterior a la última
    /// es la 0. Comportamiento "infinite carousel".
    Wrap,
    /// `‹` en la página 0 y `›` en la última no hacen nada (el callback
    /// igual recibe `i = current`, idempotente).
    Clamp,
}

impl Default for CarouselWrap {
    fn default() -> Self {
        Self::Clamp
    }
}

/// Índice resultante de moverse `delta` páginas desde `current` con
/// wrap. `total = 0` devuelve `0` (no hay páginas).
pub fn wrap_index(current: usize, total: usize, delta: i32) -> usize {
    if total == 0 {
        return 0;
    }
    let total_i = total as i32;
    let raw = current as i32 + delta;
    raw.rem_euclid(total_i) as usize
}

/// Índice resultante de moverse `delta` páginas desde `current` con
/// clamp. `total = 0` devuelve `0`.
pub fn clamp_index(current: usize, total: usize, delta: i32) -> usize {
    if total == 0 {
        return 0;
    }
    let raw = current as i32 + delta;
    raw.clamp(0, total as i32 - 1) as usize
}

/// Avanza/retrocede según la estrategia.
pub fn navigate(current: usize, total: usize, delta: i32, wrap: CarouselWrap) -> usize {
    match wrap {
        CarouselWrap::Wrap => wrap_index(current, total, delta),
        CarouselWrap::Clamp => clamp_index(current, total, delta),
    }
}

/// Especificación del carousel.
pub struct CarouselSpec<Msg> {
    /// Páginas a mostrar. Se renderiza sólo la página `current`.
    pub pages: Vec<View<Msg>>,
    /// Índice de la página visible. Se acota a `pages.len() - 1`.
    pub current: usize,
    /// Estrategia para los extremos.
    pub wrap: CarouselWrap,
    /// Si `true`, muestra flechas `‹` / `›` superpuestas a los lados.
    pub show_arrows: bool,
    pub palette: CarouselPalette,
    /// Disparado al hacer click en un dot o una flecha — recibe el nuevo
    /// índice (`0..pages.len()`).
    pub on_change: Arc<dyn Fn(usize) -> Msg + Send + Sync>,
}

/// Vista del carousel. Devuelve un `View<Msg>` con el slot que el
/// caller le asigne (página + dots abajo + flechas opcionales). Si
/// `pages` está vacía devuelve un `View` vacío del mismo slot.
pub fn carousel_view<Msg>(spec: CarouselSpec<Msg>) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let CarouselSpec {
        mut pages,
        current,
        wrap,
        show_arrows,
        palette,
        on_change,
    } = spec;

    let total = pages.len();
    if total == 0 {
        return View::<Msg>::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        });
    }
    let cur = current.min(total - 1);

    // Página visible — drenamos el vector para no clonar.
    let page = std::mem::replace(
        &mut pages[cur],
        View::<Msg>::new(Style::default()),
    );

    // Wrapper de la página: 100% × resto (todo menos la barra de dots).
    let page_layer = View::<Msg>::new(Style {
        position: Position::Relative,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![page]);

    // Flechas opcionales superpuestas a los lados.
    let mut page_children: Vec<View<Msg>> = vec![page_layer];
    if show_arrows && total > 1 {
        let on_prev = on_change.clone();
        let prev_idx = navigate(cur, total, -1, wrap);
        let prev = View::<Msg>::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
                right: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
            },
            size: Size {
                width: length(ARROW_W),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text("‹", 22.0, palette.arrow_fg)
        .hover_fill(palette.arrow_hover_bg)
        .cursor(llimphi_ui::Cursor::Pointer)
        .on_click_at(move |_, _, _, _| Some((on_prev)(prev_idx)));

        let on_next = on_change.clone();
        let next_idx = navigate(cur, total, 1, wrap);
        let next = View::<Msg>::new(Style {
            position: Position::Absolute,
            inset: Rect {
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
                left: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
            },
            size: Size {
                width: length(ARROW_W),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text("›", 22.0, palette.arrow_fg)
        .hover_fill(palette.arrow_hover_bg)
        .cursor(llimphi_ui::Cursor::Pointer)
        .on_click_at(move |_, _, _, _| Some((on_next)(next_idx)));

        page_children.push(prev);
        page_children.push(next);
    }
    let page_area = View::<Msg>::new(Style {
        position: Position::Relative,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(page_children);

    // Fila de dots abajo.
    let mut dots: Vec<View<Msg>> = Vec::with_capacity(total);
    for i in 0..total {
        let f = on_change.clone();
        let is_active = i == cur;
        let mut dot = View::<Msg>::new(Style {
            size: Size { width: length(DOT_SIZE), height: length(DOT_SIZE) },
            ..Default::default()
        })
        .radius((DOT_SIZE * 0.5) as f64)
        .cursor(llimphi_ui::Cursor::Pointer);
        if is_active {
            dot = dot.fill(palette.dot_active);
        } else {
            dot = dot.fill(palette.dot_idle).hover_fill(palette.dot_hover);
        }
        dot = dot.on_click_at(move |_, _, _, _| Some((f)(i)));
        dots.push(dot);
    }
    let dot_row = View::<Msg>::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: llimphi_ui::llimphi_layout::taffy::Size {
            width: length(DOT_GAP),
            height: length(0.0_f32),
        },
        size: Size { width: percent(1.0_f32), height: length(DOT_ROW_H) },
        ..Default::default()
    })
    .children(dots);

    View::<Msg>::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![page_area, dot_row])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_index_envuelve_en_los_extremos() {
        assert_eq!(wrap_index(0, 5, -1), 4);
        assert_eq!(wrap_index(4, 5, 1), 0);
        assert_eq!(wrap_index(2, 5, 0), 2);
        // Total grande: no overflow.
        assert_eq!(wrap_index(2, 5, 12), 4); // 2+12=14, %5=4
        assert_eq!(wrap_index(0, 5, -7), 3); // -7 rem_euclid 5 = 3
        // Total 0: defensa.
        assert_eq!(wrap_index(0, 0, 1), 0);
    }

    #[test]
    fn clamp_index_se_queda_en_los_extremos() {
        assert_eq!(clamp_index(0, 5, -1), 0);
        assert_eq!(clamp_index(4, 5, 1), 4);
        assert_eq!(clamp_index(2, 5, 1), 3);
        assert_eq!(clamp_index(2, 5, 99), 4);
        assert_eq!(clamp_index(2, 5, -99), 0);
        // Total 0: defensa.
        assert_eq!(clamp_index(0, 0, 1), 0);
    }

    #[test]
    fn navigate_aplica_estrategia() {
        assert_eq!(navigate(0, 5, -1, CarouselWrap::Wrap), 4);
        assert_eq!(navigate(0, 5, -1, CarouselWrap::Clamp), 0);
        assert_eq!(navigate(4, 5, 1, CarouselWrap::Wrap), 0);
        assert_eq!(navigate(4, 5, 1, CarouselWrap::Clamp), 4);
    }
}
