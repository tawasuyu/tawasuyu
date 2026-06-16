//! Vistas — presets de **escritorio completo**.
//!
//! Una [`Vista`] empaqueta de una sola vez todo lo que define el aspecto y el
//! tacto del escritorio:
//!
//! - **decoraciones + layout** (un [`Config`]): alto de barra de título, grosor
//!   y color del marco, modo de teselado, margen, ajuste de wallpaper y el
//!   **tema** del chrome ([`Config::theme`] → `llimphi_theme::Theme::by_name`);
//! - **atajos** (el nombre de un preset de keymap de [`crate::profiles`]).
//!
//! Aplicar una vista = escribir su `Config` en `config.ron` (el compositor lo
//! recarga en caliente) y poner su keymap como perfil activo (que se vuelca a
//! `keymap.ron`). Sin tocar el compositor ni el protocolo.
//!
//! `mirada` es la vista **nativa** y el default: es exactamente
//! [`Config::default`] + el keymap `mirada`. Las demás reproducen el look de un
//! escritorio conocido con los medios que mirada tiene hoy (marco, barra de
//! título, teselado, paleta). La fidelidad fina (gradientes Luna, semáforo de
//! macOS, Kickoff de KDE) llega con los widgets de barra de `pata`.

use mirada_layout::LayoutMode;

use crate::config::Config;

/// Un preset de escritorio completo: decoraciones + layout + tema + teclas.
#[derive(Debug, Clone, PartialEq)]
pub struct Vista {
    /// Slug estable (`"mirada"`, `"windows-xp"`, `"mac"`, `"kde"`, `"hyprland"`, `"dwm"`).
    pub name: &'static str,
    /// Rótulo legible para el menú.
    pub label: &'static str,
    /// El preset de keymap (de [`crate::profiles`]) que activa esta vista.
    pub keymap: &'static str,
    /// La config de decoraciones/layout/tema/wallpaper de la vista.
    pub config: Config,
}

/// Los slugs de las vistas de fábrica, en orden de presentación. `mirada`
/// (la nativa) encabeza.
// windows-3.1 queda FUERA de la lista: su Program Manager pide una ventana
// normal (toplevel movible) que pata —una barra— no puede ser; hasta tener una
// app dedicada, no se ofrece como vista. La fn `vista_windows_31` sigue por si
// se reintroduce.
pub const VISTA_NAMES: [&str; 7] = [
    "mirada",
    "windows-xp",
    "mac",
    "kde",
    "solaris",
    "hyprland",
    "dwm",
];

impl Vista {
    /// Una vista de fábrica por slug, o `None` si no existe.
    pub fn by_name(name: &str) -> Option<Vista> {
        Some(match name {
            "mirada" => vista_mirada(),
            "windows-xp" => vista_windows_xp(),
            "windows-3.1" => vista_windows_31(),
            "mac" => vista_mac(),
            "kde" => vista_kde(),
            "solaris" => vista_solaris(),
            "hyprland" => vista_hyprland(),
            "dwm" => vista_dwm(),
            _ => return None,
        })
    }

    /// Todas las vistas de fábrica, en el orden de [`VISTA_NAMES`].
    pub fn all() -> Vec<Vista> {
        VISTA_NAMES
            .iter()
            .filter_map(|n| Vista::by_name(n))
            .collect()
    }

    /// El rótulo legible de una vista por slug (o el slug si no existe).
    pub fn label_for(name: &str) -> String {
        Vista::by_name(name)
            .map(|v| v.label.to_string())
            .unwrap_or_else(|| name.to_string())
    }
}

/// Config base de una vista: arranca del default y se le pisan los campos de
/// aspecto. Así hereda menú raíz, dropterm, overview, etc. sin repetirlos.
fn skin(
    theme: &str,
    layout: LayoutMode,
    gap: i32,
    border_width: i32,
    titlebar_height: i32,
    border_focus: [u8; 4],
    border_normal: [u8; 4],
) -> Config {
    Config {
        theme: theme.to_string(),
        layout,
        gap,
        border_width,
        titlebar_height,
        border_focus,
        border_normal,
        ..Config::default()
    }
}

/// **mirada** — la vista nativa y default: `Config::default()` tal cual.
fn vista_mirada() -> Vista {
    Vista {
        name: "mirada",
        label: "mirada (nativo)",
        keymap: "mirada",
        config: Config::default(),
    }
}

/// **Windows XP "Luna"** — barras de título altas azules, marco grueso, tema
/// celeste, teclas estilo Windows (Alt+Tab / Alt+F4 / Win+E).
fn vista_windows_xp() -> Vista {
    Vista {
        name: "windows-xp",
        label: "Windows XP",
        keymap: "windows",
        config: skin(
            "WinXP",
            LayoutMode::MasterStack,
            4,
            3,
            28,
            [36, 94, 220, 255],  // azul Luna con foco
            [122, 152, 206, 255], // celeste sin foco
        ),
    }
}

/// **macOS** — barra de título fina, marco de 1px, tema claro, teclas ⌘.
fn vista_mac() -> Vista {
    Vista {
        name: "mac",
        label: "macOS",
        keymap: "mac",
        config: skin(
            "macOS",
            LayoutMode::MasterStack,
            8,
            1,
            24,
            [10, 132, 255, 255],
            [208, 208, 215, 255],
        ),
    }
}

/// **KDE Plasma "Breeze"** — barra media, marco de 2px, tema Breeze claro,
/// teclas estilo Windows.
fn vista_kde() -> Vista {
    Vista {
        name: "kde",
        label: "KDE Plasma",
        keymap: "windows",
        config: skin(
            "Breeze",
            LayoutMode::MasterStack,
            6,
            2,
            26,
            [61, 174, 233, 255],
            [188, 192, 196, 255],
        ),
    }
}

/// **Hyprland** — sin barra de título, marco fino con acento, margen amplio
/// (aire de gaps redondeados), teselado en espiral (dwindle), tema oscuro.
fn vista_hyprland() -> Vista {
    Vista {
        name: "hyprland",
        label: "Hyprland",
        keymap: "hyprland",
        config: skin(
            "Dark",
            LayoutMode::Spiral,
            10,
            2,
            0,
            [110, 140, 220, 255],
            [46, 54, 70, 255],
        ),
    }
}

/// **dwm** — minimalismo puro: sin barra de título, marco de 1px, sin margen,
/// maestra+pila, tema oscuro.
fn vista_dwm() -> Vista {
    Vista {
        name: "dwm",
        label: "dwm",
        keymap: "dwm",
        config: skin(
            "Dark",
            LayoutMode::MasterStack,
            0,
            1,
            0,
            [110, 140, 220, 255],
            [46, 54, 70, 255],
        ),
    }
}

/// **Windows 3.1** — gris Motif con barra de título azul marino, marco
/// biselado, escritorio teal; el Program Manager lo monta la barra de pata.
fn vista_windows_31() -> Vista {
    Vista {
        name: "windows-3.1",
        label: "Windows 3.1",
        keymap: "windows",
        config: skin(
            "Win3.1",
            LayoutMode::MasterStack,
            2,
            2,
            20,
            [0, 0, 128, 255],       // azul marino con foco
            [128, 128, 128, 255],   // gris Motif sin foco
        ),
    }
}

/// **Solaris CDE** (era dorada) — Motif gris-azulado con acento teal, barras de
/// título medianas; el Front Panel inferior lo monta la barra de pata.
fn vista_solaris() -> Vista {
    Vista {
        name: "solaris",
        label: "Solaris (CDE)",
        keymap: "windows",
        config: skin(
            "CDE",
            LayoutMode::MasterStack,
            4,
            2,
            22,
            [64, 132, 132, 255],    // teal CDE con foco
            [108, 116, 134, 255],   // gris Motif sin foco
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todas_las_vistas_resuelven_y_la_nativa_es_el_default() {
        assert_eq!(Vista::all().len(), VISTA_NAMES.len());
        let mirada = Vista::by_name("mirada").unwrap();
        assert_eq!(mirada.keymap, "mirada");
        assert_eq!(mirada.config, Config::default()); // nativa = default exacto
    }

    #[test]
    fn cada_vista_referencia_un_keymap_de_fabrica() {
        use crate::keymap::Keymap;
        for v in Vista::all() {
            assert!(
                Keymap::is_builtin_name(v.keymap),
                "vista {} apunta a un keymap inexistente: {}",
                v.name,
                v.keymap
            );
        }
    }

    #[test]
    fn cada_vista_fija_un_tema_conocido() {
        // El Cerebro es UI-agnóstico: no resuelve la paleta (eso lo hace el
        // front con llimphi-theme), pero sí garantiza un nombre del set válido.
        let conocidos = [
            "Dark", "Light", "Aurora", "Sunset", "WinXP", "macOS", "Breeze", "Win3.1", "CDE",
        ];
        for v in Vista::all() {
            assert!(
                conocidos.contains(&v.config.theme.as_str()),
                "vista {} fija un tema fuera del set: {}",
                v.name,
                v.config.theme
            );
        }
    }

    #[test]
    fn las_vistas_se_distinguen_en_el_aspecto() {
        // dwm sin barra ni margen; XP con barra alta; mac con marco fino.
        let dwm = Vista::by_name("dwm").unwrap().config;
        assert_eq!(dwm.titlebar_height, 0);
        assert_eq!(dwm.gap, 0);
        let xp = Vista::by_name("windows-xp").unwrap().config;
        assert!(xp.titlebar_height >= 24);
        assert_eq!(xp.theme, "WinXP");
        let hypr = Vista::by_name("hyprland").unwrap().config;
        assert_eq!(hypr.titlebar_height, 0);
        assert_eq!(hypr.layout, LayoutMode::Spiral);
    }
}
