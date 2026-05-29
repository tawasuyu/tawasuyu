//! Widget cualquier-cosa que dice un texto fijo. Cubre `kind` desconocidos
//! para que un TOML con typos siga arrancando la barra.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, AlignItems, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::View;

use crate::widget::{Msg, Widget};

pub struct Placeholder {
    label: String,
}

impl Placeholder {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into() }
    }
}

impl Widget for Placeholder {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn view(&self, theme: &Theme) -> View<Msg> {
        View::new(Style {
            size: Size { width: auto(), height: length(22.0_f32) },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(self.label.clone(), 13.0, theme.fg_muted)
    }
}
