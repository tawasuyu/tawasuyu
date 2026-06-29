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
#[allow(clippy::too_many_arguments)]
fn skin(
    theme: &str,
    layout: LayoutMode,
    gap: i32,
    border_width: i32,
    titlebar_height: i32,
    border_focus: [u8; 4],
    border_normal: [u8; 4],
    tiledad: f32,
) -> Config {
    Config {
        theme: theme.to_string(),
        layout,
        gap,
        border_width,
        titlebar_height,
        border_focus,
        border_normal,
        tiledad,
        ..Config::default()
    }
}

/// **mirada** — la vista nativa y default: `Config::default()` + el **glass
/// encendido** (es el único look de fábrica con cristal; el resto lo trae en 0).
/// El `Config::default()` global sigue con `glass_blur: 0` (doctrina «lo caro
/// nace opt-in»): el glass es un atributo del **theme mirada**, no del default
/// crudo, así que sólo aparece cuando este perfil/theme está activo.
fn vista_mirada() -> Vista {
    Vista {
        name: "mirada",
        label: "mirada (nativo)",
        keymap: "mirada",
        config: Config {
            glass_blur: 16,
            glass_quality: 2,
            ..Config::default()
        },
    }
}

/// **Windows XP "Luna"** — barras de título altas azules, marco grueso, tema
/// celeste, teclas estilo Windows (Alt+Tab / Alt+F4 / Win+E). Tiledad baja:
/// ventanas flotantes con z-order y un snap discreto en bordes/esquinas (aero).
fn vista_windows_xp() -> Vista {
    let mut config = skin(
        "WinXP",
        LayoutMode::MasterStack,
        4,
        3,
        28,
        [36, 94, 220, 255],   // azul Luna con foco
        [122, 152, 206, 255], // celeste sin foco
        0.2,
    );
    // El sello de Luna: barra de título con brillo (degradé claro→azul).
    config.titlebar_gradient = true;
    Vista {
        name: "windows-xp",
        label: "Windows XP",
        keymap: "windows",
        config,
    }
}

/// **macOS** — barra de título fina, marco de 1px, tema claro, teclas ⌘.
/// Tiledad muy baja: ventanas flotantes; el snap es mínimo (sólo el borde).
fn vista_mac() -> Vista {
    let mut config = skin(
        "macOS",
        LayoutMode::MasterStack,
        8,
        1,
        24,
        [176, 176, 184, 255], // hairline gris al foco (mac NO usa marco azul)
        [210, 210, 216, 255], // hairline más claro sin foco
        0.15,
    );
    // Barra clara casi blanca con título/íconos oscuros — el chrome de mac.
    config.titlebar_focus = Some([232, 232, 237, 255]);
    config.titlebar_normal = Some([244, 244, 247, 255]);
    config.titlebar_text = Some([60, 60, 66, 255]);
    Vista {
        name: "mac",
        label: "macOS",
        keymap: "mac",
        config,
    }
}

/// **KDE Plasma "Breeze"** — barra media, marco de 2px, tema Breeze claro,
/// teclas estilo Windows.
fn vista_kde() -> Vista {
    let mut config = skin(
        "Breeze",
        LayoutMode::MasterStack,
        6,
        2,
        26,
        [61, 174, 233, 255],  // hairline azul Breeze al foco
        [188, 192, 196, 255], // gris sin foco
        0.55,
    );
    // Breeze es plano y claro: barra gris papel con título oscuro (el azul es
    // sólo acento del marco al foco, no de la barra).
    config.titlebar_focus = Some([247, 248, 249, 255]);
    config.titlebar_normal = Some([239, 240, 241, 255]);
    config.titlebar_text = Some([35, 38, 41, 255]);
    Vista {
        name: "kde",
        label: "KDE Plasma",
        keymap: "windows",
        config,
    }
}

/// **Hyprland** — sin barra de título, marco fino con acento, margen amplio
/// (aire de gaps redondeados), teselado en espiral (dwindle), tema oscuro.
/// Tiledad casi máxima: soltar en casi cualquier lado tesela a la región más
/// cercana — flotar es la excepción, no la regla.
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
            0.95,
        ),
    }
}

/// **dwm** — minimalismo puro: sin barra de título, marco de 1px, sin margen,
/// maestra+pila, tema oscuro. Tiledad casi máxima: teselado de cuerpo entero.
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
            0.95,
        ),
    }
}

/// **Windows 3.1** — gris Motif con barra de título azul marino, marco
/// biselado, escritorio teal; el Program Manager lo monta la barra de pata.
fn vista_windows_31() -> Vista {
    let mut config = skin(
        "Win3.1",
        LayoutMode::MasterStack,
        2,
        4,                      // marco grueso biselado
        20,
        [198, 198, 198, 255],   // marco gris Motif (#c0c0c0) — biselado, no azul
        [174, 174, 174, 255],   // gris algo más apagado sin foco
        0.1,
    );
    config.border_bevel = true;
    // El sello de 3.1: marco gris biselado pero barra de título azul marino
    // (gris cuando la ventana no tiene foco). El texto sigue claro (legible).
    config.titlebar_focus = Some([0, 0, 128, 255]);
    config.titlebar_normal = Some([128, 128, 128, 255]);
    Vista {
        name: "windows-3.1",
        label: "Windows 3.1",
        keymap: "windows",
        config,
    }
}

/// **Solaris CDE** (era dorada) — Motif gris-azulado con acento teal, barras de
/// título medianas; el Front Panel inferior lo monta la barra de pata. Marco
/// **grueso con relieve 3D** (`border_bevel`): el look retro Motif/CDE de
/// ventanas «levantadas» con luz arriba-izquierda y sombra abajo-derecha.
fn vista_solaris() -> Vista {
    let mut config = skin(
        "CDE",
        LayoutMode::MasterStack,
        4,
        5,                      // marco grueso (Motif/CDE)
        22,
        [64, 132, 132, 255],    // teal CDE con foco
        [108, 116, 134, 255],   // gris Motif sin foco
        0.25,
    );
    config.border_bevel = true;
    Vista {
        name: "solaris",
        label: "Solaris (CDE)",
        keymap: "windows",
        config,
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
        // La nativa = default EXCEPTO el glass (es el único look de fábrica con
        // cristal; el default crudo lo trae en 0 por la doctrina opt-in).
        assert_eq!(mirada.config, Config { glass_blur: 16, glass_quality: 2, ..Config::default() });
        assert_eq!(Config::default().glass_blur, 0, "el default crudo NO trae glass");
        // El resto de las vistas tampoco (heredan `..Config::default()`).
        for v in Vista::all() {
            if v.name != "mirada" {
                assert_eq!(v.config.glass_blur, 0, "vista {} no debe traer glass", v.name);
            }
        }
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

    #[test]
    fn las_vistas_retro_traen_marco_grueso_con_relieve() {
        // CDE (y Win3.1) son los looks Motif: marco grueso con bevel 3D. El
        // resto va plano. Y el flag viaja en las decoraciones que el Cerebro
        // emite hacia el Cuerpo.
        let cde = Vista::by_name("solaris").unwrap().config;
        assert!(cde.border_bevel, "la vista CDE debe traer relieve 3D");
        assert!(cde.border_width >= 4, "el marco CDE debe ser grueso");
        assert!(cde.decorations().border_bevel, "el bevel debe viajar en Decorations");
        let w31 = Vista::by_name("windows-3.1").unwrap().config;
        assert!(w31.border_bevel, "Win3.1 también es Motif biselado");
        // Las modernas van planas.
        assert!(!Vista::by_name("mirada").unwrap().config.border_bevel);
        assert!(!Vista::by_name("mac").unwrap().config.border_bevel);
        assert!(!Config::default().border_bevel, "el default crudo va plano");
    }

    #[test]
    fn las_vistas_afinan_barra_y_marco_a_su_inspiracion() {
        // XP "Luna": barra con brillo (degradé).
        assert!(Vista::by_name("windows-xp").unwrap().config.titlebar_gradient);
        // mac y Breeze: barras claras con texto oscuro (legibilidad).
        for n in ["mac", "kde"] {
            let c = Vista::by_name(n).unwrap().config;
            let tb = c.titlebar_focus.expect("barra clara propia");
            let tx = c.titlebar_text.expect("texto oscuro propio");
            assert!(tb[0] > 200 && tb[1] > 200 && tb[2] > 200, "{n}: barra debe ser clara");
            assert!(tx[0] < 100 && tx[1] < 100 && tx[2] < 100, "{n}: texto debe ser oscuro");
        }
        // Win3.1: marco gris (claro) pero barra de título azul marino —
        // desacople real (la barra NO hereda el color del marco).
        let w31 = Vista::by_name("windows-3.1").unwrap().config;
        let marco = w31.border_focus;
        assert!(marco[0] > 150 && marco[1] > 150 && marco[2] > 150, "marco 3.1 gris claro");
        assert_eq!(w31.titlebar_focus, Some([0, 0, 128, 255]), "barra 3.1 azul marino");
        assert_ne!(
            w31.titlebar_focus.unwrap(),
            w31.border_focus,
            "la barra debe diferir del marco (desacople)"
        );
        // Por contraste, las modernas dejan la barra acoplada al marco.
        assert_eq!(Vista::by_name("mirada").unwrap().config.titlebar_focus, None);
    }

    #[test]
    fn la_tiledad_ordena_de_flotante_a_teselado() {
        // Las vistas «de ventanas» teselan poco; las tiling, casi todo. El
        // nativo queda en el medio (KDE6 equilibrado).
        let t = |n: &str| Vista::by_name(n).unwrap().config.tiledad;
        assert!(t("mac") < t("windows-xp"), "mac flota más que XP");
        assert!(t("windows-xp") < t("mirada"), "XP flota más que el nativo");
        assert!(t("mirada") < t("hyprland"), "el nativo tesela menos que hyprland");
        assert_eq!(t("dwm"), t("hyprland")); // ambos teselado puro
        // Todo el rango es difuso y válido.
        for v in Vista::all() {
            assert!((0.0..=1.0).contains(&v.config.tiledad), "{} fuera de rango", v.name);
        }
    }
}
