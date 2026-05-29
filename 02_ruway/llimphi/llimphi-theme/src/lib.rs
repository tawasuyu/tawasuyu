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

use std::time::Duration;

// =====================================================================
// Tokens transversales — motion, alpha, radius
// =====================================================================
//
// Los widgets de elegancia (tooltip, toast, modal, spinner, splash, …)
// comparten **duraciones**, **alphas** y **radios** para que el sistema
// se sienta uno solo. Cada token es `const`: las apps pueden referenciar
// `motion::NORMAL`/`alpha::SCRIM` directamente, o tomarlos del `Theme`
// vía `theme.motion()` / `theme.alpha()` / `theme.radius()` cuando una
// future variante por preset lo requiera.

/// Duraciones canónicas (segundo nivel: rítmico, no nervioso, no
/// soporífero). Los widgets eligen `FAST` para microinteracciones
/// (hover, focus), `NORMAL` para transiciones principales (toast entrar,
/// modal abrir) y `SLOW` para énfasis o entradas dramáticas (splash de
/// boot).
pub mod motion {
    use super::Duration;

    pub const FAST: Duration = Duration::from_millis(80);
    pub const NORMAL: Duration = Duration::from_millis(160);
    pub const SLOW: Duration = Duration::from_millis(320);

    /// Easing estándar — cubic-out. Energía inicial, asentamiento suave.
    /// La gran mayoría de transiciones de salida / aparición.
    #[inline]
    pub fn ease_out_cubic(t: f32) -> f32 {
        let inv = 1.0 - t.clamp(0.0, 1.0);
        1.0 - inv * inv * inv
    }

    /// Easing énfasis — cubic-in-out. Para movimientos que cruzan la
    /// pantalla y necesitan acentuar el centro (modales, splashes).
    #[inline]
    pub fn ease_in_out_cubic(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t < 0.5 {
            4.0 * t * t * t
        } else {
            let f = -2.0 * t + 2.0;
            1.0 - f * f * f / 2.0
        }
    }

    /// Lineal — no es elegante pero a veces es lo correcto (barra de
    /// progreso, valores numéricos crudos).
    #[inline]
    pub fn linear(t: f32) -> f32 {
        t.clamp(0.0, 1.0)
    }
}

/// Valores de opacidad alfa (0–255) para capas semánticas. Usar siempre
/// que se quiera *transparencia coherente*. El widget que improvisa su
/// propio alpha rompe la firma visual.
pub mod alpha {
    /// Scrim que cubre la app cuando hay overlay (menú/modal/picker).
    /// Apaga el fondo lo justo para que el overlay tenga jerarquía,
    /// sin ocultar contexto.
    pub const SCRIM: u8 = 64;

    /// Tinte aplicado a un panel "vidrio" sobre fondo activo (tooltip,
    /// status hint). Casi opaco pero deja respirar.
    pub const GLASS_PANEL: u8 = 232;

    /// Elementos deshabilitados — visibles pero con menos peso.
    pub const DISABLED: u8 = 140;

    /// Hint sutil (text watermark, ghost) — apenas legible.
    pub const HINT: u8 = 96;
}

/// Radios de esquina canónicos. La elegancia se construye en escalera:
/// `XS` para chips e inputs, `SM` para botones, `MD` para paneles,
/// `LG` para superficies grandes (toast, modal, card destacada).
pub mod radius {
    pub const XS: f64 = 2.0;
    pub const SM: f64 = 4.0;
    pub const MD: f64 = 8.0;
    pub const LG: f64 = 12.0;
    pub const XL: f64 = 20.0;
}

/// Paleta de la app. Slots semánticos que cubren los casos comunes
/// (fondo, texto, hover, foco, acento). Los widgets reusables toman su
/// `Palette` específico desde acá vía `Palette::from_theme(&theme)`.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Nombre legible del preset — alimenta `Theme::by_name`,
    /// `next_after`, y los UIs que ciclan presets (theme-switcher).
    pub name: &'static str,

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
            name: "Dark",
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
            name: "Light",
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

    /// Tema "Aurora" — verdes nocturnos con acento aqua. Análogo al
    /// preset del nahual-theme.
    pub const fn aurora() -> Self {
        Self {
            name: "Aurora",
            bg_app: Color::from_rgba8(8, 18, 22, 255),
            bg_panel: Color::from_rgba8(14, 28, 34, 255),
            bg_panel_alt: Color::from_rgba8(12, 24, 30, 255),
            bg_input: Color::from_rgba8(10, 22, 28, 255),
            bg_input_focus: Color::from_rgba8(14, 30, 38, 255),
            bg_button: Color::from_rgba8(20, 44, 52, 255),
            bg_button_hover: Color::from_rgba8(30, 66, 78, 255),
            bg_selected: Color::from_rgba8(30, 90, 100, 255),
            bg_row_hover: Color::from_rgba8(20, 46, 56, 255),
            fg_text: Color::from_rgba8(214, 232, 232, 255),
            fg_muted: Color::from_rgba8(130, 168, 168, 255),
            fg_placeholder: Color::from_rgba8(90, 120, 120, 255),
            fg_destructive: Color::from_rgba8(220, 110, 110, 255),
            border: Color::from_rgba8(38, 70, 78, 255),
            border_focus: Color::from_rgba8(80, 200, 200, 255),
            accent: Color::from_rgba8(80, 200, 200, 255),
        }
    }

    /// Tema "Sunset" — cálidos con acento naranja, sobre base oscura.
    pub const fn sunset() -> Self {
        Self {
            name: "Sunset",
            bg_app: Color::from_rgba8(22, 14, 14, 255),
            bg_panel: Color::from_rgba8(34, 22, 22, 255),
            bg_panel_alt: Color::from_rgba8(28, 18, 18, 255),
            bg_input: Color::from_rgba8(28, 18, 18, 255),
            bg_input_focus: Color::from_rgba8(36, 24, 22, 255),
            bg_button: Color::from_rgba8(54, 34, 28, 255),
            bg_button_hover: Color::from_rgba8(78, 50, 38, 255),
            bg_selected: Color::from_rgba8(120, 64, 38, 255),
            bg_row_hover: Color::from_rgba8(56, 36, 28, 255),
            fg_text: Color::from_rgba8(238, 220, 200, 255),
            fg_muted: Color::from_rgba8(174, 142, 120, 255),
            fg_placeholder: Color::from_rgba8(120, 96, 80, 255),
            fg_destructive: Color::from_rgba8(220, 100, 100, 255),
            border: Color::from_rgba8(70, 46, 36, 255),
            border_focus: Color::from_rgba8(232, 140, 70, 255),
            accent: Color::from_rgba8(232, 140, 70, 255),
        }
    }

    /// Todos los presets del repo, en el orden canónico de rotación
    /// (Dark → Light → Aurora → Sunset → Dark…). El theme-switcher
    /// los consume vía [`Theme::next_after`].
    pub fn all() -> Vec<Self> {
        vec![Self::dark(), Self::light(), Self::aurora(), Self::sunset()]
    }

    /// Busca un preset por nombre exacto.
    pub fn by_name(name: &str) -> Option<Self> {
        Self::all().into_iter().find(|t| t.name == name)
    }

    /// Próximo preset en la rotación de [`Theme::all`]. Si `current` no
    /// se encuentra, retorna el primero — el switcher nunca se traba.
    pub fn next_after(current: &str) -> Self {
        let all = Self::all();
        let idx = all
            .iter()
            .position(|t| t.name == current)
            .map(|i| (i + 1) % all.len())
            .unwrap_or(0);
        all[idx]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_unique_names() {
        let all = Theme::all();
        let mut names: Vec<&str> = all.iter().map(|t| t.name).collect();
        let n_before = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), n_before, "nombres duplicados en Theme::all()");
    }

    #[test]
    fn by_name_finds_each_preset() {
        for t in Theme::all() {
            let by = Theme::by_name(t.name).expect("preset registrado");
            assert_eq!(by.name, t.name);
        }
    }

    #[test]
    fn by_name_returns_none_for_unknown() {
        assert!(Theme::by_name("ThisDoesNotExist").is_none());
    }

    #[test]
    fn next_after_cycles_through_all_presets() {
        let all = Theme::all();
        let mut current = all[0].name;
        let mut visited = vec![current];
        for _ in 0..all.len() - 1 {
            current = Theme::next_after(current).name;
            visited.push(current);
        }
        let names: Vec<&str> = all.iter().map(|t| t.name).collect();
        assert_eq!(visited, names);
        // El siguiente debe volver al primero.
        let wrapped = Theme::next_after(current).name;
        assert_eq!(wrapped, all[0].name);
    }

    #[test]
    fn next_after_unknown_falls_back_to_first() {
        let n = Theme::next_after("Nope").name;
        assert_eq!(n, Theme::all()[0].name);
    }

    #[test]
    fn dark_is_the_default() {
        assert_eq!(Theme::default().name, "Dark");
    }
}
