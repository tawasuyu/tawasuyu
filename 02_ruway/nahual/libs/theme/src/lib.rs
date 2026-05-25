//! `nahual_theme` — paleta de colores y backgrounds compartidos.
//!
//! El `Theme` se instala como `Global` de GPUI. Los widgets lo leen vía
//! `cx.global::<Theme>()` durante su `render`, y se subscriben con
//! `cx.observe_global::<Theme>(…)` para auto-redibujarse cuando cambia.
//!
//! Filosofía: el theme es **dato puro** (sin lógica de UI). No conoce a
//! ningún widget concreto. Cada widget pide slots semánticos (panel_bg,
//! row_hover, accent…) sin acoplarse a colores hex específicos.

use gpui::{hsla, linear_color_stop, linear_gradient, Background, Global, Hsla};

pub mod toolkit;

/// Paleta semántica del theme. Cada slot tiene un nombre funcional, no
/// cromático — así los widgets piden "fondo de panel" sin acoplarse a
/// "azul oscuro".
///
/// Convención de slots:
/// - `bg_*` se devuelve como `Background` (soporta gradientes); los widgets
///   lo pasan a `.bg(...)` directamente.
/// - `fg_*`, `accent`, `border` son `Hsla` (colores planos para texto y
///   ornamentos).
/// - `bg_row_*` son `Hsla` porque las filas de una lista virtualizada se
///   beneficiarían poco de un gradiente individual.
#[derive(Clone, Debug)]
pub struct Theme {
    pub name: &'static str,
    pub is_dark: bool,

    // Fondos.
    pub bg_app: Background,
    pub bg_panel: Background,
    pub bg_panel_alt: Background,
    pub bg_row_hover: Hsla,
    pub bg_row_active: Hsla,

    // Foregrounds.
    pub fg_text: Hsla,
    pub fg_muted: Hsla,
    pub fg_disabled: Hsla,

    // Acentos y ornamentos.
    pub accent: Hsla,
    pub accent_strong: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,

    /// Marker colors para indicar "este file está abierto en container N".
    /// Paleta circular — el N-ésimo container usa `marker_palette[n % len]`.
    pub marker_palette: Vec<Hsla>,
}

impl Global for Theme {}

/// Helper privado: deriva los 5 slots "ornament secundario"
/// (bg_input/button/button_hover + accent_destructive +
/// bg_destructive_hover) según `is_dark`.
///
/// Devuelve los slots en el orden de los métodos públicos del
/// `Theme`. Los métodos del impl los exponen individualmente.
fn ornament_slots(is_dark: bool) -> (Hsla, Hsla, Hsla, Hsla, Hsla) {
    if is_dark {
        (
            // bg_input: muy oscuro, sutil tinte azul/gris
            hsla(220.0 / 360.0, 0.20, 0.07, 1.0),
            // bg_button: medio oscuro
            hsla(220.0 / 360.0, 0.18, 0.20, 1.0),
            // bg_button_hover: un poco más claro
            hsla(220.0 / 360.0, 0.20, 0.27, 1.0),
            // accent_destructive: rojo medio-claro para visibilidad
            hsla(0.0, 0.55, 0.65, 1.0),
            // bg_destructive_hover: rojo oscuro de fondo
            hsla(0.0, 0.55, 0.18, 1.0),
        )
    } else {
        (
            hsla(220.0 / 360.0, 0.10, 0.97, 1.0),
            hsla(220.0 / 360.0, 0.15, 0.85, 1.0),
            hsla(220.0 / 360.0, 0.20, 0.75, 1.0),
            hsla(0.0, 0.65, 0.45, 1.0),
            hsla(0.0, 0.55, 0.92, 1.0),
        )
    }
}

impl Theme {
    /// Bg sutil para fields editables que se quieren marcar como
    /// "input target" sin ser un panel. Derivado de `is_dark`.
    pub fn bg_input(&self) -> Hsla {
        ornament_slots(self.is_dark).0
    }

    /// Bg para clickable controls (botones secundarios, edit/delete
    /// icons en filas). Más prominente que `bg_panel_alt`, menos que
    /// `accent`. Derivado de `is_dark`.
    pub fn bg_button(&self) -> Hsla {
        ornament_slots(self.is_dark).1
    }

    /// Hover de [`Self::bg_button`].
    pub fn bg_button_hover(&self) -> Hsla {
        ornament_slots(self.is_dark).2
    }

    /// Accent rojo para acciones destructivas (delete, drop, force).
    pub fn accent_destructive(&self) -> Hsla {
        ornament_slots(self.is_dark).3
    }

    /// Bg de hover sobre clickable destructive elements (icon ✕,
    /// botones de "borrar"). Más oscuro que `accent_destructive`.
    pub fn bg_destructive_hover(&self) -> Hsla {
        ornament_slots(self.is_dark).4
    }

    pub fn global(cx: &gpui::App) -> &Self {
        cx.global::<Self>()
    }

    /// Carga el theme persistido si existe + es válido; sino default
    /// a Nebula. El persisted lo escribe [`Self::set`] cada vez que el
    /// theme cambia (típicamente vía `theme_switcher`).
    pub fn install_default(cx: &mut gpui::App) {
        let theme = load_persisted().unwrap_or_else(Self::nebula);
        // Asegura que GTK/Qt arranquen con el `gtk.css` del tema activo.
        let _ = toolkit::export_toolkit_configs(&theme);
        cx.set_global(theme);
    }

    /// Reemplaza el theme global y persiste su `name` al config file.
    /// GPUI notifica a todos los `observe_global` suscriptores en el
    /// siguiente frame.
    ///
    /// La persistencia es best-effort: si write falla (no hay home,
    /// permission denied, etc.), el theme cambia in-memory pero no
    /// sobrevive al restart. No se rebota — la UX no se interrumpe
    /// por un I/O secundario.
    pub fn set(cx: &mut gpui::App, theme: Self) {
        let _ = persist(&theme);
        // Reexporta `gtk.css` para que las apps GTK/Qt sigan el cambio.
        let _ = toolkit::export_toolkit_configs(&theme);
        cx.set_global(theme);
    }

    /// Lista todos los presets en orden estable. Usado por el switcher para
    /// ciclar y por el menú de "Tema" cuando lo agreguemos.
    pub fn all() -> Vec<Self> {
        vec![
            Self::nebula(),
            Self::aurora(),
            Self::sunset(),
            Self::flat_dark(),
            Self::solarized_light(),
            Self::high_contrast(),
            Self::print_color(),
            Self::print_bw(),
        ]
    }

    /// Devuelve el preset cuyo `name` matchea (case-insensitive). `None` si
    /// el nombre no existe — útil para validar input de usuario al cargar
    /// preferencias persistidas.
    pub fn by_name(name: &str) -> Option<Self> {
        Self::all()
            .into_iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
    }

    /// Próximo preset en la rotación de `all()`. Si `current` no está, se
    /// vuelve al primero. La rotación es circular (último → primero).
    pub fn next_after(current: &str) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|t| t.name == current);
        match idx {
            Some(i) => all[(i + 1) % all.len()].clone(),
            None => all[0].clone(),
        }
    }

    // =====================================================================
    // Presets
    // =====================================================================

    /// **Nebula** — default. Gradiente vertical violáceo profundo → teal
    /// medianoche. Pensado para sentirse moderno y descansado de noche.
    pub fn nebula() -> Self {
        let bg_app = linear_gradient(
            165.0,
            linear_color_stop(hsla(265.0 / 360.0, 0.38, 0.07, 1.0), 0.0),
            linear_color_stop(hsla(195.0 / 360.0, 0.42, 0.09, 1.0), 1.0),
        );
        let bg_panel = linear_gradient(
            165.0,
            linear_color_stop(hsla(245.0 / 360.0, 0.28, 0.10, 1.0), 0.0),
            linear_color_stop(hsla(210.0 / 360.0, 0.30, 0.12, 1.0), 1.0),
        );
        let bg_panel_alt = linear_gradient(
            165.0,
            linear_color_stop(hsla(255.0 / 360.0, 0.25, 0.13, 1.0), 0.0),
            linear_color_stop(hsla(220.0 / 360.0, 0.27, 0.14, 1.0), 1.0),
        );

        Self {
            name: "Nebula",
            is_dark: true,
            bg_app,
            bg_panel,
            bg_panel_alt,
            bg_row_hover: hsla(220.0 / 360.0, 0.30, 0.20, 0.45),
            bg_row_active: hsla(280.0 / 360.0, 0.55, 0.28, 0.65),
            fg_text: hsla(210.0 / 360.0, 0.35, 0.88, 1.0),
            fg_muted: hsla(215.0 / 360.0, 0.22, 0.58, 1.0),
            fg_disabled: hsla(215.0 / 360.0, 0.10, 0.40, 1.0),
            accent: hsla(280.0 / 360.0, 0.65, 0.65, 1.0),
            accent_strong: hsla(285.0 / 360.0, 0.78, 0.74, 1.0),
            border: hsla(225.0 / 360.0, 0.20, 0.22, 1.0),
            border_strong: hsla(280.0 / 360.0, 0.40, 0.45, 1.0),
            marker_palette: vec![
                hsla(280.0 / 360.0, 0.65, 0.55, 0.45),
                hsla(195.0 / 360.0, 0.65, 0.50, 0.45),
                hsla(35.0 / 360.0, 0.75, 0.55, 0.45),
                hsla(135.0 / 360.0, 0.55, 0.50, 0.45),
                hsla(0.0, 0.60, 0.55, 0.45),
            ],
        }
    }

    /// **Aurora** — verdes-cian-azul, evoca aurora boreal. Más frío que
    /// Nebula, contraste alto.
    pub fn aurora() -> Self {
        let bg_app = linear_gradient(
            190.0,
            linear_color_stop(hsla(170.0 / 360.0, 0.45, 0.06, 1.0), 0.0),
            linear_color_stop(hsla(220.0 / 360.0, 0.50, 0.09, 1.0), 1.0),
        );
        let bg_panel = linear_gradient(
            190.0,
            linear_color_stop(hsla(165.0 / 360.0, 0.32, 0.10, 1.0), 0.0),
            linear_color_stop(hsla(215.0 / 360.0, 0.36, 0.12, 1.0), 1.0),
        );
        let bg_panel_alt = linear_gradient(
            190.0,
            linear_color_stop(hsla(170.0 / 360.0, 0.30, 0.13, 1.0), 0.0),
            linear_color_stop(hsla(220.0 / 360.0, 0.32, 0.15, 1.0), 1.0),
        );

        Self {
            name: "Aurora",
            is_dark: true,
            bg_app,
            bg_panel,
            bg_panel_alt,
            bg_row_hover: hsla(180.0 / 360.0, 0.40, 0.22, 0.50),
            bg_row_active: hsla(160.0 / 360.0, 0.55, 0.30, 0.65),
            fg_text: hsla(180.0 / 360.0, 0.20, 0.92, 1.0),
            fg_muted: hsla(185.0 / 360.0, 0.18, 0.62, 1.0),
            fg_disabled: hsla(185.0 / 360.0, 0.10, 0.40, 1.0),
            accent: hsla(150.0 / 360.0, 0.70, 0.55, 1.0),
            accent_strong: hsla(160.0 / 360.0, 0.85, 0.65, 1.0),
            border: hsla(195.0 / 360.0, 0.25, 0.20, 1.0),
            border_strong: hsla(160.0 / 360.0, 0.55, 0.45, 1.0),
            marker_palette: vec![
                hsla(150.0 / 360.0, 0.75, 0.50, 0.45),
                hsla(195.0 / 360.0, 0.70, 0.50, 0.45),
                hsla(225.0 / 360.0, 0.70, 0.55, 0.45),
                hsla(85.0 / 360.0, 0.65, 0.50, 0.45),
                hsla(330.0 / 360.0, 0.65, 0.55, 0.45),
            ],
        }
    }

    /// **Sunset** — naranjas-rosas-violetas profundos. Cálido, alto contraste
    /// con texto claro.
    pub fn sunset() -> Self {
        let bg_app = linear_gradient(
            170.0,
            linear_color_stop(hsla(20.0 / 360.0, 0.50, 0.08, 1.0), 0.0),
            linear_color_stop(hsla(310.0 / 360.0, 0.45, 0.10, 1.0), 1.0),
        );
        let bg_panel = linear_gradient(
            170.0,
            linear_color_stop(hsla(15.0 / 360.0, 0.32, 0.12, 1.0), 0.0),
            linear_color_stop(hsla(315.0 / 360.0, 0.30, 0.13, 1.0), 1.0),
        );
        let bg_panel_alt = linear_gradient(
            170.0,
            linear_color_stop(hsla(20.0 / 360.0, 0.30, 0.15, 1.0), 0.0),
            linear_color_stop(hsla(320.0 / 360.0, 0.28, 0.16, 1.0), 1.0),
        );

        Self {
            name: "Sunset",
            is_dark: true,
            bg_app,
            bg_panel,
            bg_panel_alt,
            bg_row_hover: hsla(25.0 / 360.0, 0.40, 0.25, 0.45),
            bg_row_active: hsla(5.0 / 360.0, 0.55, 0.32, 0.65),
            fg_text: hsla(30.0 / 360.0, 0.30, 0.92, 1.0),
            fg_muted: hsla(25.0 / 360.0, 0.20, 0.62, 1.0),
            fg_disabled: hsla(25.0 / 360.0, 0.10, 0.42, 1.0),
            accent: hsla(15.0 / 360.0, 0.78, 0.62, 1.0),
            accent_strong: hsla(355.0 / 360.0, 0.85, 0.68, 1.0),
            border: hsla(15.0 / 360.0, 0.25, 0.25, 1.0),
            border_strong: hsla(355.0 / 360.0, 0.55, 0.45, 1.0),
            marker_palette: vec![
                hsla(15.0 / 360.0, 0.80, 0.55, 0.45),
                hsla(310.0 / 360.0, 0.65, 0.55, 0.45),
                hsla(45.0 / 360.0, 0.80, 0.55, 0.45),
                hsla(285.0 / 360.0, 0.65, 0.60, 0.45),
                hsla(355.0 / 360.0, 0.70, 0.55, 0.45),
            ],
        }
    }

    /// **Flat Dark** — sin gradientes, paleta cool gris-azulado. Para quien
    /// prefiere monocromía. Útil para contrastar visualmente con los temas
    /// de gradiente.
    pub fn flat_dark() -> Self {
        let bg_app: Background = hsla(220.0 / 360.0, 0.15, 0.09, 1.0).into();
        let bg_panel: Background = hsla(220.0 / 360.0, 0.15, 0.12, 1.0).into();
        let bg_panel_alt: Background = hsla(220.0 / 360.0, 0.15, 0.14, 1.0).into();
        Self {
            name: "Flat Dark",
            is_dark: true,
            bg_app,
            bg_panel,
            bg_panel_alt,
            bg_row_hover: hsla(220.0 / 360.0, 0.20, 0.20, 1.0),
            bg_row_active: hsla(220.0 / 360.0, 0.40, 0.30, 1.0),
            fg_text: hsla(210.0 / 360.0, 0.20, 0.85, 1.0),
            fg_muted: hsla(215.0 / 360.0, 0.15, 0.55, 1.0),
            fg_disabled: hsla(215.0 / 360.0, 0.10, 0.40, 1.0),
            accent: hsla(210.0 / 360.0, 0.70, 0.55, 1.0),
            accent_strong: hsla(210.0 / 360.0, 0.85, 0.65, 1.0),
            border: hsla(220.0 / 360.0, 0.15, 0.20, 1.0),
            border_strong: hsla(220.0 / 360.0, 0.30, 0.35, 1.0),
            marker_palette: vec![
                hsla(210.0 / 360.0, 0.65, 0.55, 0.40),
                hsla(160.0 / 360.0, 0.55, 0.50, 0.40),
                hsla(30.0 / 360.0, 0.75, 0.55, 0.40),
                hsla(0.0, 0.55, 0.55, 0.40),
            ],
        }
    }

    /// **Solarized Light** — preset claro inspirado en la paleta clásica de
    /// Schoonover. Sin gradientes (en light un gradiente sutil pasa
    /// desapercibido y solo introduce ruido).
    pub fn solarized_light() -> Self {
        let bg_app: Background = hsla(44.0 / 360.0, 0.87, 0.94, 1.0).into();
        let bg_panel: Background = hsla(46.0 / 360.0, 0.42, 0.88, 1.0).into();
        let bg_panel_alt: Background = hsla(46.0 / 360.0, 0.42, 0.92, 1.0).into();
        Self {
            name: "Solarized Light",
            is_dark: false,
            bg_app,
            bg_panel,
            bg_panel_alt,
            bg_row_hover: hsla(46.0 / 360.0, 0.45, 0.80, 0.65),
            bg_row_active: hsla(45.0 / 360.0, 0.55, 0.72, 0.85),
            fg_text: hsla(196.0 / 360.0, 0.13, 0.30, 1.0),
            fg_muted: hsla(196.0 / 360.0, 0.13, 0.45, 1.0),
            fg_disabled: hsla(196.0 / 360.0, 0.10, 0.62, 1.0),
            accent: hsla(205.0 / 360.0, 0.69, 0.42, 1.0),
            accent_strong: hsla(205.0 / 360.0, 0.82, 0.38, 1.0),
            border: hsla(46.0 / 360.0, 0.30, 0.78, 1.0),
            border_strong: hsla(205.0 / 360.0, 0.40, 0.55, 1.0),
            marker_palette: vec![
                hsla(205.0 / 360.0, 0.69, 0.42, 0.30),
                hsla(175.0 / 360.0, 0.74, 0.32, 0.30),
                hsla(45.0 / 360.0, 1.00, 0.36, 0.30),
                hsla(331.0 / 360.0, 0.74, 0.42, 0.30),
                hsla(18.0 / 360.0, 0.89, 0.40, 0.30),
            ],
        }
    }

    /// **Print Color** — preview de impresión a color sobre papel.
    /// Fondo crema cálido (#f7f4ea-ish), texto y ornamentos en
    /// luminancias bajas para que sobrevivan ink-bleed. Sin gradientes
    /// (los gradients no imprimen bien) y sin glow.
    pub fn print_color() -> Self {
        let bg_app: Background = hsla(42.0 / 360.0, 0.30, 0.94, 1.0).into();
        let bg_panel: Background = hsla(40.0 / 360.0, 0.25, 0.97, 1.0).into();
        let bg_panel_alt: Background = hsla(40.0 / 360.0, 0.20, 0.92, 1.0).into();
        Self {
            name: "Print Color",
            is_dark: false,
            bg_app,
            bg_panel,
            bg_panel_alt,
            bg_row_hover: hsla(40.0 / 360.0, 0.30, 0.86, 0.70),
            bg_row_active: hsla(35.0 / 360.0, 0.45, 0.78, 0.85),
            fg_text: hsla(30.0 / 360.0, 0.15, 0.18, 1.0),
            fg_muted: hsla(30.0 / 360.0, 0.12, 0.40, 1.0),
            fg_disabled: hsla(30.0 / 360.0, 0.08, 0.62, 1.0),
            accent: hsla(15.0 / 360.0, 0.70, 0.40, 1.0),
            accent_strong: hsla(355.0 / 360.0, 0.78, 0.36, 1.0),
            border: hsla(40.0 / 360.0, 0.22, 0.82, 1.0),
            border_strong: hsla(30.0 / 360.0, 0.30, 0.55, 1.0),
            marker_palette: vec![
                hsla(15.0 / 360.0, 0.70, 0.35, 0.30),
                hsla(210.0 / 360.0, 0.65, 0.35, 0.30),
                hsla(140.0 / 360.0, 0.55, 0.30, 0.30),
                hsla(285.0 / 360.0, 0.55, 0.38, 0.30),
                hsla(40.0 / 360.0, 0.85, 0.40, 0.30),
            ],
        }
    }

    /// **Print B&W** — preview de impresión monocromática. Fondo
    /// blanco puro, todo en escala de grises. Cualquier slot que
    /// dependa de "color" en widgets astrológicos se diferencia por
    /// forma o por dash pattern, no por tinte.
    pub fn print_bw() -> Self {
        let bg_app: Background = hsla(0.0, 0.0, 1.00, 1.0).into();
        let bg_panel: Background = hsla(0.0, 0.0, 0.99, 1.0).into();
        let bg_panel_alt: Background = hsla(0.0, 0.0, 0.95, 1.0).into();
        Self {
            name: "Print B&W",
            is_dark: false,
            bg_app,
            bg_panel,
            bg_panel_alt,
            bg_row_hover: hsla(0.0, 0.0, 0.88, 0.85),
            bg_row_active: hsla(0.0, 0.0, 0.78, 0.95),
            fg_text: hsla(0.0, 0.0, 0.10, 1.0),
            fg_muted: hsla(0.0, 0.0, 0.40, 1.0),
            fg_disabled: hsla(0.0, 0.0, 0.65, 1.0),
            accent: hsla(0.0, 0.0, 0.20, 1.0),
            accent_strong: hsla(0.0, 0.0, 0.05, 1.0),
            border: hsla(0.0, 0.0, 0.80, 1.0),
            border_strong: hsla(0.0, 0.0, 0.40, 1.0),
            marker_palette: vec![
                hsla(0.0, 0.0, 0.30, 0.35),
                hsla(0.0, 0.0, 0.50, 0.35),
                hsla(0.0, 0.0, 0.20, 0.35),
                hsla(0.0, 0.0, 0.60, 0.35),
            ],
        }
    }

    /// **High Contrast** — accesibilidad. Negro puro con texto blanco y
    /// ornamentos amarillo/verde fuertes. Suficientemente diferente para
    /// notar inmediatamente al usar el switcher.
    pub fn high_contrast() -> Self {
        let bg_app: Background = hsla(0.0, 0.0, 0.0, 1.0).into();
        let bg_panel: Background = hsla(0.0, 0.0, 0.05, 1.0).into();
        let bg_panel_alt: Background = hsla(0.0, 0.0, 0.10, 1.0).into();
        Self {
            name: "High Contrast",
            is_dark: true,
            bg_app,
            bg_panel,
            bg_panel_alt,
            bg_row_hover: hsla(60.0 / 360.0, 1.00, 0.50, 0.35),
            bg_row_active: hsla(120.0 / 360.0, 1.00, 0.40, 0.55),
            fg_text: hsla(0.0, 0.0, 1.0, 1.0),
            fg_muted: hsla(0.0, 0.0, 0.75, 1.0),
            fg_disabled: hsla(0.0, 0.0, 0.50, 1.0),
            accent: hsla(60.0 / 360.0, 1.00, 0.60, 1.0),
            accent_strong: hsla(60.0 / 360.0, 1.00, 0.75, 1.0),
            border: hsla(0.0, 0.0, 0.30, 1.0),
            border_strong: hsla(60.0 / 360.0, 1.00, 0.60, 1.0),
            marker_palette: vec![
                hsla(60.0 / 360.0, 1.00, 0.55, 0.50),
                hsla(120.0 / 360.0, 1.00, 0.50, 0.50),
                hsla(180.0 / 360.0, 1.00, 0.55, 0.50),
                hsla(0.0, 1.00, 0.60, 0.50),
                hsla(300.0 / 360.0, 1.00, 0.65, 0.50),
            ],
        }
    }
}

// ============================================================================
// Persistencia de la preferencia de theme
// ============================================================================

use std::path::{Path, PathBuf};

const CONFIG_SUBDIR: &str = "nahual";
const CONFIG_FILE: &str = "theme";

/// Path al archivo donde se persiste la preferencia de theme.
///
/// Convención XDG: `$XDG_CONFIG_HOME/nahual/theme` si está set;
/// sino `$HOME/.config/nahual/theme`. `None` si ni `XDG_CONFIG_HOME`
/// ni `HOME` están definidos (típicamente en sandboxes / CI).
pub fn config_path() -> Option<PathBuf> {
    Some(config_home()?.join(CONFIG_SUBDIR).join(CONFIG_FILE))
}

/// Directorio base de configuración del usuario: `$XDG_CONFIG_HOME` si
/// está definido, sino `$HOME/.config`. `None` si ninguno está set
/// (típicamente en sandboxes / CI). Lo usan [`config_path`] y el módulo
/// [`toolkit`] para ubicar `gtk-3.0/gtk.css` y `gtk-4.0/gtk.css`.
pub(crate) fn config_home() -> Option<PathBuf> {
    std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".config"))
        })
}

/// Lee el theme persistido. `None` si: no hay config dir, file no
/// existe, lectura falla, o el name guardado no matchea ningún
/// preset (ej. tema renombrado entre versiones).
pub fn load_persisted() -> Option<Theme> {
    let path = config_path()?;
    load_from_path(&path)
}

/// Persiste el name del theme al config file. Crea el dir parent si
/// no existe. Devuelve el `io::Error` para que tests puedan
/// asertar; los call sites de producción pueden ignorarlo
/// (best-effort persistence).
pub fn persist(theme: &Theme) -> std::io::Result<()> {
    let path = config_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no se pudo determinar config dir (HOME/XDG_CONFIG_HOME no set)",
        )
    })?;
    persist_to_path(theme, &path)
}

/// Variante de [`load_persisted`] que toma un path explícito. Útil
/// para tests + para apps que quieren su propio path
/// (ej. multi-user single-machine).
pub fn load_from_path(path: &Path) -> Option<Theme> {
    let raw = std::fs::read_to_string(path).ok()?;
    let name = raw.trim();
    Theme::by_name(name)
}

/// Variante de [`persist`] que toma un path explícito.
pub fn persist_to_path(theme: &Theme, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, theme.name)
}

#[cfg(test)]
mod persistence_tests {
    use super::*;

    fn unique_path(label: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "nahual-theme-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            label
        ));
        p
    }

    #[test]
    fn persist_then_load_round_trip() {
        let path = unique_path("round-trip");
        let theme = Theme::aurora();
        persist_to_path(&theme, &path).unwrap();
        let loaded = load_from_path(&path).expect("load");
        assert_eq!(loaded.name, "Aurora");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn load_from_missing_file_returns_none() {
        let path = unique_path("missing");
        // path NO existe.
        assert!(load_from_path(&path).is_none());
    }

    #[test]
    fn load_from_unknown_name_returns_none() {
        let path = unique_path("unknown");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "DefinitelyNotARealTheme").unwrap();
        assert!(load_from_path(&path).is_none());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn persist_creates_parent_dir_if_missing() {
        let path = unique_path("nested-create");
        // Aseguramos que el parent NO existe antes.
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        persist_to_path(&Theme::sunset(), &path).unwrap();
        assert!(path.exists());
        let loaded = load_from_path(&path).unwrap();
        assert_eq!(loaded.name, "Sunset");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn config_path_uses_xdg_config_home_when_set() {
        // Snapshot del env, mutación local, restauración.
        let prev = std::env::var("XDG_CONFIG_HOME").ok();
        // SAFETY: tests del crate single-thread por default; este
        // env mutation no impacta otros tests del mismo proceso.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", "/custom/xdg");
        }
        let p = config_path().unwrap();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
        assert_eq!(p, PathBuf::from("/custom/xdg/nahual/theme"));
    }
}
