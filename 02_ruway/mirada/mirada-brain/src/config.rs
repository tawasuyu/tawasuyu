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

use mirada_layout::{Disposicion, LayoutMode, LayoutParams, WallpaperFit};
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

/// (De)serializa un [`WallpaperFit`] como su slug en kebab-case (`"stretch"`,
/// `"fit"`, `"fill"`, `"center"`, `"tile"`). El derive `serde` del propio
/// enum produce identificadores RON desnudos, incompatibles con la forma
/// quoteada que escribimos en la plantilla.
mod wallpaper_fit_slug_serde {
    use mirada_layout::WallpaperFit;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(fit: &WallpaperFit, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(fit.slug())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<WallpaperFit, D::Error> {
        let slug = String::deserialize(d)?;
        WallpaperFit::from_slug(&slug).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "modo de wallpaper desconocido «{slug}» (usa stretch, fit, fill, center o tile)"
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
    /// Alto de la barra de título en px; `0` = sin barra (sólo el título de la
    /// ventana enfocada superpuesto). Se reserva arriba de cada ventana.
    pub titlebar_height: i32,
    /// Ruta a la fuente para las etiquetas del compositor (título, menú).
    /// Vacía = se prueba una lista de fuentes comunes del sistema.
    pub font_path: String,
    /// Ruta a la imagen de fondo del escritorio (PNG/JPEG/WebP). Vacía =
    /// color sólido. Su colocación dentro de la salida la dicta
    /// [`Self::wallpaper_fit`].
    pub wallpaper_path: String,
    /// Cómo se ajusta el wallpaper a la salida: `stretch` (deforma para cubrir),
    /// `fit` (entra entero con barras), `fill` (cubre y recorta), `center`
    /// (tamaño nativo centrado) o `tile` (repetido). En RON va como cadena
    /// kebab-case: `"stretch"`, `"fit"`, `"fill"`, `"center"`, `"tile"`.
    #[serde(with = "wallpaper_fit_slug_serde")]
    pub wallpaper_fit: WallpaperFit,
    /// Entradas del menú raíz (estilo openbox) que aparece al click derecho
    /// sobre el fondo. Vacío = sin menú (el click derecho en el fondo no hace
    /// nada). Cada entrada lanza su `command` con `sh -c`.
    pub menu: Vec<MenuEntry>,
    /// Zonas de la pantalla (fracciones `0..=1`): **blancos de arrastre**.
    /// Al arrastrar una ventana sobre una zona, el compositor la resalta; al
    /// soltarla encima, la ancla a ese rect (flotante). Soltarla fuera de toda
    /// zona la deja flotando donde cae (overflow). Vacío = sin zonas. Es el
    /// primer preset; `mirada-ctl cycle-zones` cicla a los de [`Self::zone_presets`].
    pub zones: Vec<ZoneCfg>,
    /// Presets adicionales de zonas. `mirada-ctl cycle-zones` (bindeable a un
    /// atajo) cicla `zones → preset 0 → preset 1 → … → zones`. Cada preset es
    /// una lista de zonas como [`Self::zones`].
    pub zone_presets: Vec<Vec<ZoneCfg>>,
    /// Cómo se reparten los monitores en el escritorio global cuando hay más
    /// de uno: `"horizontal"` (uno al lado del otro, default) o `"vertical"`
    /// (uno encima del otro). El orden lo dicta [`OutputOverride::order`].
    /// Mismo vocabulario que [`mirada_layout::Disposicion`].
    pub output_direction: String,
    /// Overrides por salida (monitor). Cada entrada se identifica por el
    /// `name` del conector DRM (`HDMI-A-1`, `DP-1`, …) y puede sobreescribir
    /// el wallpaper, su modo de ajuste y el orden de la salida en el
    /// escritorio compuesto. Lo que no se indique cae al valor global.
    /// Vacío = orden de discovery, wallpaper global para todas.
    pub outputs: Vec<OutputOverride>,
    /// **Vista espacial** (el "Prezi" de mirada): habilita el zoom-out que
    /// muestra todos los escritorios como mosaicos para saltar entre ellos.
    /// `false` la deshabilita (la tecla/menú no hace nada).
    #[serde(default = "default_true")]
    pub overview_enabled: bool,
    /// Columnas de la grilla de mosaicos en la vista espacial. `0` = automático
    /// (≈ raíz cuadrada del número de escritorios; 9 → 3×3).
    #[serde(default)]
    pub overview_columns: u32,
    /// Duración en milisegundos del vuelo de cámara al abrir la vista espacial y
    /// al aterrizar en un escritorio. `0` = sin animación (salto seco).
    #[serde(default = "default_overview_anim_ms")]
    pub overview_anim_ms: u32,
    /// Mostrar el título de cada ventana sobre su miniatura en la vista
    /// espacial. `false` = sólo el rectángulo (mosaicos más limpios).
    #[serde(default = "default_true")]
    pub overview_show_titles: bool,
    /// Divisor de frames de las ventanas **de fondo** (visibles pero sin foco,
    /// teseladas): el Cuerpo les espacia los `wl_surface.frame` callbacks a 1 de
    /// cada N vblanks, así dejan de quemar GPU pintando a 60 Hz detrás del foco.
    /// `1` (default) = throttle apagado (todas a pleno ritmo). `2` = mitad de
    /// ritmo, `4` = un cuarto… La enfocada, las flotantes y la de pantalla
    /// completa siempre van a pleno ritmo; las dormidas (zoom-Z) ya tienen los
    /// frames cortados del todo.
    #[serde(default = "default_one")]
    pub background_frame_divisor: u32,
}

/// Default de los toggles que arrancan en `true` (serde necesita una fn).
fn default_true() -> bool {
    true
}

/// Default de los divisores que arrancan neutros (1 = sin efecto).
fn default_one() -> u32 {
    1
}

/// Default de [`Config::overview_anim_ms`]: un vuelo de cámara ágil.
fn default_overview_anim_ms() -> u32 {
    260
}

/// Ajustes específicos de una salida (monitor) — se aplican sólo a la salida
/// cuyo nombre coincide. Hoy alcanzan el fondo del escritorio: imagen y modo
/// de ajuste. Lo que se deja vacío (`""`) cae al valor global.
///
/// El `name` es el nombre del conector como lo reporta el backend DRM en sus
/// logs de arranque: `HDMI-A-1`, `DP-1`, `eDP-1`, … (mayúsculas y guiones).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputOverride {
    /// Nombre del conector DRM al que se aplica este override.
    pub name: String,
    /// Wallpaper específico de esta salida. Vacío = usa el global.
    #[serde(default)]
    pub wallpaper_path: String,
    /// Ajuste del wallpaper específico para esta salida. Vacío = usa el
    /// global. Mismo vocabulario que [`Config::wallpaper_fit`] (`"stretch"`,
    /// `"fit"`, `"fill"`, `"center"`, `"tile"`). Se guarda como slug en vez
    /// de [`WallpaperFit`] para que en RON quepa una cadena desnuda y
    /// `""` sirva como ausente — el `Option<WallpaperFit>` exigiría
    /// `Some("fill")` / `None`, ruido innecesario en la config.
    #[serde(default)]
    pub wallpaper_fit: String,
    /// Orden de esta salida en el escritorio compuesto: las salidas se
    /// disponen ordenadas crecientemente por `(order, name)`. La de menor
    /// `order` queda **primaria** (origen `(0, 0)`). Default `0` — entonces
    /// el desempate por `name` da un orden estable, predecible y reproducible
    /// (sin override, todas son `0` y mandan los nombres alfabéticamente).
    #[serde(default)]
    pub order: i32,
    /// Escala HiDPI en 120-avos: `120` = 100 %, `180` = 150 %, `240` = 200 %.
    /// Misma convención que `wp_fractional_scale` de Wayland y que
    /// [`mirada_layout::ESCALA_100`]. Vale `0` (default) = sin override → la
    /// salida se anuncia a 100 % nativo. Valores `<= 0` se ignoran.
    #[serde(default)]
    pub scale_120: u32,
    /// Rotación / espejado del scanout. Slugs: `"normal"` (default si vacío),
    /// `"90"`, `"180"`, `"270"`, `"flipped"`, `"flipped-90"`, `"flipped-180"`,
    /// `"flipped-270"`. Validado al cargar la config (`from_ron`); el
    /// compositor lo parsea a su `Transform` al usar.
    #[serde(default)]
    pub transform: String,
}

/// Parsea el slug de [`Config::output_direction`] a [`Disposicion`].
fn parse_disposition(slug: &str) -> Option<Disposicion> {
    match slug {
        "horizontal" => Some(Disposicion::Horizontal),
        "vertical" => Some(Disposicion::Vertical),
        _ => None,
    }
}

/// Slugs válidos para [`OutputOverride::transform`]. Mismo orden que la enum
/// `Transform` de smithay (Normal / 90 / 180 / 270 / Flipped / Flipped90 /
/// Flipped180 / Flipped270). El consumidor (drm_backend) hace el match a su
/// tipo; acá sólo validamos.
pub const TRANSFORM_SLUGS: &[&str] = &[
    "normal",
    "90",
    "180",
    "270",
    "flipped",
    "flipped-90",
    "flipped-180",
    "flipped-270",
];

/// `true` si `slug` es un valor reconocido de [`OutputOverride::transform`].
/// Vacío (`""`) cuenta como ausente y es válido — significa «sin override».
pub fn is_valid_transform_slug(slug: &str) -> bool {
    slug.is_empty() || TRANSFORM_SLUGS.contains(&slug)
}

impl OutputOverride {
    /// El `wallpaper_fit` parseado, si la cadena no está vacía. `None` =
    /// no se setea (el llamante debe caer al global). `Err` si la cadena
    /// trae un slug desconocido — se propaga al cargar la config.
    fn parsed_wallpaper_fit(&self) -> Result<Option<WallpaperFit>, String> {
        if self.wallpaper_fit.is_empty() {
            return Ok(None);
        }
        WallpaperFit::from_slug(&self.wallpaper_fit)
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "modo de wallpaper desconocido «{}» en outputs[name=\"{}\"] (usa stretch, fit, fill, center o tile)",
                    self.wallpaper_fit, self.name
                )
            })
    }
}

/// Una zona: `(x, y, w, h)` en fracciones `0..=1` de la pantalla. El `name` es
/// opcional, sólo una etiqueta para tu propia referencia (no se pinta).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneCfg {
    #[serde(default)]
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Una entrada del menú raíz. Es una **hoja** que lanza `command`, o un
/// **submenú** si trae `submenu` no vacío (en ese caso `command` se ignora).
/// La forma plana `(label, command)` sigue siendo válida: `submenu` default
/// vacío. Anidan a cualquier profundidad.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MenuEntry {
    pub label: String,
    /// Comando a lanzar (`sh -c`) si es hoja. Ignorado si hay `submenu`.
    #[serde(default)]
    pub command: String,
    /// Entradas hijas; no vacío = esta entrada es un submenú.
    #[serde(default)]
    pub submenu: Vec<MenuEntry>,
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
            titlebar_height: dec.titlebar_height,
            font_path: String::new(),
            wallpaper_path: String::new(),
            wallpaper_fit: WallpaperFit::default(),
            menu: default_root_menu(),
            zones: Vec::new(),
            zone_presets: Vec::new(),
            output_direction: "horizontal".to_string(),
            outputs: Vec::new(),
            overview_enabled: true,
            overview_columns: 0,
            overview_anim_ms: 260,
            overview_show_titles: true,
            background_frame_divisor: 1,
        }
    }
}

/// Menú raíz por defecto cuando el usuario no configura `menu`: un set mínimo
/// de acciones que cualquier escritorio espera al hacer click-derecho sobre el
/// fondo (terminal, navegador, lanzador, recargar config, cerrar sesión). Los
/// comandos usan fallbacks `||` para que funcionen sin saber qué tiene el
/// sistema instalado.
pub fn default_root_menu() -> Vec<MenuEntry> {
    let leaf = |label: &str, cmd: &str| MenuEntry {
        label: label.to_string(),
        command: cmd.to_string(),
        submenu: Vec::new(),
    };
    let sub = |label: &str, children: Vec<MenuEntry>| MenuEntry {
        label: label.to_string(),
        command: String::new(),
        submenu: children,
    };
    vec![
        leaf("Terminal", "shuma || kitty || alacritty || foot || xterm"),
        leaf("Navegador", "xdg-open https://duckduckgo.com"),
        leaf("Archivos", "xdg-open \"$HOME\""),
        leaf(
            "Lanzador de apps",
            "rofi -show drun || wofi --show drun || dmenu_run",
        ),
        sub(
            "Mirada",
            vec![
                leaf("Recargar config", "mirada-ctl reload-config || true"),
                leaf("Vista espacial", "mirada-ctl overview-toggle || true"),
                leaf("Ciclar zonas", "mirada-ctl cycle-zones || true"),
            ],
        ),
        sub(
            "Sesión",
            vec![
                leaf("Bloquear", "loginctl lock-session || swaylock || xset s activate"),
                leaf("Cerrar sesión", "loginctl terminate-user \"$USER\""),
                leaf("Suspender", "systemctl suspend"),
                leaf("Reiniciar", "systemctl reboot"),
                leaf("Apagar", "systemctl poweroff"),
            ],
        ),
    ]
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
            titlebar_height: self.titlebar_height.max(0),
        }
    }

    /// Columnas de la grilla de la **vista espacial** para `count` escritorios:
    /// el override [`overview_columns`](Self::overview_columns) si es `> 0`
    /// (acotado a `count`), o el automático ≈ raíz cuadrada redondeada hacia
    /// arriba (9 escritorios → 3 columnas). Nunca devuelve `0`.
    pub fn overview_grid_columns(&self, count: usize) -> usize {
        let count = count.max(1);
        if self.overview_columns > 0 {
            return (self.overview_columns as usize).min(count);
        }
        ((count as f32).sqrt().ceil() as usize).max(1)
    }

    /// La dirección de disposición de las salidas en el escritorio compuesto.
    /// Default `Horizontal` si el slug no se reconoce — el chequeo duro se
    /// hace al cargar la config (ver [`Self::from_ron`]).
    pub fn output_disposition(&self) -> Disposicion {
        parse_disposition(&self.output_direction).unwrap_or(Disposicion::Horizontal)
    }

    /// El `order` configurado para la salida `name` — `0` si no hay override.
    pub fn output_order_for(&self, name: &str) -> i32 {
        self.outputs
            .iter()
            .find(|o| o.name == name)
            .map(|o| o.order)
            .unwrap_or(0)
    }

    /// Escala HiDPI en 120-avos a usar para la salida `name`: el override si
    /// existe y es positivo; si no, `120` (100 % nativo, [`mirada_layout::ESCALA_100`]).
    pub fn output_scale_120_for(&self, name: &str) -> u32 {
        for o in &self.outputs {
            if o.name == name && o.scale_120 > 0 {
                return o.scale_120;
            }
        }
        mirada_layout::ESCALA_100 as u32
    }

    /// Slug de transformación a usar para la salida `name`: el override si
    /// existe y es no vacío; si no, `"normal"`. Vocabulario en
    /// [`TRANSFORM_SLUGS`]. Un slug inválido se ignora silenciosamente —
    /// el chequeo duro se hace al cargar la config (ver [`Self::from_ron`]).
    pub fn output_transform_for(&self, name: &str) -> &str {
        for o in &self.outputs {
            if o.name == name && is_valid_transform_slug(&o.transform) && !o.transform.is_empty() {
                return &o.transform;
            }
        }
        "normal"
    }

    /// La ruta del wallpaper a usar para la salida `name`. Si hay un override
    /// en [`Self::outputs`] con `wallpaper_path` no vacío para esa salida, se
    /// usa esa; si no, cae al global [`Self::wallpaper_path`]. Vacía = fondo
    /// de color sólido.
    pub fn wallpaper_path_for(&self, name: &str) -> &str {
        for o in &self.outputs {
            if o.name == name && !o.wallpaper_path.is_empty() {
                return &o.wallpaper_path;
            }
        }
        &self.wallpaper_path
    }

    /// El modo de ajuste del wallpaper para la salida `name`. Si hay un
    /// override en [`Self::outputs`] con `wallpaper_fit` no vacío para esa
    /// salida, se usa ese; si no, cae al global [`Self::wallpaper_fit`].
    /// Un slug inválido en el override se ignora silenciosamente — el chequeo
    /// duro se hace al cargar la config (ver [`Self::from_ron`]).
    pub fn wallpaper_fit_for(&self, name: &str) -> WallpaperFit {
        for o in &self.outputs {
            if o.name == name {
                if let Ok(Some(f)) = o.parsed_wallpaper_fit() {
                    return f;
                }
            }
        }
        self.wallpaper_fit
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

    /// Parsea la config desde el texto RON de un archivo. Valida también que
    /// los slugs de overrides sean conocidos —`wallpaper_fit` de cada
    /// [`OutputOverride`] y el [`Self::output_direction`] global— para que un
    /// typo (ej. `"marciano"`) se rechace acá con un mensaje claro, en vez
    /// de ignorarse en silencio al pintar.
    pub fn from_ron(text: &str) -> Result<Config, String> {
        let cfg: Config = ron::from_str(text).map_err(|e| format!("RON inválido: {e}"))?;
        for o in &cfg.outputs {
            o.parsed_wallpaper_fit()?;
            if !is_valid_transform_slug(&o.transform) {
                return Err(format!(
                    "transform desconocido «{}» en outputs[name=\"{}\"] (usa {})",
                    o.transform,
                    o.name,
                    TRANSFORM_SLUGS.join(", ")
                ));
            }
        }
        if parse_disposition(&cfg.output_direction).is_none() {
            return Err(format!(
                "output_direction desconocido «{}» (usa horizontal o vertical)",
                cfg.output_direction
            ));
        }
        Ok(cfg)
    }

    /// Serializa la config a RON (con los slugs de layout/wallpaper como
    /// cadenas, gracias a los `with` serdes). Es el inverso de [`Self::from_ron`].
    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }

    /// Persiste la config al archivo RON del usuario. Escribe atómicamente
    /// (tmp + rename) y crea el directorio si falta. Lo usa el panel de
    /// configuración para guardar lo que el usuario edita en la UI; el
    /// `FileWatch` del compositor recarga el cambio en caliente.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = self
            .to_ron()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("ron.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
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
    ///
    /// Si la env `MIRADA_RESET_CONFIG=1` está, ignora el archivo (no lo
    /// borra) y arranca con los defaults — escotilla para verificar cambios
    /// en defaults sin tener que `rm` la config a mano.
    pub fn load_or_default(path: &Path) -> Config {
        if std::env::var_os("MIRADA_RESET_CONFIG").is_some() {
            eprintln!(
                "mirada · MIRADA_RESET_CONFIG activo; ignoro «{}» y uso los defaults",
                path.display()
            );
            return Config::default();
        }
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
    // Barra de título sobre cada ventana (px). 0 = sin barra (sólo el título
    // de la ventana enfocada, superpuesto). La franja se reserva arriba.
    titlebar_height: 24,

    // Fuente para las etiquetas (título, menú). Vacía = se prueba una lista
    // de fuentes comunes del sistema (Liberation, DejaVu, Noto, Adwaita…).
    font_path: \"\",

    // Imagen de fondo del escritorio (PNG/JPEG/WebP). Vacía = color sólido.
    // Ej: \"/home/yo/.config/mirada/fondo.png\".
    wallpaper_path: \"\",
    // Cómo encaja la imagen en la salida:
    //   stretch — deforma para cubrir exactamente (default).
    //   fit     — la imagen entra entera, con barras negras (letterbox).
    //   fill    — la imagen cubre la salida, los bordes se recortan.
    //   center  — tamaño nativo centrado (padding negro o recorte si es grande).
    //   tile    — repetida en su tamaño nativo desde la esquina superior-izquierda.
    wallpaper_fit: \"stretch\",

    // Menú raíz (estilo openbox): aparece al click DERECHO sobre el fondo.
    // Una entrada es hoja (lanza command con `sh -c`) o submenú (si trae
    // `submenu`, anidable a cualquier profundidad). Vaciá la lista a `[]` si
    // no querés menú. Lo que sigue es el set por defecto — editalo, agregá
    // tus apps, o dejalo así para tener algo usable de fábrica.
    menu: [
        (label: \"Terminal\",  command: \"shuma || kitty || alacritty || foot || xterm\"),
        (label: \"Navegador\", command: \"xdg-open https://duckduckgo.com\"),
        (label: \"Archivos\",  command: \"xdg-open \\\"$HOME\\\"\"),
        (label: \"Lanzador de apps\", command: \"rofi -show drun || wofi --show drun || dmenu_run\"),
        (label: \"Mirada\", submenu: [
            (label: \"Recargar config\",  command: \"mirada-ctl reload-config || true\"),
            (label: \"Vista espacial\",   command: \"mirada-ctl overview-toggle || true\"),
            (label: \"Ciclar zonas\",     command: \"mirada-ctl cycle-zones || true\"),
        ]),
        (label: \"Sesión\", submenu: [
            (label: \"Bloquear\",      command: \"loginctl lock-session || swaylock || xset s activate\"),
            (label: \"Cerrar sesión\", command: \"loginctl terminate-user \\\"$USER\\\"\"),
            (label: \"Suspender\",     command: \"systemctl suspend\"),
            (label: \"Reiniciar\",     command: \"systemctl reboot\"),
            (label: \"Apagar\",        command: \"systemctl poweroff\"),
        ]),
    ],

    // Zonas: blancos de arrastre (fracciones 0..=1 de la pantalla). Al arrastrar
    // una ventana sobre una zona se resalta; al soltarla encima, aterriza en ese
    // rect; soltarla fuera la deja flotando donde cae (overflow). El `name` es
    // opcional (sólo tu referencia). Vacío = sin zonas. Ej (media/cuartos):
    //   zones: [
    //       (x: 0.0, y: 0.0, w: 0.5, h: 1.0),
    //       (x: 0.5, y: 0.0, w: 0.5, h: 0.5),
    //       (x: 0.5, y: 0.5, w: 0.5, h: 0.5),
    //   ],
    zones: [],

    // Presets adicionales de zonas. `mirada-ctl cycle-zones` (bindealo a un
    // atajo) cicla zones → preset 0 → preset 1 → … → zones. Ej:
    //   zone_presets: [
    //       [ (x: 0.0, y: 0.0, w: 0.5, h: 1.0), (x: 0.5, y: 0.0, w: 0.5, h: 1.0) ],
    //       [ (x: 0.0, y: 0.0, w: 1.0, h: 1.0) ],
    //   ],
    zone_presets: [],

    // Cómo se reparten los monitores en el escritorio global cuando hay más
    // de uno: \"horizontal\" (uno al lado del otro) o \"vertical\" (encima).
    output_direction: \"horizontal\",

    // Overrides por salida (monitor). Cada entrada identifica el conector
    // DRM por su `name` (ej. \"HDMI-A-1\", \"DP-1\", \"eDP-1\"; sale en los
    // logs de arranque del compositor). Sobreescribe wallpaper + orden +
    // escala HiDPI + transformación de la salida. Lo que se deja vacío
    // cae al global. La salida con `order` más chico queda primaria
    // (origen 0,0). `scale_120` en 120-avos (120=100%, 180=150%, 240=200%).
    // `transform`: normal / 90 / 180 / 270 / flipped / flipped-90 /
    // flipped-180 / flipped-270. Vacío = orden alfabético, sin overrides. Ej:
    //   outputs: [
    //       (name: \"DP-1\",     order: 0, scale_120: 240,
    //                            wallpaper_path: \"/home/yo/fondos/code.png\",
    //                            wallpaper_fit: \"fill\"),
    //       (name: \"HDMI-A-1\", order: 1, transform: \"90\",
    //                            wallpaper_path: \"/home/yo/fondos/sala.png\"),
    //   ],
    outputs: [],
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
    fn to_ron_round_trips_a_config_no_trivial() {
        // Una config con campos no-default en cada familia: layout, colores,
        // wallpaper, menú, zonas y override de salida. Round-trip por RON.
        let c = Config::from_ron(
            r#"( layout: "grid", gap: 12, master_ratio: 0.7, border_focus: (1, 2, 3, 255),
                 wallpaper_path: "/w.png", wallpaper_fit: "fill",
                 menu: [(label: "T", command: "kitty")],
                 zones: [(x: 0.0, y: 0.0, w: 0.5, h: 1.0)],
                 output_direction: "vertical",
                 outputs: [(name: "DP-1", scale_120: 240, transform: "90")] )"#,
        )
        .unwrap();
        let text = c.to_ron().expect("debe serializar a RON");
        let back = Config::from_ron(&text).expect("el RON serializado debe reparsear");
        assert_eq!(c, back);
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

    #[test]
    fn menu_flat_entries_parse_without_submenu() {
        let c = Config::from_ron(
            r#"( menu: [(label: "Terminal", command: "kitty")] )"#,
        )
        .unwrap();
        assert_eq!(c.menu.len(), 1);
        assert_eq!(c.menu[0].label, "Terminal");
        assert_eq!(c.menu[0].command, "kitty");
        assert!(c.menu[0].submenu.is_empty());
    }

    #[test]
    fn zones_parsean_con_nombre_opcional() {
        let c = Config::from_ron(
            r#"( zones: [
                (x: 0.0, y: 0.0, w: 0.6, h: 1.0),
                (name: "chat", x: 0.6, y: 0.0, w: 0.4, h: 1.0),
            ] )"#,
        )
        .unwrap();
        assert_eq!(c.zones.len(), 2);
        assert_eq!(c.zones[0].name, ""); // name es opcional
        assert!((c.zones[0].w - 0.6).abs() < 1e-6);
        assert_eq!(c.zones[1].name, "chat");
    }

    #[test]
    fn wallpaper_fit_parsea_de_su_slug() {
        for (slug, want) in [
            ("stretch", WallpaperFit::Stretch),
            ("fit", WallpaperFit::Fit),
            ("fill", WallpaperFit::Fill),
            ("center", WallpaperFit::Center),
            ("tile", WallpaperFit::Tile),
        ] {
            let c = Config::from_ron(&format!(r#"( wallpaper_fit: "{slug}" )"#)).unwrap();
            assert_eq!(c.wallpaper_fit, want);
        }
    }

    #[test]
    fn wallpaper_fit_default_es_stretch() {
        assert_eq!(Config::default().wallpaper_fit, WallpaperFit::Stretch);
    }

    #[test]
    fn output_override_aplica_su_wallpaper_solo_al_monitor_nombrado() {
        let c = Config::from_ron(
            r#"( wallpaper_path: "global.png", outputs: [
                (name: "HDMI-A-1", wallpaper_path: "sala.png"),
                (name: "DP-1",     wallpaper_path: "code.png", wallpaper_fit: "fill"),
            ] )"#,
        )
        .unwrap();
        assert_eq!(c.wallpaper_path_for("HDMI-A-1"), "sala.png");
        assert_eq!(c.wallpaper_fit_for("HDMI-A-1"), WallpaperFit::Stretch);
        assert_eq!(c.wallpaper_path_for("DP-1"), "code.png");
        assert_eq!(c.wallpaper_fit_for("DP-1"), WallpaperFit::Fill);
        // Salida sin override cae al global.
        assert_eq!(c.wallpaper_path_for("eDP-1"), "global.png");
        assert_eq!(c.wallpaper_fit_for("eDP-1"), WallpaperFit::Stretch);
    }

    #[test]
    fn output_override_con_path_vacio_no_pisa_al_global() {
        // Un override con sólo `name` (path vacío) deja el wallpaper en el
        // global — útil si sólo se quiere cambiar el `fit` del monitor.
        let c = Config::from_ron(
            r#"( wallpaper_path: "global.png", outputs: [
                (name: "HDMI-A-1", wallpaper_fit: "fit"),
            ] )"#,
        )
        .unwrap();
        assert_eq!(c.wallpaper_path_for("HDMI-A-1"), "global.png");
        assert_eq!(c.wallpaper_fit_for("HDMI-A-1"), WallpaperFit::Fit);
    }

    #[test]
    fn output_override_rechaza_fit_desconocido() {
        let err = Config::from_ron(
            r#"( outputs: [ (name: "DP-1", wallpaper_fit: "marciano") ] )"#,
        )
        .unwrap_err();
        assert!(err.contains("marciano"), "mensaje útil: {err}");
    }

    #[test]
    fn output_direction_default_es_horizontal() {
        let c = Config::default();
        assert_eq!(c.output_disposition(), Disposicion::Horizontal);
    }

    #[test]
    fn output_direction_parsea_vertical_y_horizontal() {
        let c = Config::from_ron(r#"( output_direction: "vertical" )"#).unwrap();
        assert_eq!(c.output_disposition(), Disposicion::Vertical);
        let c = Config::from_ron(r#"( output_direction: "horizontal" )"#).unwrap();
        assert_eq!(c.output_disposition(), Disposicion::Horizontal);
    }

    #[test]
    fn output_direction_desconocido_es_rechazado() {
        let err = Config::from_ron(r#"( output_direction: "diagonal" )"#).unwrap_err();
        assert!(err.contains("diagonal"), "mensaje útil: {err}");
    }

    #[test]
    fn output_order_for_cae_a_cero_sin_override() {
        let c = Config::default();
        assert_eq!(c.output_order_for("HDMI-A-1"), 0);
    }

    #[test]
    fn output_scale_120_default_es_100_pct_si_no_hay_override() {
        let c = Config::default();
        assert_eq!(c.output_scale_120_for("HDMI-A-1"), 120);
    }

    #[test]
    fn output_scale_120_lee_el_override() {
        let c = Config::from_ron(
            r#"( outputs: [
                (name: "DP-1", scale_120: 240),
                (name: "HDMI-A-1", scale_120: 0),
            ] )"#,
        )
        .unwrap();
        assert_eq!(c.output_scale_120_for("DP-1"), 240);
        // `scale_120: 0` cuenta como sin override → 100 %.
        assert_eq!(c.output_scale_120_for("HDMI-A-1"), 120);
        // Salida sin entrada → 100 %.
        assert_eq!(c.output_scale_120_for("eDP-1"), 120);
    }

    #[test]
    fn output_transform_default_es_normal() {
        let c = Config::default();
        assert_eq!(c.output_transform_for("HDMI-A-1"), "normal");
    }

    #[test]
    fn output_transform_lee_el_override() {
        let c = Config::from_ron(
            r#"( outputs: [
                (name: "HDMI-A-1", transform: "90"),
                (name: "DP-1", transform: "flipped-180"),
            ] )"#,
        )
        .unwrap();
        assert_eq!(c.output_transform_for("HDMI-A-1"), "90");
        assert_eq!(c.output_transform_for("DP-1"), "flipped-180");
        assert_eq!(c.output_transform_for("eDP-1"), "normal");
    }

    #[test]
    fn output_transform_desconocido_es_rechazado() {
        let err = Config::from_ron(
            r#"( outputs: [ (name: "DP-1", transform: "diagonal") ] )"#,
        )
        .unwrap_err();
        assert!(err.contains("diagonal"), "mensaje útil: {err}");
    }

    #[test]
    fn output_order_for_lee_el_override() {
        let c = Config::from_ron(
            r#"( outputs: [
                (name: "DP-1", order: 0),
                (name: "HDMI-A-1", order: 5),
            ] )"#,
        )
        .unwrap();
        assert_eq!(c.output_order_for("HDMI-A-1"), 5);
        assert_eq!(c.output_order_for("DP-1"), 0);
        // Sin entrada cae a 0.
        assert_eq!(c.output_order_for("eDP-1"), 0);
    }

    #[test]
    fn menu_nested_submenus_parse() {
        let c = Config::from_ron(
            r#"( menu: [
                (label: "Apps", submenu: [
                    (label: "Navegador", command: "firefox"),
                    (label: "Más", submenu: [
                        (label: "nada", command: "nada"),
                    ]),
                ]),
            ] )"#,
        )
        .unwrap();
        assert_eq!(c.menu.len(), 1);
        let apps = &c.menu[0];
        assert_eq!(apps.label, "Apps");
        assert_eq!(apps.submenu.len(), 2);
        assert_eq!(apps.submenu[0].command, "firefox");
        assert_eq!(apps.submenu[1].submenu[0].label, "nada");
    }
}
