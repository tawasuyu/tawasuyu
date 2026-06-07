//! `llimphi-widget-button` — botón clicable con estado hover.
//!
//! Reusable entre apps Llimphi: `button_view(label, palette, on_click)`
//! devuelve una vista que cambia de color cuando el cursor pasa por
//! encima y emite `on_click` al ser apretada. El caller controla las
//! dimensiones envolviendo el `View` retornado en un contenedor flex
//! con el tamaño que necesite (botón ancho completo, chip 80×30, etc).
//!
//! No expone estado interno — todo el estado vive en el `Model` del App
//! (el hover lo trackea llimphi-ui automáticamente vía `hover_fill`).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Paleta del botón. Por default un chip dark con highlight tenue al
/// hover — similar al patrón `bg_panel_alt` + `bg_row_hover` de
/// `nahual-theme`.
#[derive(Debug, Clone, Copy)]
pub struct ButtonPalette {
    pub bg: Color,
    pub bg_hover: Color,
    pub fg: Color,
    pub radius: f64,
}

impl Default for ButtonPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl ButtonPalette {
    /// Construye la paleta desde un `Theme` semántico.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_button,
            bg_hover: t.bg_button_hover,
            fg: t.fg_text,
            radius: 5.0,
        }
    }
}

/// Compone un botón rectangular: bg + texto + on_click + hover. Por
/// default ocupa ancho 100% del padre y alto 30 px; sobre-escribir
/// pasando un `Style` propio vía [`button_styled`].
pub fn button_view<Msg: Clone + 'static>(
    label: impl Into<String>,
    palette: &ButtonPalette,
    on_click: Msg,
) -> View<Msg> {
    button_styled(
        label,
        Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(30.0_f32),
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
        },
        Alignment::Center,
        palette,
        on_click,
    )
}

/// Tinte de la onda de ripple derivado de la paleta: el color de texto
/// (`fg`, normalmente claro sobre el botón dark) a alpha bajo, así contrasta
/// con el fondo y se adapta al theme sin añadir un campo a [`ButtonPalette`].
fn ripple_ink(palette: &ButtonPalette) -> Color {
    let c = palette.fg.components;
    Color { components: [c[0], c[1], c[2], 0.22], ..palette.fg }
}

/// Como [`button_view`] pero con feedback **ripple/InkWell**: al presionarlo
/// emite la salpicadura Material (círculo que se expande desde el punto del
/// tap y se desvanece, recortado al contorno del botón). `key` debe ser
/// **estable y único** entre los botones vivos del frame (índice del botón,
/// hash de su acción) — es lo que enlaza la onda retenida con este botón entre
/// frames. El tinte sale de la paleta ([`ripple_ink`]).
pub fn button_ripple<Msg: Clone + 'static>(
    label: impl Into<String>,
    key: u64,
    palette: &ButtonPalette,
    on_click: Msg,
) -> View<Msg> {
    button_view(label, palette, on_click).ripple(key, ripple_ink(palette))
}

/// Variante con `Style` y alineación de texto explícitos — útil cuando
/// la app necesita un botón con dimensiones particulares o el texto a
/// la izquierda.
pub fn button_styled<Msg: Clone + 'static>(
    label: impl Into<String>,
    style: Style,
    text_alignment: Alignment,
    palette: &ButtonPalette,
    on_click: Msg,
) -> View<Msg> {
    let label: String = label.into();
    // Semántica accesible: rol Button + el texto visible como nombre. Si el
    // caller le pasó un label vacío (botones puramente icónicos), igual sale
    // como Button — lo correcto es agregarle un aria_label propio desde fuera.
    let aria = label.clone();
    // Gloss superior: gradient blanco alpha 28 → 0 sobre la mitad de
    // arriba. `paint_with` corre entre el fill (que respeta hover_fill)
    // y el texto, así que la luz se suma al color de base sin sustituirlo
    // — el hover sigue funcionando idéntico. El RoundedRect cubre el
    // botón completo y `Extend::Pad` (default de peniko) deja la mitad
    // inferior en alpha 0. Match: chrome/splash — superficie con luz
    // descendente desde el edge superior.
    let radius = palette.radius;
    View::new(style)
        .fill(palette.bg)
        .hover_fill(palette.bg_hover)
        .radius(radius)
        .paint_with(move |scene, _ts, rect| {
            use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, RoundedRect};
            use llimphi_ui::llimphi_raster::peniko::{Fill, Gradient};
            if rect.w <= 0.0 || rect.h <= 0.0 {
                return;
            }
            let x0 = rect.x as f64;
            let y0 = rect.y as f64;
            let x1 = (rect.x + rect.w) as f64;
            let y1 = (rect.y + rect.h) as f64;
            let y_mid = y0 + (y1 - y0) * 0.5;
            let rr = RoundedRect::new(x0, y0, x1, y1, radius);
            let top = Color::from_rgba8(255, 255, 255, 28);
            let bot = Color::from_rgba8(255, 255, 255, 0);
            let gradient = Gradient::new_linear(Point::new(x0, y0), Point::new(x0, y_mid))
                .with_stops([top, bot].as_slice());
            scene.fill(Fill::NonZero, Affine::IDENTITY, &gradient, None, &rr);
        })
        .text_aligned(label, 13.0, palette.fg, text_alignment)
        .role(llimphi_ui::Role::Button)
        .aria_label(aria)
        .on_click(on_click)
        .cursor(llimphi_ui::Cursor::Pointer)
}

/// Como [`button_styled`] pero con feedback **ripple/InkWell** (ver
/// [`button_ripple`] para la semántica de `key`).
pub fn button_styled_ripple<Msg: Clone + 'static>(
    label: impl Into<String>,
    key: u64,
    style: Style,
    text_alignment: Alignment,
    palette: &ButtonPalette,
    on_click: Msg,
) -> View<Msg> {
    button_styled(label, style, text_alignment, palette, on_click)
        .ripple(key, ripple_ink(palette))
}
