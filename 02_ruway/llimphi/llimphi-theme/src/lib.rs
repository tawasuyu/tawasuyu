//! `llimphi-theme` — paleta compartida entre apps Llimphi.
//!
//! Define un set de slots semánticos (`bg_app`, `fg_text`, `accent`, etc.)
//! que cada widget mapea a su propio `Palette` específico vía
//! `Palette::from_theme(&theme)`. El analógo Llimphi al `nahual-theme`
//! GPUI, pero con colores `peniko::Color` y sin macros de Background /
//! gradiente — Llimphi pinta colores sólidos por ahora.
//!
//! Disponer del Theme en un crate aparte permite:
//! 1. **Consistencia visual**: las apps comparten paleta sin redefinirla.
//! 2. **Temas intercambiables**: `Theme::dark()` vs `Theme::light()` (o
//!    más adelante, sobreescritos por config del usuario).
//! 3. **Widgets desacoplados**: cada widget acepta su `Palette` (no el
//!    Theme entero), así un consumidor que sólo necesita un botón con
//!    colores no-temáticos puede construir su `ButtonPalette` a mano.

#![forbid(unsafe_code)]

pub use llimphi_raster::peniko::Color;

/// Paleta de la app. Slots semánticos que cubren los casos comunes
/// (fondo, texto, hover, foco, acento). Los widgets reusables toman su
/// `Palette` específico desde acá vía `Palette::from_theme(&theme)`.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    // --- Fondos ---
    /// Fondo de la ventana / superficie raíz.
    pub bg_app: Color,
    /// Fondo de paneles (sidebars, cards).
    pub bg_panel: Color,
    /// Fondo alternativo para barras / strips (tab bar, status bar).
    pub bg_panel_alt: Color,
    /// Fondo de campos de input (texto editable).
    pub bg_input: Color,
    /// Fondo de input cuando tiene foco.
    pub bg_input_focus: Color,
    /// Fondo de botón (chip).
    pub bg_button: Color,
    /// Fondo de botón al hover.
    pub bg_button_hover: Color,
    /// Fondo de la fila/item seleccionado (lista, tree).
    pub bg_selected: Color,
    /// Fondo de fila al hover (sin selección).
    pub bg_row_hover: Color,

    // --- Foregrounds (texto) ---
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_placeholder: Color,
    pub fg_destructive: Color,

    // --- Bordes y acento ---
    pub border: Color,
    pub border_focus: Color,
    /// Acento primario — divisores activos, borde de input focado,
    /// underline del tab activo, etc. Tono único de la app.
    pub accent: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// Tema oscuro — el default. Análogo al `nahual-theme` dark en su
    /// versión Llimphi: tonos azulados profundos, acento azul claro.
    pub const fn dark() -> Self {
        Self {
            bg_app: Color::from_rgba8(14, 16, 22, 255),
            bg_panel: Color::from_rgba8(22, 26, 36, 255),
            bg_panel_alt: Color::from_rgba8(18, 22, 30, 255),
            bg_input: Color::from_rgba8(16, 20, 28, 255),
            bg_input_focus: Color::from_rgba8(20, 26, 38, 255),
            bg_button: Color::from_rgba8(36, 42, 56, 255),
            bg_button_hover: Color::from_rgba8(54, 64, 86, 255),
            bg_selected: Color::from_rgba8(58, 78, 128, 255),
            bg_row_hover: Color::from_rgba8(36, 44, 60, 255),
            fg_text: Color::from_rgba8(214, 222, 232, 255),
            fg_muted: Color::from_rgba8(140, 152, 170, 255),
            fg_placeholder: Color::from_rgba8(95, 105, 122, 255),
            fg_destructive: Color::from_rgba8(220, 110, 110, 255),
            border: Color::from_rgba8(46, 54, 70, 255),
            border_focus: Color::from_rgba8(110, 140, 220, 255),
            accent: Color::from_rgba8(110, 140, 220, 255),
        }
    }

    /// Tema claro — pendiente de pulir contraste cuando llegue una app
    /// que lo pida. Calculado por inversión parcial del dark.
    pub const fn light() -> Self {
        Self {
            bg_app: Color::from_rgba8(244, 246, 250, 255),
            bg_panel: Color::from_rgba8(232, 236, 242, 255),
            bg_panel_alt: Color::from_rgba8(224, 230, 240, 255),
            bg_input: Color::from_rgba8(255, 255, 255, 255),
            bg_input_focus: Color::from_rgba8(250, 252, 255, 255),
            bg_button: Color::from_rgba8(220, 226, 236, 255),
            bg_button_hover: Color::from_rgba8(200, 210, 226, 255),
            bg_selected: Color::from_rgba8(160, 180, 220, 255),
            bg_row_hover: Color::from_rgba8(214, 222, 236, 255),
            fg_text: Color::from_rgba8(28, 36, 50, 255),
            fg_muted: Color::from_rgba8(100, 112, 130, 255),
            fg_placeholder: Color::from_rgba8(150, 160, 178, 255),
            fg_destructive: Color::from_rgba8(180, 60, 60, 255),
            border: Color::from_rgba8(196, 204, 218, 255),
            border_focus: Color::from_rgba8(60, 100, 200, 255),
            accent: Color::from_rgba8(60, 100, 200, 255),
        }
    }
}
