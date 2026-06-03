//! Config general del WM — los ajustes que no son atajos ([`crate::keymap`])
//! ni reglas de ventana ([`crate::rules`]): el comando de la terminal
//! dropdown, la geometría del cajón quake, los parámetros iniciales del
//! teselado y si el foco sigue al puntero.
//!
//! Mismo patrón que keymap/rules: RON de texto en
//! `~/.config/mirada/config.ron`, leído una vez al arrancar y aplicado al
//! [`Desktop`](crate::Desktop). Si el archivo no existe se escribe una
//! plantilla documentada y se usan los defaults; si está corrupto, se
//! avisa y se cae a los defaults.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mirada_layout::{LayoutMode, LayoutParams};
use mirada_protocol::Decorations;

/// `app_id` con el que se marca y reconoce la terminal dropdown (quake).
/// El comando configurable [`Config::dropterm_cmd`] **debe** fijar este
/// `app_id` (con `kitty --class`, `foot --app-id`, etc.) o el Cerebro no
/// la reconocerá al abrirse.
pub const DROPTERM_APP_ID: &str = "mirada.dropterm";

/// El comando por defecto de la terminal dropdown. `kitty --class` fija el
/// `app_id` en Wayland, que es como se la reconoce.
const DEFAULT_DROPTERM_CMD: &str = "kitty --class mirada.dropterm";

/// (De)serializa un [`LayoutMode`] como su `slug` de cadena (`"grid"`,
/// `"master-stack"`, …), reusando el vocabulario de [`crate::action`].
mod layout_slug_serde {
    use mirada_layout::LayoutMode;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(mode: &LayoutMode, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(crate::action::layout_slug(*mode))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<LayoutMode, D::Error> {
        let slug = String::deserialize(d)?;
        crate::action::layout_from_slug(&slug).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "modo de teselado desconocido «{slug}» (usa master-stack, centered-master, \
                 spiral, grid, columns, rows o monocle)"
            ))
        })
    }
}

/// Los ajustes del escritorio que el usuario puede configurar sin tocar el
/// keymap ni las reglas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Comando que lanza la terminal dropdown (quake). Debe fijar el
    /// `app_id` [`DROPTERM_APP_ID`] para que el Cerebro la reconozca.
    pub dropterm_cmd: String,
    /// Alto del cajón dropdown como porcentaje (`1..=100`) del alto de la
    /// salida; baja anclado arriba a todo el ancho.
    pub dropterm_height_pct: u32,
    /// Modo de teselado inicial de cada escritorio. En RON va como cadena
    /// con su `slug` (`"master-stack"`, `"grid"`, …): los guiones no son
    /// identificadores válidos para un enum sin comillas.
    #[serde(with = "layout_slug_serde")]
    pub layout: LayoutMode,
    /// Margen en píxeles alrededor de cada ventana teselada.
    pub gap: i32,
    /// Fracción del ancho de la ventana maestra (se acota a `0.05..=0.95`).
    pub master_ratio: f32,
    /// Cuántas ventanas van en el área maestra (`nmaster`; al menos 1).
    pub master_count: usize,
    /// Paso al agrandar/encoger el área maestra (`grow-master`/`shrink-master`).
    /// Más chico = control más fino. Se acota al rango útil.
    pub master_step: f32,
    /// Paso en px al mover o redimensionar una ventana flotante por teclado.
    pub float_step: i32,
    /// El foco del teclado sigue al puntero, sin necesidad de click.
    pub focus_follows_mouse: bool,
    /// Grosor del marco de ventana en píxeles; `0` = sin marco.
    pub border_width: i32,
    /// Color RGBA (`0..=255`) del marco de la ventana enfocada.
    pub border_focus: [u8; 4],
    /// Color RGBA (`0..=255`) del marco de las ventanas sin foco.
    pub border_normal: [u8; 4],
    /// Ruta a la fuente para las etiquetas del compositor (título, menú).
    /// Vacía = se prueba una lista de fuentes comunes del sistema.
    pub font_path: String,
    /// Ruta a la imagen de fondo del escritorio (PNG/JPEG/WebP). Vacía =
    /// color sólido. La imagen se escala para cubrir la salida (stretch).
    pub wallpaper_path: String,
    /// Entradas del menú raíz (estilo openbox) que aparece al click derecho
    /// sobre el fondo. Vacío = sin menú (el click derecho en el fondo no hace
    /// nada). Cada entrada lanza su `command` con `sh -c`.
    pub menu: Vec<MenuEntry>,
}

/// Una entrada del menú raíz: la etiqueta que se pinta y el comando que lanza.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MenuEntry {
    pub label: String,
    pub command: String,
}

impl Default for Config {
    fn default() -> Self {
        let lp = LayoutParams::default();
        let dec = Decorations::default();
        Self {
            dropterm_cmd: DEFAULT_DROPTERM_CMD.to_string(),
            dropterm_height_pct: 45,
            layout: lp.mode,
            gap: lp.gap,
            master_ratio: lp.master_ratio,
            master_count: lp.master_count,
            master_step: 0.05,
            float_step: 40,
            focus_follows_mouse: true,
            border_width: dec.border_width,
            border_focus: dec.border_focus,
            border_normal: dec.border_normal,
            font_path: String::new(),
            wallpaper_path: String::new(),
            menu: Vec::new(),
        }
    }
}

impl Config {
    /// El paso del área maestra, acotado a un rango útil (`0.01..=0.5`).
    pub fn master_step(&self) -> f32 {
        self.master_step.clamp(0.01, 0.5)
    }

    /// El paso en px para mover/redimensionar flotantes, al menos `1`.
    pub fn float_step(&self) -> i32 {
        self.float_step.max(1)
    }

    /// El alto del dropdown acotado a `1..=100`, listo para multiplicar.
    pub fn dropterm_height_pct(&self) -> i32 {
        self.dropterm_height_pct.clamp(1, 100) as i32
    }

    /// Los parámetros de decoración que derivan de la config (marco, …),
    /// con el grosor acotado a `>= 0`.
    pub fn decorations(&self) -> Decorations {
        Decorations {
            border_width: self.border_width.max(0),
            border_focus: self.border_focus,
            border_normal: self.border_normal,
        }
    }

    /// Los parámetros de teselado iniciales que derivan de la config, ya
    /// acotados — lo que se le da a cada escritorio al arrancar.
    pub fn layout_params(&self) -> LayoutParams {
        LayoutParams {
            mode: self.layout,
            master_ratio: self.master_ratio.clamp(0.05, 0.95),
            master_count: self.master_count.max(1),
            gap: self.gap.max(0),
        }
    }

    /// Parsea la config desde el texto RON de un archivo.
    pub fn from_ron(text: &str) -> Result<Config, String> {
        ron::from_str(text).map_err(|e| format!("RON inválido: {e}"))
    }

    /// La ruta canónica de la config: `~/.config/mirada/config.ron`.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "mirada")
            .map(|d| d.config_dir().join("config.ron"))
    }

    /// Carga la config de un archivo RON.
    pub fn load(path: &Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("E/S: {e}"))?;
        Config::from_ron(&text)
    }

    /// Vigila el archivo de config para recargarlo en caliente.
    pub fn watch(path: &Path) -> notify::Result<crate::watch::FileWatch> {
        crate::watch::FileWatch::new(path)
    }

    /// Carga la config del usuario con un fallback amable: si el archivo no
    /// existe, escribe una plantilla documentada y devuelve los defaults; si
    /// está corrupto, avisa y devuelve los defaults.
    pub fn load_or_default(path: &Path) -> Config {
        if path.exists() {
            match Config::load(path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "mirada · config «{}» inválida ({e}); uso los valores por defecto.",
                        path.display()
                    );
                    Config::default()
                }
            }
        } else {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match std::fs::write(path, CONFIG_TEMPLATE) {
                Ok(()) => eprintln!("mirada · plantilla de config escrita en {}", path.display()),
                Err(e) => eprintln!("mirada · no pude escribir la plantilla de config: {e}"),
            }
            Config::default()
        }
    }
}

/// La plantilla que se escribe la primera vez: los defaults explícitos, con
/// comentarios. Editarla cambia el comportamiento al reiniciar mirada.
const CONFIG_TEMPLATE: &str = "\
// Config de mirada — ajustes del escritorio que no son atajos (keymap.ron)
// ni reglas de ventana (rules.ron). Reinicia mirada para aplicar cambios.
(
    // Comando de la terminal dropdown (quake), que Super+grave despliega.
    // DEBE fijar el app_id `mirada.dropterm` para que mirada la reconozca:
    //   kitty --class mirada.dropterm   ·   foot --app-id mirada.dropterm
    dropterm_cmd: \"kitty --class mirada.dropterm\",
    // Alto del cajón dropdown, en % del alto de pantalla (baja desde arriba).
    dropterm_height_pct: 45,

    // Teselado inicial de cada escritorio.
    //   master-stack · centered-master · spiral · grid · columns · rows · monocle
    layout: \"master-stack\",
    gap: 8,                    // margen en px alrededor de cada ventana
    master_ratio: 0.6,         // fracción de ancho de la ventana maestra
    master_count: 1,           // cuántas ventanas en el área maestra
    master_step: 0.05,         // paso de grow/shrink-master (más chico = más fino)
    float_step: 40,            // paso en px para mover/redimensionar flotantes por teclado

    // El foco del teclado sigue al puntero (sin click). false = foco al clickear.
    focus_follows_mouse: true,

    // Marco de ventana. Colores RGBA en 0..=255; border_width: 0 = sin marco.
    border_width: 2,
    border_focus: (92, 143, 235, 255),    // azul al foco
    border_normal: (56, 56, 69, 255),     // gris discreto sin foco

    // Fuente para las etiquetas (título, menú). Vacía = se prueba una lista
    // de fuentes comunes del sistema (Liberation, DejaVu, Noto, Adwaita…).
    font_path: \"\",

    // Imagen de fondo del escritorio (PNG/JPEG/WebP). Vacía = color sólido.
    // Se escala para cubrir la salida. Ej: \"/home/yo/.config/mirada/fondo.png\".
    wallpaper_path: \"\",

    // Menú raíz (estilo openbox): aparece al click DERECHO sobre el fondo.
    // Vacío = sin menú. Cada entrada lanza su comando con `sh -c`. Ej:
    //   menu: [
    //       (label: \"Terminal\",  command: \"kitty\"),
    //       (label: \"Navegador\", command: \"firefox\"),
    //       (label: \"Archivos\",  command: \"nada\"),
    //   ],
    menu: [],
)
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_parses_to_the_defaults() {
        assert_eq!(Config::from_ron(CONFIG_TEMPLATE).unwrap(), Config::default());
    }

    #[test]
    fn omitted_fields_fall_back_to_defaults() {
        let c = Config::from_ron("( gap: 20 )").unwrap();
        assert_eq!(c.gap, 20);
        // El resto queda en su default.
        assert_eq!(c.dropterm_cmd, Config::default().dropterm_cmd);
        assert!(c.focus_follows_mouse);
    }

    #[test]
    fn layout_params_clamp_out_of_range_values() {
        let c = Config::from_ron("( master_ratio: 2.0, master_count: 0, gap: -5 )").unwrap();
        let lp = c.layout_params();
        assert_eq!(lp.master_ratio, 0.95);
        assert_eq!(lp.master_count, 1);
        assert_eq!(lp.gap, 0);
    }

    #[test]
    fn dropterm_height_is_clamped() {
        let c = Config::from_ron("( dropterm_height_pct: 250 )").unwrap();
        assert_eq!(c.dropterm_height_pct(), 100);
        let c = Config::from_ron("( dropterm_height_pct: 0 )").unwrap();
        assert_eq!(c.dropterm_height_pct(), 1);
    }

    #[test]
    fn decorations_derive_from_the_config_and_clamp_width() {
        let c = Config::from_ron(
            "( border_width: -3, border_focus: (10, 20, 30, 255), border_normal: (1, 2, 3, 4) )",
        )
        .unwrap();
        let d = c.decorations();
        assert_eq!(d.border_width, 0); // acotado a >= 0
        assert_eq!(d.border_focus, [10, 20, 30, 255]);
        assert_eq!(d.border_normal, [1, 2, 3, 4]);
    }

    #[test]
    fn default_decorations_match_the_protocol_default() {
        assert_eq!(Config::default().decorations(), Decorations::default());
    }

    #[test]
    fn the_layout_mode_parses_from_its_slug_string() {
        let c = Config::from_ron(r#"( layout: "centered-master" )"#).unwrap();
        assert_eq!(c.layout, LayoutMode::CenteredMaster);
    }

    #[test]
    fn an_unknown_layout_slug_is_rejected() {
        assert!(Config::from_ron(r#"( layout: "tetris" )"#).is_err());
    }
}
