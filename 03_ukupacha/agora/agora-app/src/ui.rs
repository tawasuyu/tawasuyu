//! Helpers de layout, paletas y utilidades de hex compartidas por los
//! tiles, más las pantallas transversales (banner de estado, unlock).

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

use crate::model::{Msg, Screen, StatusBanner, StatusLevel};

// =============================================================================
//  Layout genérico
// =============================================================================

pub(crate) fn column<M: 'static>(children: Vec<View<M>>) -> View<M> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: edge_padding(10.0, 6.0),
        gap: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

pub(crate) fn grow<M: 'static>(v: View<M>) -> View<M> {
    let mut v = v;
    v.style.flex_grow = 1.0;
    v.style.flex_basis = length(0.0_f32);
    v.style.min_size = Size {
        width: length(0.0_f32),
        height: length(0.0_f32),
    };
    v
}

pub(crate) fn empty<M: 'static>() -> View<M> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
}

pub(crate) fn spacer<M: 'static>(h: f32) -> View<M> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
}

pub(crate) fn label_line<M: 'static>(text: &str, size: f32, color: Color) -> View<M> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(size + 8.0),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text.to_string(), size, color, Alignment::Start)
}

pub(crate) fn edge_padding(
    h: f32,
    v: f32,
) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(h),
        right: length(h),
        top: length(v),
        bottom: length(v),
    }
}

/// Input de texto de alto fijo (32 px), envuelto para encajar en una columna.
pub(crate) fn input_row(
    state: &TextInputState,
    placeholder: &str,
    focused: bool,
    palette: &TextInputPalette,
    on_focus: Msg,
) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(32.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![text_input_view(
        state,
        placeholder,
        focused,
        palette,
        on_focus,
    )])
}

/// Fila horizontal de alto fijo con gap de 6 px entre hijos. Para colocar
/// dos botones lado a lado (exportar / limpiar, etc.).
pub(crate) fn row(h: f32, children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        flex_shrink: 0.0,
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(children)
}

/// Botón de ancho `frac` (fracción del contenedor) y alto fijo `h`.
pub(crate) fn boton_frac(
    label: impl Into<String>,
    frac: f32,
    h: f32,
    palette: &ButtonPalette,
    msg: Msg,
) -> View<Msg> {
    button_styled(
        label,
        Style {
            size: Size {
                width: percent(frac),
                height: length(h),
            },
            padding: edge_padding(10.0, 0.0),
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        palette,
        msg,
    )
}

// =============================================================================
//  Paletas
// =============================================================================

pub(crate) fn button_palette_primary(t: &Theme) -> ButtonPalette {
    ButtonPalette {
        bg: t.accent,
        bg_hover: t.bg_button_hover,
        fg: t.bg_app,
        radius: 4.0,
    }
}

pub(crate) fn button_palette_secondary(t: &Theme) -> ButtonPalette {
    ButtonPalette {
        bg: t.bg_button,
        bg_hover: t.bg_button_hover,
        fg: t.fg_text,
        radius: 4.0,
    }
}

// =============================================================================
//  Hex
// =============================================================================

pub(crate) fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Parsea un hash de 32 bytes desde su forma hex (64 dígitos, tolera espacios
/// y mayúsculas). `None` si la longitud no es exacta o hay un dígito inválido.
pub(crate) fn parse_hash32(s: &str) -> Option<[u8; 32]> {
    let limpio: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if limpio.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&limpio[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Decodifica una cadena hex de longitud par en bytes. `None` ante longitud
/// impar o dígito inválido. Usado para verificar postcard pegado.
pub(crate) fn hex_to_bytes(s: &str) -> Option<Vec<u8>> {
    let limpio: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if limpio.len() % 2 != 0 {
        return None;
    }
    (0..limpio.len() / 2)
        .map(|i| u8::from_str_radix(&limpio[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

// =============================================================================
//  Banner de estado
// =============================================================================

/// Compone el tiled view con un banner al pie (34 px fijos).
pub(crate) fn status_layout(theme: &Theme, tiled: View<Msg>, banner: &StatusBanner) -> View<Msg> {
    let (bg, fg) = match banner.level {
        StatusLevel::Info => (theme.bg_panel, theme.fg_text),
        StatusLevel::Error => (theme.fg_destructive, theme.bg_app),
    };

    let texto = View::new(Style {
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: edge_padding(12.0, 0.0),
        ..Default::default()
    })
    .text_aligned(banner.text.clone(), 12.0, fg, Alignment::Start);

    let cerrar = button_styled(
        "×",
        Style {
            size: Size {
                width: length(34.0_f32),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &ButtonPalette {
            bg,
            bg_hover: theme.bg_button_hover,
            fg,
            radius: 0.0,
        },
        Msg::DescartarStatus,
    );

    let banner_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .children(vec![texto, cerrar]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![grow(tiled), banner_row])
}

// =============================================================================
//  Pantalla: Unlock
// =============================================================================

pub(crate) fn unlock_view(screen: &Screen, theme: &Theme) -> View<Msg> {
    let (input, status_text) = match screen {
        Screen::Unlock { input, status } => (input, status.as_str()),
        Screen::Main => unreachable!("unlock_view sólo se llama con Screen::Unlock"),
    };
    let input_palette = TextInputPalette::from_theme(theme);

    let titulo = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        "ágora · desbloqueo".to_string(),
        24.0,
        theme.accent,
        Alignment::Center,
    );

    let hint = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(
        "passphrase del keystore (Enter desbloquea)".to_string(),
        12.0,
        theme.fg_muted,
        Alignment::Center,
    );

    let input_view = View::new(Style {
        size: Size {
            width: length(360.0_f32),
            height: length(36.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![text_input_view(
        input,
        "•••",
        true,
        &input_palette,
        Msg::UnlockSubmit, // click en el input no cambia foco — sólo hay uno
    )]);

    let status = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(
        status_text.to_string(),
        12.0,
        theme.fg_destructive,
        Alignment::Center,
    );

    let boton = button_styled(
        "desbloquear",
        Style {
            size: Size {
                width: length(360.0_f32),
                height: length(34.0_f32),
            },
            padding: edge_padding(10.0, 0.0),
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        },
        Alignment::Center,
        &button_palette_primary(theme),
        Msg::UnlockSubmit,
    );

    let card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(420.0_f32),
            height: length(260.0_f32),
        },
        padding: Rect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(24.0_f32),
            bottom: length(20.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(8.0)
    .children(vec![titulo, hint, spacer(8.0), input_view, boton, status]);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![card])
}
