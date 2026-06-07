//! `llimphi-widget-chip` — chip compacto con cuatro sabores: **filter**
//! (toggle binario), **choice** (radio dentro de un grupo), **input**
//! (etiqueta removible con × al final) y **assist** (chip-acción con
//! ícono opcional).
//!
//! Forma: rectángulo redondeado bien pegado (radius pill), padding
//! horizontal 10/4, alto 24 px. Tema-consciente: cuando `selected`,
//! pinta `accent` apenas tinted (alpha bajo) — sobrio, no chillón.
//!
//! Como toda la elegancia de Llimphi, hereda de `llimphi-theme::motion`
//! para la transición de fill al togglear (animación implícita via
//! `View::animated(key, motion::MICRO)`).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::{motion, radius, Theme};

/// Paleta del chip — tres slots: idle, selected, fg.
#[derive(Debug, Clone, Copy)]
pub struct ChipPalette {
    pub bg_idle: Color,
    pub bg_selected: Color,
    pub fg_idle: Color,
    pub fg_selected: Color,
    pub border: Color,
}

impl ChipPalette {
    pub fn from_theme(t: &Theme) -> Self {
        // El selected es el accent atenuado hacia el fondo —"tinte", no
        // bloque sólido—. Calculado en sRGB-ish: `accent · 0.32 + bg · 0.68`.
        let a = t.accent.components;
        let b = t.bg_panel.components;
        let mix = Color {
            components: [
                a[0] * 0.32 + b[0] * 0.68,
                a[1] * 0.32 + b[1] * 0.68,
                a[2] * 0.32 + b[2] * 0.68,
                1.0,
            ],
            ..t.accent
        };
        Self {
            bg_idle: t.bg_button,
            bg_selected: mix,
            fg_idle: t.fg_text,
            fg_selected: t.fg_text,
            border: t.border,
        }
    }
}

/// Sabor del chip — decide cómo se interpreta el `selected` y si lleva
/// botón × removible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipKind {
    /// Toggle binario (selected ↔ no). Usado en `FilterChip` de Material.
    Filter,
    /// Radio dentro de un grupo (selected = exclusive). Visualmente igual
    /// a Filter; la diferencia está en el caller que mantiene 1 seleccionado.
    Choice,
    /// Etiqueta con × final que dispara `on_remove`. Para tags, multi-select
    /// presentado como contenedor de chips.
    Input,
    /// Acción rápida (no togglea); selected siempre falso.
    Assist,
}

/// Chip base — devuelve el view con anim implícita de fill por estado.
/// `key` debe ser estable entre frames del mismo chip (idx + grupo).
pub fn chip_view<Msg: Clone + 'static>(
    label: impl Into<String>,
    kind: ChipKind,
    selected: bool,
    key: u64,
    palette: &ChipPalette,
    on_click: Msg,
    on_remove: Option<Msg>,
) -> View<Msg> {
    let bg = if selected { palette.bg_selected } else { palette.bg_idle };
    let fg = if selected { palette.fg_selected } else { palette.fg_idle };

    let mut children = vec![View::new(Style {
        size: Size { width: auto(), height: auto() },
        ..Default::default()
    })
    .text_aligned(label.into(), 12.0, fg, Alignment::Center)];

    if let (ChipKind::Input, Some(rm)) = (kind, on_remove) {
        // ×: glifo pequeño a la derecha con on_click propio. No es
        // botón nested (View no soporta nested on_click), así que es
        // un nodo hermano con on_click separado dentro del mismo chip
        // — el árbitro de hit-test elige el más interno.
        children.push(
            View::new(Style {
                size: Size { width: length(14.0_f32), height: length(14.0_f32) },
                margin: Rect {
                    left: length(6.0_f32),
                    right: length(0.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .radius(7.0)
            .hover_fill(palette.border)
            .text_aligned("×".to_string(), 12.0, fg, Alignment::Center)
            .on_click(rm)
            .cursor(llimphi_ui::Cursor::Pointer),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size { width: auto(), height: length(24.0_f32) },
        ..Default::default()
    })
    .fill(bg)
    .radius(radius::XL)
    .border(1.0, palette.border)
    .animated(key, motion::MICRO)
    .children(children)
    .on_click(on_click)
    .cursor(llimphi_ui::Cursor::Pointer)
}

/// Atajo: chip filter (toggle).
pub fn filter_chip<Msg: Clone + 'static>(
    label: impl Into<String>,
    selected: bool,
    key: u64,
    palette: &ChipPalette,
    on_toggle: Msg,
) -> View<Msg> {
    chip_view(label, ChipKind::Filter, selected, key, palette, on_toggle, None)
}

/// Atajo: chip input (removible).
pub fn input_chip<Msg: Clone + 'static>(
    label: impl Into<String>,
    key: u64,
    palette: &ChipPalette,
    on_click: Msg,
    on_remove: Msg,
) -> View<Msg> {
    chip_view(
        label,
        ChipKind::Input,
        false,
        key,
        palette,
        on_click,
        Some(on_remove),
    )
}

/// Atajo: chip assist (acción).
pub fn assist_chip<Msg: Clone + 'static>(
    label: impl Into<String>,
    key: u64,
    palette: &ChipPalette,
    on_click: Msg,
) -> View<Msg> {
    chip_view(label, ChipKind::Assist, false, key, palette, on_click, None)
}
