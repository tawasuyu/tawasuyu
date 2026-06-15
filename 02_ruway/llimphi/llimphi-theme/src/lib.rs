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
// Color estable por semilla — avatares, etiquetas, hash-coloring
// =====================================================================

/// Paleta sobria de 8 tonos para colorear entidades por hash (avatares de
/// contactos, etiquetas de calendario…). Tonos apagados que conviven con
/// cualquier `Theme`. Usada vía [`stable_color`].
pub const ENTITY_PALETTE: [(u8, u8, u8); 8] = [
    (94, 129, 172),  // azul acero
    (163, 109, 156), // malva
    (122, 162, 110), // verde salvia
    (191, 138, 92),  // terracota
    (108, 153, 168), // celeste apagado
    (170, 120, 120), // rosa viejo
    (130, 140, 175), // lavanda
    (150, 150, 110), // oliva
];

/// Color estable derivado de una semilla: hash FNV-1a del texto → índice en
/// [`ENTITY_PALETTE`]. La misma semilla da siempre el mismo color, sin estado.
/// Para avatares (por correo), etiquetas, badges de entidad, etc.
pub fn stable_color(seed: &str) -> Color {
    let mut h: u32 = 2_166_136_261;
    for b in seed.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    let (r, g, b) = ENTITY_PALETTE[(h as usize) % ENTITY_PALETTE.len()];
    Color::from_rgba8(r, g, b, 255)
}

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
/// soporífero). Los widgets eligen `MICRO` para tintes de hover/focus
/// que sólo necesitan suavizar el "salto", `FAST` para microinteracciones
/// completas (chip que pulsa), `NORMAL` para transiciones principales
/// (toast entrar, modal abrir), `SLOW` para énfasis o entradas dramáticas
/// (splash de boot, hero shared-element).
pub mod motion {
    use super::Duration;

    /// Tintes hover/focus — apenas perceptible, sólo elimina el "clack".
    pub const MICRO: Duration = Duration::from_millis(50);
    pub const FAST: Duration = Duration::from_millis(80);
    pub const NORMAL: Duration = Duration::from_millis(160);
    pub const SLOW: Duration = Duration::from_millis(320);
    /// Entradas dramáticas (splash, hero shared-element).
    pub const DRAMATIC: Duration = Duration::from_millis(480);

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

    /// Easing fuerte — quint-out. Arranca más rápido que cubic-out y
    /// asienta más suave. Para elementos que aparecen "lanzados" (toast,
    /// FAB).
    #[inline]
    pub fn ease_out_quint(t: f32) -> f32 {
        let inv = 1.0 - t.clamp(0.0, 1.0);
        1.0 - inv * inv * inv * inv * inv
    }

    /// Overshoot suave — back-out con `c1=1.70158` (Material/Penner
    /// estándar). El valor pasa de 0 al objetivo, lo sobrepasa ~10 % y
    /// vuelve. Para entradas que necesitan "ping" (modal, snackbar,
    /// elemento nuevo en una lista). No usar para hover — la oscilación
    /// se percibe nerviosa.
    #[inline]
    pub fn ease_out_back(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        const C1: f32 = 1.701_58;
        const C3: f32 = C1 + 1.0;
        let u = t - 1.0;
        1.0 + C3 * u * u * u + C1 * u * u
    }

    /// Lineal — no es elegante pero a veces es lo correcto (barra de
    /// progreso, valores numéricos crudos).
    #[inline]
    pub fn linear(t: f32) -> f32 {
        t.clamp(0.0, 1.0)
    }
}

/// Tokens de **elevación** — sombras escalonadas. Como `Shadow` vive en
/// `llimphi-compositor` (y `llimphi-theme` no depende de él para
/// quedarse leaf), cada nivel se expone como `(alpha_u8, blur_px,
/// dy_px)`. Los widgets construyen su `Shadow` puenteándolo:
/// `Shadow { color: Color::from_rgba8(0,0,0, a), blur, dy, dx: 0.0, spread: 0.0 }`.
/// Escala perceptual logarítmica: cada nivel ~×2 de blur.
pub mod elevation {
    /// `(alpha 0–255, blur px, dy px)`. dy ≈ blur·0.4 (sombra natural,
    /// fuente de luz un poco arriba).
    pub type Elev = (u8, f64, f64);

    /// E1 — chip levantado del fondo (hover button, badge).
    pub const E1: Elev = (44, 4.0, 1.5);
    /// E2 — card/tile flotante sobre el panel (default cards).
    pub const E2: Elev = (60, 10.0, 4.0);
    /// E3 — superficie destacada (menú contextual, dropdown).
    pub const E3: Elev = (84, 18.0, 8.0);
    /// E4 — overlay sobre la app (modal, dialog).
    pub const E4: Elev = (110, 32.0, 14.0);
    /// E5 — sello de identidad (FAB, hero, picker activo).
    pub const E5: Elev = (140, 48.0, 22.0);
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

    /// Tema claro — contraste revisado para WCAG AA sobre `bg_app`:
    /// `fg_text` ~12:1, `fg_muted` ~5.4:1 (texto secundario legible),
    /// `fg_destructive` y `accent` oscurecidos para superar 4.5:1 sobre
    /// fondos claros. `fg_placeholder` queda deliberadamente tenue
    /// (hint, no contenido).
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
            fg_text: Color::from_rgba8(24, 32, 45, 255),
            fg_muted: Color::from_rgba8(86, 98, 116, 255),
            fg_placeholder: Color::from_rgba8(140, 150, 168, 255),
            fg_destructive: Color::from_rgba8(168, 48, 48, 255),
            border: Color::from_rgba8(190, 199, 214, 255),
            border_focus: Color::from_rgba8(48, 92, 196, 255),
            accent: Color::from_rgba8(48, 92, 196, 255),
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

    /// Tema "Print" — blanco y negro de alto contraste para impresión.
    /// Fondo blanco papel, tinta negra, sin grises decorativos: todo lo
    /// que se imprime tiene que leerse en una fotocopiadora. `fg_muted`
    /// es un gris medio (3.5:1) reservado a metadatos; el cuerpo va en
    /// negro puro. Acento y bordes negros — la tinta es una sola.
    pub const fn print() -> Self {
        Self {
            name: "Print",
            bg_app: Color::from_rgba8(255, 255, 255, 255),
            bg_panel: Color::from_rgba8(255, 255, 255, 255),
            bg_panel_alt: Color::from_rgba8(246, 246, 246, 255),
            bg_input: Color::from_rgba8(255, 255, 255, 255),
            bg_input_focus: Color::from_rgba8(248, 248, 248, 255),
            bg_button: Color::from_rgba8(238, 238, 238, 255),
            bg_button_hover: Color::from_rgba8(224, 224, 224, 255),
            bg_selected: Color::from_rgba8(220, 220, 220, 255),
            bg_row_hover: Color::from_rgba8(240, 240, 240, 255),
            fg_text: Color::from_rgba8(0, 0, 0, 255),
            fg_muted: Color::from_rgba8(90, 90, 90, 255),
            fg_placeholder: Color::from_rgba8(140, 140, 140, 255),
            fg_destructive: Color::from_rgba8(0, 0, 0, 255),
            border: Color::from_rgba8(0, 0, 0, 255),
            border_focus: Color::from_rgba8(0, 0, 0, 255),
            accent: Color::from_rgba8(0, 0, 0, 255),
        }
    }

    /// Skin **Windows XP "Luna"** — escritorio azul-gris claro, selección y
    /// acento en el azul XP (#316AC5), chrome celeste. Para la vista `windows-xp`.
    pub const fn xp_blue() -> Self {
        Self {
            name: "WinXP",
            bg_app: Color::from_rgba8(236, 240, 249, 255),
            bg_panel: Color::from_rgba8(214, 223, 247, 255),
            bg_panel_alt: Color::from_rgba8(60, 100, 190, 255), // franja azul (taskbar)
            bg_input: Color::from_rgba8(255, 255, 255, 255),
            bg_input_focus: Color::from_rgba8(248, 250, 255, 255),
            bg_button: Color::from_rgba8(222, 230, 246, 255),
            bg_button_hover: Color::from_rgba8(198, 214, 244, 255),
            bg_selected: Color::from_rgba8(49, 106, 197, 255), // azul de selección XP
            bg_row_hover: Color::from_rgba8(214, 226, 248, 255),
            fg_text: Color::from_rgba8(20, 30, 50, 255),
            fg_muted: Color::from_rgba8(78, 92, 120, 255),
            fg_placeholder: Color::from_rgba8(130, 142, 168, 255),
            fg_destructive: Color::from_rgba8(176, 32, 32, 255),
            border: Color::from_rgba8(122, 152, 206, 255),
            border_focus: Color::from_rgba8(49, 106, 197, 255),
            accent: Color::from_rgba8(36, 94, 220, 255), // Luna blue
        }
    }

    /// Skin **macOS (Big Sur claro)** — casi blanco, grises sutiles, acento
    /// azul de sistema (#0A84FF). Para la vista `mac`.
    pub const fn mac_light() -> Self {
        Self {
            name: "macOS",
            bg_app: Color::from_rgba8(246, 246, 248, 255),
            bg_panel: Color::from_rgba8(236, 236, 240, 255),
            bg_panel_alt: Color::from_rgba8(242, 242, 245, 235), // menubar translúcida
            bg_input: Color::from_rgba8(255, 255, 255, 255),
            bg_input_focus: Color::from_rgba8(252, 252, 255, 255),
            bg_button: Color::from_rgba8(228, 228, 233, 255),
            bg_button_hover: Color::from_rgba8(214, 214, 221, 255),
            bg_selected: Color::from_rgba8(10, 132, 255, 255),
            bg_row_hover: Color::from_rgba8(232, 234, 240, 255),
            fg_text: Color::from_rgba8(28, 28, 32, 255),
            fg_muted: Color::from_rgba8(110, 110, 120, 255),
            fg_placeholder: Color::from_rgba8(160, 160, 170, 255),
            fg_destructive: Color::from_rgba8(215, 58, 50, 255),
            border: Color::from_rgba8(208, 208, 215, 255),
            border_focus: Color::from_rgba8(10, 132, 255, 255),
            accent: Color::from_rgba8(10, 132, 255, 255),
        }
    }

    /// Skin **KDE Plasma "Breeze" (claro)** — gris papel (#eff0f1), acento
    /// azul Breeze (#3daee9). Para la vista `kde`.
    pub const fn kde_breeze() -> Self {
        Self {
            name: "Breeze",
            bg_app: Color::from_rgba8(239, 240, 241, 255),
            bg_panel: Color::from_rgba8(252, 252, 252, 255),
            bg_panel_alt: Color::from_rgba8(49, 54, 59, 255), // panel oscuro Breeze
            bg_input: Color::from_rgba8(255, 255, 255, 255),
            bg_input_focus: Color::from_rgba8(248, 252, 254, 255),
            bg_button: Color::from_rgba8(224, 226, 228, 255),
            bg_button_hover: Color::from_rgba8(208, 211, 214, 255),
            bg_selected: Color::from_rgba8(61, 174, 233, 255),
            bg_row_hover: Color::from_rgba8(227, 229, 231, 255),
            fg_text: Color::from_rgba8(35, 38, 41, 255),
            fg_muted: Color::from_rgba8(99, 104, 109, 255),
            fg_placeholder: Color::from_rgba8(150, 155, 160, 255),
            fg_destructive: Color::from_rgba8(218, 68, 83, 255),
            border: Color::from_rgba8(188, 192, 196, 255),
            border_focus: Color::from_rgba8(61, 174, 233, 255),
            accent: Color::from_rgba8(61, 174, 233, 255),
        }
    }

    /// Skin **Windows 3.1**: gris Motif (#c0c0c0) con barra de título azul
    /// marino (#000080) y escritorio teal. La era de los biseles. Para la vista
    /// `windows-3.1`.
    pub const fn win31() -> Self {
        Self {
            name: "Win3.1",
            bg_app: Color::from_rgba8(0, 128, 128, 255), // escritorio teal clásico
            bg_panel: Color::from_rgba8(192, 192, 192, 255), // gris ventana
            bg_panel_alt: Color::from_rgba8(0, 0, 128, 255), // barra de título azul marino
            bg_input: Color::from_rgba8(255, 255, 255, 255),
            bg_input_focus: Color::from_rgba8(255, 255, 255, 255),
            bg_button: Color::from_rgba8(192, 192, 192, 255),
            bg_button_hover: Color::from_rgba8(208, 208, 208, 255),
            bg_selected: Color::from_rgba8(0, 0, 128, 255),
            bg_row_hover: Color::from_rgba8(200, 200, 200, 255),
            fg_text: Color::from_rgba8(0, 0, 0, 255),
            fg_muted: Color::from_rgba8(64, 64, 64, 255),
            fg_placeholder: Color::from_rgba8(112, 112, 112, 255),
            fg_destructive: Color::from_rgba8(128, 0, 0, 255),
            border: Color::from_rgba8(128, 128, 128, 255),
            border_focus: Color::from_rgba8(0, 0, 128, 255),
            accent: Color::from_rgba8(0, 0, 128, 255), // azul Win3.1
        }
    }

    /// Skin **Solaris CDE** (era dorada): gris-beige Motif con acento teal —
    /// el Common Desktop Environment. Para la vista `solaris`.
    pub const fn cde() -> Self {
        Self {
            name: "CDE",
            bg_app: Color::from_rgba8(45, 70, 90, 255), // fondo azul-gris CDE
            bg_panel: Color::from_rgba8(174, 178, 195, 255), // gris-lila Motif
            bg_panel_alt: Color::from_rgba8(120, 130, 150, 255),
            bg_input: Color::from_rgba8(220, 222, 230, 255),
            bg_input_focus: Color::from_rgba8(235, 237, 244, 255),
            bg_button: Color::from_rgba8(160, 166, 185, 255),
            bg_button_hover: Color::from_rgba8(176, 182, 200, 255),
            bg_selected: Color::from_rgba8(90, 130, 130, 255),
            bg_row_hover: Color::from_rgba8(168, 174, 192, 255),
            fg_text: Color::from_rgba8(20, 24, 32, 255),
            fg_muted: Color::from_rgba8(64, 72, 84, 255),
            fg_placeholder: Color::from_rgba8(100, 108, 120, 255),
            fg_destructive: Color::from_rgba8(140, 40, 40, 255),
            border: Color::from_rgba8(108, 116, 134, 255),
            border_focus: Color::from_rgba8(64, 132, 132, 255),
            accent: Color::from_rgba8(64, 132, 132, 255), // teal CDE
        }
    }

    /// Superficie "hundida" — un escalón más profunda que `bg_app`, para
    /// áreas de lectura intensa (output de terminal, viewports de log,
    /// IDE-text) que deben recibir el texto con más contraste que el chrome
    /// y leerse recesadas respecto del marco. En temas oscuros oscurece
    /// `bg_app` hacia el negro; en claros lo aleja un paso del blanco. Las
    /// cards/strips (`bg_panel`, `bg_panel_alt`) quedan flotando por encima.
    /// Derivada de la paleta — no inventa un color suelto.
    pub fn sunken(&self) -> Color {
        let c = self.bg_app.components;
        // Luminancia relativa aproximada en sRGB (sin linealizar — alcanza
        // para decidir oscuro/claro).
        let lum = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let factor = if lum < 0.5 { 0.5 } else { 0.93 };
        Color::from_rgba8(
            (c[0] * factor * 255.0).round().clamp(0.0, 255.0) as u8,
            (c[1] * factor * 255.0).round().clamp(0.0, 255.0) as u8,
            (c[2] * factor * 255.0).round().clamp(0.0, 255.0) as u8,
            255,
        )
    }

    /// Todos los presets del repo, en el orden canónico de rotación
    /// (Dark → Light → Aurora → Sunset → Dark…). El theme-switcher
    /// los consume vía [`Theme::next_after`]. `print()` queda fuera de la
    /// rotación a propósito — es un modo deliberado (imprimir), no un
    /// gusto estético que se cicle por accidente.
    pub fn all() -> Vec<Self> {
        vec![Self::dark(), Self::light(), Self::aurora(), Self::sunset()]
    }

    /// Busca un preset por nombre exacto. Incluye los modos deliberados que
    /// quedan fuera de la rotación casual (`print` y los skins de vista
    /// `WinXP`/`macOS`/`Breeze`), para que `Config::theme` los resuelva.
    pub fn by_name(name: &str) -> Option<Self> {
        Self::all()
            .into_iter()
            .chain([
                Self::print(),
                Self::xp_blue(),
                Self::mac_light(),
                Self::kde_breeze(),
                Self::win31(),
                Self::cde(),
            ])
            .find(|t| t.name == name)
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

    /// En temas oscuros la superficie hundida es más oscura que el chrome
    /// (`bg_app`); en claros, también desciende (se lee recesada). En ambos
    /// casos difiere de `bg_app` — no es un no-op.
    #[test]
    fn sunken_is_deeper_than_bg_app() {
        let lum = |c: Color| {
            let k = c.components;
            0.2126 * k[0] + 0.7152 * k[1] + 0.0722 * k[2]
        };
        for t in Theme::all() {
            assert!(
                lum(t.sunken()) < lum(t.bg_app),
                "{}: sunken debe ser más oscura que bg_app",
                t.name
            );
        }
    }
}
