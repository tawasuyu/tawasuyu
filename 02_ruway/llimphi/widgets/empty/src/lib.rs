//! `llimphi-widget-empty` — empty state con icono, título y descripción.
//!
//! Patrón para reemplazar pantallas en blanco con orientación: cuando
//! una lista no tiene items, un editor no tiene archivo abierto, una
//! búsqueda no arrojó resultados — en vez de fondo plano, mostrar
//! un icono grande apagado + título + descripción corta + (opcional)
//! botón de acción primaria.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_icons::{icon_view, Icon};
use llimphi_theme::{alpha, Theme};

/// Paleta del empty state — colores apagados para no competir con la
/// UI principal.
#[derive(Debug, Clone, Copy)]
pub struct EmptyPalette {
    pub fg_icon: Color,
    pub fg_title: Color,
    pub fg_desc: Color,
}

impl EmptyPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            fg_icon: with_alpha8(t.fg_muted, alpha::HINT),
            fg_title: t.fg_muted,
            fg_desc: with_alpha8(t.fg_muted, alpha::DISABLED),
        }
    }
}

fn with_alpha8(c: Color, a: u8) -> Color {
    let [r, g, b, _] = c.components;
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    AlphaColor::new([r, g, b, a as f32 / 255.0])
}

/// Construye el empty state. La app llama desde su `view()` cuando
/// detecta el caso vacío:
///
/// ```ignore
/// if model.items.is_empty() {
///     return empty_view(Icon::File, "Sin archivos abiertos",
///                       Some("Abrí uno con Ctrl+O para empezar."),
///                       &palette);
/// }
/// ```
pub fn empty_view<Msg: Clone + 'static>(
    icon: Icon,
    title: impl Into<String>,
    description: Option<&str>,
    palette: &EmptyPalette,
) -> View<Msg> {
    let icon_cell = View::new(Style {
        size: Size {
            width: length(72.0_f32),
            height: length(72.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![icon_view(icon, palette.fg_icon, 1.4)]);

    let title_view = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(title.into(), 15.5, palette.fg_title, Alignment::Center);

    let mut children = vec![icon_cell, title_view];
    if let Some(desc) = description {
        children.push(
            View::new(Style {
                size: Size {
                    width: length(360.0_f32),
                    height: length(40.0_f32),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .text_aligned(desc.to_string(), 12.0, palette.fg_desc, Alignment::Center),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(0.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}
