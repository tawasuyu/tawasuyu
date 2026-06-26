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

/// Colocación **rica** de un escritorio en el plano del Prezi: posición y tamaño
/// en **unidades de celda** (`1.0` = una celda de la grilla base) más un giro
/// propio en radianes. Generaliza la grilla entera [`Config::overview_geometry`]
/// — cada celda `(c, r)` equivale a `OverviewPlace { x: c, y: r, w: 1, h: 1,
/// rot: 0 }` — para permitir colocación libre y rotación. Es lo que autoriza el
/// editor de recorrido del panel y lo que la vista espacial respeta.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OverviewPlace {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Giro propio del tile alrededor de su centro, en radianes.
    pub rot: f32,
}

impl Default for OverviewPlace {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, w: 1.0, h: 1.0, rot: 0.0 }
    }
}

impl OverviewPlace {
    pub fn new(x: f32, y: f32, w: f32, h: f32, rot: f32) -> Self {
        Self { x, y, w, h, rot }
    }

    /// Centro del tile en unidades de celda.
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
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
    /// Pintar la barra de título con un degradé vertical (claro arriba → color
    /// base abajo) en vez de plana. La fija el theme del perfil activo.
    #[serde(default)]
    pub titlebar_gradient: bool,
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
    /// Carpeta de wallpapers para el **fondo automático** (slideshow): si no
    /// está vacía, el compositor rota por las imágenes de esa carpeta cada
    /// [`Self::wallpaper_interval_secs`]. Vacía = fondo fijo (`wallpaper_path`).
    #[serde(default)]
    pub wallpaper_dir: String,
    /// Segundos entre cambios de wallpaper del slideshow. `0` = sin rotar.
    #[serde(default)]
    pub wallpaper_interval_secs: u32,
    /// **Fuente** del fondo — el select inconfundible de arriba del formulario.
    /// `"auto"` (compat: imagen `wallpaper_path` / slideshow `wallpaper_dir` /
    /// gradiente por defecto, según qué esté seteado), `"color"` (color sólido),
    /// `"gradient"` (gradiente de `wallpaper_gradient`), `"local"` (una imagen,
    /// estática), `"directory"` (carpeta en rotación), `"procedural"` (geometría
    /// generada, `wallpaper_pattern` + `wallpaper_palette`) o `"remote"` (el
    /// daemon `mirada-wallpaper` baja la imagen y reescribe `wallpaper_path`).
    /// El compositor despacha sobre este valor.
    #[serde(default = "default_wallpaper_source")]
    pub wallpaper_source: String,
    /// Color sólido (RGB) para `wallpaper_source = "color"`.
    #[serde(default = "default_wallpaper_color")]
    pub wallpaper_color: [u8; 3],
    /// Stops del gradiente (RGB, de arriba a abajo) para `"gradient"`. Vacío o
    /// con un solo stop → el gradiente sobrio por defecto.
    #[serde(default)]
    pub wallpaper_gradient: Vec<[u8; 3]>,
    /// Patrón procedural (slug de `mirada_procedural::Pattern`: `"stripes"`,
    /// `"rings"`, `"waves"`, `"low-poly"`, `"voronoi"`, `"bauhaus"`) para
    /// `"procedural"`.
    #[serde(default = "default_wallpaper_pattern")]
    pub wallpaper_pattern: String,
    /// Paleta (RGB) del patrón procedural. Vacía → la paleta por defecto del
    /// motor.
    #[serde(default)]
    pub wallpaper_palette: Vec<[u8; 3]>,
    /// Entradas del menú raíz (estilo openbox) que aparece al click derecho
    /// sobre el fondo. Vacío = sin menú (el click derecho en el fondo no hace
    /// nada). Cada entrada lanza su `command` con `sh -c`. Si la config no trae
    /// la clave `menu`, se usa [`default_root_menu`] — que lista la suite real
    /// desde `app-bus`, fuente única (así una app nueva aparece sin editar nada).
    #[serde(default = "default_root_menu")]
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
    /// Autoarranque **rico**: apps que mirada lanza al iniciar la sesión, con
    /// la opción de **fijarles el escritorio** y de declararlas **remotas**
    /// (se envuelven en `waypipe ssh`). Es el equivalente estructurado del
    /// archivo `autostart` (una línea = un comando suelto, local y sin
    /// escritorio): acá una sesión de otra máquina se integra al diseño de
    /// escritorios igual que una app local. Ambos coexisten. Vacío = nada.
    #[serde(default)]
    pub startup: Vec<StartupApp>,
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
    /// Nombre del tema/paleta de `llimphi-theme` que pinta el chrome del
    /// escritorio (panel, barra, menús) — `llimphi_theme::Theme::by_name`.
    /// Lo fijan las **vistas** ([`crate::vistas`]): `"Dark"`, `"WinXP"`,
    /// `"macOS"`, `"Breeze"`, etc. Default `"Dark"`. No afecta los colores del
    /// marco de ventana (esos van por `border_focus`/`border_normal`).
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Cómo transiciona Win+Tab entre escritorios: salto seco (`Direct`),
    /// deslizamiento estilo Hyprland (`Hyprland`, **cableado**: el render aplica
    /// el slide horizontal con ease-out) o zoom-out a la vista espacial (`Prezi`,
    /// hoy cae al mismo slide). Default `Hyprland` para que Win+Tab anime.
    #[serde(default)]
    pub workspace_switch_mode: WorkspaceSwitchMode,
    /// Geometría 2D del Prezi: la celda `(col, fila)` de cada escritorio en el
    /// espacio del overview. `overview_geometry[i]` = escritorio `i` (0-based).
    /// Vacío = grilla automática (`i % cols`, `i / cols`). Lo edita el editor de
    /// Prezi del panel; la vista espacial lo respeta. Permite arreglos no
    /// rectangulares (una L, una cruz, una fila, etc.).
    #[serde(default)]
    pub overview_geometry: Vec<(i32, i32)>,
    /// Colocación **rica** de cada escritorio en el plano del Prezi (posición
    /// libre + tamaño + giro, en unidades de celda). Tiene prioridad sobre
    /// [`overview_geometry`](Self::overview_geometry) cuando hay una entrada por
    /// escritorio; vacío = se deriva de la grilla (cada celda → tile 1×1 sin
    /// giro). Lo edita el editor de recorrido del panel; la vista espacial lo
    /// respeta, incluida la rotación.
    #[serde(default)]
    pub overview_places: Vec<OverviewPlace>,
    /// Distribución de teclado XKB (`"us"`, `"es"`, `"latam"`, `"fr"`, …).
    /// Vacío = la del sistema (XKB_DEFAULT_LAYOUT / `us`). La aplica el
    /// compositor al crear el teclado; cambia al reiniciar la sesión.
    #[serde(default)]
    pub xkb_layout: String,
    /// Variante XKB opcional (`"dvorak"`, `"nodeadkeys"`, …). Vacío = sin
    /// variante.
    #[serde(default)]
    pub xkb_variant: String,
    /// Scroll natural (el contenido sigue al dedo/rueda) en punteros y
    /// touchpads. La aplica el compositor a cada dispositivo libinput.
    #[serde(default)]
    pub natural_scroll: bool,
    /// Tocar para clickear en touchpads (tap-to-click). Sólo afecta a los
    /// dispositivos que soportan tap.
    #[serde(default = "default_true")]
    pub tap_to_click: bool,
    /// Velocidad del puntero (aceleración libinput), de `-1.0` (lento) a `1.0`
    /// (rápido). `0.0` = neutro/sistema.
    #[serde(default)]
    pub pointer_speed: f64,
    /// Duración en ms del deslizamiento entre escritorios (Win+Tab estilo
    /// Hyprland). `0` = salto seco. Lo lee el compositor para el ease-out.
    #[serde(default = "default_slide_ms")]
    pub slide_ms: u32,
    /// Nombre del **tema de cursor** XCursor que pinta el puntero (los "sets":
    /// `"Soberania"`, `"Soberania-Light"`, o cualquier tema instalado en los
    /// directorios de íconos XDG — `~/.local/share/icons`, `/usr/share/icons`…).
    /// mirada trae embebidos los sets `Soberania*`; cambiar este campo cambia el
    /// set. Vacío = el cuadrado de software por defecto (sin tema).
    #[serde(default = "default_cursor_theme")]
    pub cursor_theme: String,
    /// Tamaño nominal del cursor en px (se elige la imagen del tema más cercana).
    /// Default 24.
    #[serde(default = "default_cursor_size")]
    pub cursor_size: u32,
    /// **Inactividad — apagar la pantalla** (DPMS) tras estos segundos sin
    /// actividad. `0` = nunca. Ver [`crate::idle`].
    #[serde(default)]
    pub idle_screen_off_secs: u32,
    /// **Inactividad — bloquear la sesión** tras estos segundos sin actividad.
    /// `0` = nunca.
    #[serde(default)]
    pub idle_lock_secs: u32,
    /// Si `true` (default), un reproductor de vídeo/llamada que pone un
    /// *idle inhibitor* pausa los contadores de inactividad (consciencia de
    /// multimedia): la pantalla no se apaga ni se bloquea mientras mirás algo.
    #[serde(default = "default_true")]
    pub idle_respect_inhibitors: bool,
    /// **Movimiento — animación de apertura de ventana.** Duración en ms del
    /// fundido de entrada (fade-in) con que aparece una ventana recién mapeada.
    /// `0` = aparición seca (sin animación). Lo lee el compositor en su path
    /// GLES (igual que el slide de Win+Tab). Ver `PLAN.md` §«Capa de
    /// embellecimiento».
    #[serde(default = "default_window_open_ms")]
    pub window_open_ms: u32,
    /// Curva del fade-in de apertura. Default desaceleración cúbica (la misma
    /// que el slide y el zoom del Prezi).
    #[serde(default)]
    pub window_open_easing: Easing,
    /// **Escala inicial del «pop» de apertura**, en porcentaje (50–100). La
    /// ventana nace a esta escala (centrada) y crece a 100 % mientras corre el
    /// fade. `100` = sin pop (sólo el fundido). Con curva `EaseOutBack` el
    /// crecimiento sobre-impulsa apenas por encima de 100 % y asienta — el
    /// rebote elástico clásico. Lo aplica el render envolviendo la ventana en un
    /// `RescaleRenderElement` (el mismo recurso que las miniaturas del Prezi).
    #[serde(default = "default_window_open_scale_pct")]
    pub window_open_scale_pct: u8,
    /// **Glow de foco — ms del fundido del marco al ganar/perder foco.** Al
    /// enfocar una ventana, su marco y barra de título transicionan del color
    /// «sin foco» al «con foco» (y al revés al desenfocar) en esta duración, en
    /// vez de cambiar de golpe. `0` = cambio seco. Lo aplica el render
    /// interpolando el color (no es un shader de bloom: es el crossfade barato).
    #[serde(default = "default_focus_glow_ms")]
    pub focus_glow_ms: u32,
    /// **Reducir movimiento** (accesibilidad): cuando está activo, el
    /// compositor pone en cero todas las duraciones de animación (apertura de
    /// ventana, slide entre escritorios, vuelo de cámara del Prezi). Un único
    /// interruptor maestro para quien marea el movimiento.
    #[serde(default)]
    pub reduce_motion: bool,
}

impl Config {
    /// Proyecta los campos de inactividad a la política pura [`crate::IdleConfig`].
    pub fn idle_config(&self) -> crate::IdleConfig {
        crate::IdleConfig {
            screen_off_secs: self.idle_screen_off_secs,
            lock_secs: self.idle_lock_secs,
            respect_inhibitors: self.idle_respect_inhibitors,
        }
    }
}

/// Default de [`Config::slide_ms`]: un slide ágil.
fn default_slide_ms() -> u32 {
    220
}

/// Default de [`Config::cursor_theme`]: el set propio de mirada.
fn default_cursor_theme() -> String {
    "Soberania".to_string()
}

/// Default de [`Config::cursor_size`]: un puntero cómodo.
fn default_cursor_size() -> u32 {
    24
}

/// Modo de transición de Win+Tab entre escritorios. Ver
/// [`Config::workspace_switch_mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceSwitchMode {
    /// Salto seco e instantáneo.
    Direct,
    /// Deslizamiento horizontal estilo Hyprland (cableado: slide con ease-out).
    /// Es el **default** para que Win+Tab anime sin configurar nada.
    #[default]
    Hyprland,
    /// Zoom-out a la vista espacial «Prezi» (hoy cae al mismo slide).
    Prezi,
}

impl WorkspaceSwitchMode {
    /// Slug estable para RON/UI (`"direct"`/`"hyprland"`/`"prezi"`).
    pub fn slug(self) -> &'static str {
        match self {
            WorkspaceSwitchMode::Direct => "direct",
            WorkspaceSwitchMode::Hyprland => "hyprland",
            WorkspaceSwitchMode::Prezi => "prezi",
        }
    }

    /// Parsea un slug; `None` si no calza.
    pub fn from_slug(s: &str) -> Option<Self> {
        match s {
            "direct" => Some(WorkspaceSwitchMode::Direct),
            "hyprland" => Some(WorkspaceSwitchMode::Hyprland),
            "prezi" => Some(WorkspaceSwitchMode::Prezi),
            _ => None,
        }
    }
}

/// Default del tema del chrome (el oscuro de `llimphi-theme`).
fn default_theme() -> String {
    "Dark".to_string()
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

/// Default de [`Config::window_open_ms`]: un fade-in breve y ágil.
fn default_window_open_ms() -> u32 {
    160
}

/// Default de [`Config::window_open_scale_pct`]: un pop sutil (nace al 92 %).
fn default_window_open_scale_pct() -> u8 {
    92
}

/// Default de [`Config::focus_glow_ms`]: un fundido de foco breve.
fn default_focus_glow_ms() -> u32 {
    140
}

/// Curva de interpolación para las animaciones del compositor (hoy el fade-in
/// de apertura; a futuro foco/cierre/menús). Es **pura**: `apply(t)` mapea un
/// progreso lineal `t∈[0,1]` a un progreso curvado. La comparten todos los
/// puntos que hoy hardcodean su curva (el slide de Win+Tab y el zoom del Prezi
/// usan `EaseOutCubic`), para que «calibrar el movimiento» sea un solo enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    /// Sin curva — progreso lineal.
    Linear,
    /// Desaceleración cúbica (arranca rápido y frena suave al final). El
    /// default: la misma curva que ya usan el slide y el Prezi.
    #[default]
    EaseOutCubic,
    /// Desaceleración con un leve sobre-impulso al final («pop» elástico). Para
    /// el fade-in el rebase de 1.0 se recorta (alfa tope), así que su efecto se
    /// nota recién cuando la apertura escale (pop) — ver `PLAN.md`.
    EaseOutBack,
}

impl Easing {
    /// Aplica la curva a `t∈[0,1]`. `EaseOutBack` puede devolver brevemente >1.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseOutCubic => 1.0 - (1.0 - t).powi(3),
            Easing::EaseOutBack => {
                // Constantes canónicas de easings.net para `easeOutBack`.
                const C1: f32 = 1.70158;
                const C3: f32 = C1 + 1.0;
                let u = t - 1.0;
                1.0 + C3 * u.powi(3) + C1 * u.powi(2)
            }
        }
    }

    /// Slug estable para RON/UI.
    pub fn slug(self) -> &'static str {
        match self {
            Easing::Linear => "linear",
            Easing::EaseOutCubic => "ease_out_cubic",
            Easing::EaseOutBack => "ease_out_back",
        }
    }

    /// Inversa de [`slug`](Self::slug); `None` si no calza.
    pub fn from_slug(s: &str) -> Option<Self> {
        match s {
            "linear" => Some(Easing::Linear),
            "ease_out_cubic" => Some(Easing::EaseOutCubic),
            "ease_out_back" => Some(Easing::EaseOutBack),
            _ => None,
        }
    }
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

/// Zonas de arrastre por defecto: **mitad izquierda** y **mitad derecha**. Así
/// la función drag-to-zone EXISTE de fábrica (antes `zones` venía vacío y no
/// pasaba nada al arrastrar — parecía que la feature no estaba). Arrastrá una
/// ventana al borde izq./der. para anclarla a esa mitad; soltala en el centro y
/// queda flotando (overflow). Más layouts en [`default_zone_presets`]
/// (`mirada-ctl cycle-zones`).
pub fn default_zones() -> Vec<ZoneCfg> {
    vec![
        ZoneCfg { name: "izquierda".into(), x: 0.0, y: 0.0, w: 0.5, h: 1.0 },
        ZoneCfg { name: "derecha".into(), x: 0.5, y: 0.0, w: 0.5, h: 1.0 },
    ]
}

/// Presets adicionales que cicla `mirada-ctl cycle-zones`: cuadrantes (cuatro
/// esquinas) y tercios verticales. El orden importa: el hit-test toma la PRIMERA
/// zona que contiene el punto.
pub fn default_zone_presets() -> Vec<Vec<ZoneCfg>> {
    vec![
        // Cuadrantes: arrastrá a una esquina para un cuarto de pantalla.
        vec![
            ZoneCfg { name: "sup-izq".into(), x: 0.0, y: 0.0, w: 0.5, h: 0.5 },
            ZoneCfg { name: "sup-der".into(), x: 0.5, y: 0.0, w: 0.5, h: 0.5 },
            ZoneCfg { name: "inf-izq".into(), x: 0.0, y: 0.5, w: 0.5, h: 0.5 },
            ZoneCfg { name: "inf-der".into(), x: 0.5, y: 0.5, w: 0.5, h: 0.5 },
        ],
        // Tercios verticales: tres columnas iguales. (Literales redondeados para
        // casar exactamente con la plantilla RON — ver test del template.)
        vec![
            ZoneCfg { name: "tercio-1".into(), x: 0.0, y: 0.0, w: 0.3333, h: 1.0 },
            ZoneCfg { name: "tercio-2".into(), x: 0.3333, y: 0.0, w: 0.3333, h: 1.0 },
            ZoneCfg { name: "tercio-3".into(), x: 0.6666, y: 0.0, w: 0.3334, h: 1.0 },
        ],
    ]
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

/// Una app de **autoarranque rico** (ver [`Config::startup`]). Lleva el comando,
/// y opcionalmente: el host remoto (la envuelve en `waypipe ssh`) y la ubicación
/// inicial (escritorio/flotante/fullscreen, igual que una [`crate::rules::Rule`]).
/// Así una sesión remota se declara y se ubica en el diseño de escritorios
/// exactamente como una app local.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StartupApp {
    /// Comando a lanzar (programa + argumentos), tal cual para `sh -c`. Si
    /// [`Self::remote`] no está vacío, es el comando que corre EN el host remoto.
    pub command: String,
    /// Host remoto `[user@]host`. Vacío = app local. No vacío = el comando se
    /// envuelve en `waypipe ssh <host> <command>` y la ventana llega por el
    /// túnel ssh como un cliente Wayland más (sin protocolo nuevo).
    #[serde(default)]
    pub remote: String,
    /// Escritorio de destino (1-based); `0` = donde caiga (el activo). Para que
    /// el anclado funcione hay que declarar también [`Self::app_id`] — así la
    /// ventana, al abrirse, se reconoce y se manda a este escritorio por la
    /// misma vía que `rules.ron`.
    #[serde(default)]
    pub workspace: usize,
    /// Subcadena del `app_id` de la ventana resultante, para ubicarla al abrir
    /// (mismo criterio que `rules.ron`). Necesaria si se quiere `workspace`,
    /// `floating` o `fullscreen`. Vacía = sólo se lanza, sin ubicación fija.
    #[serde(default)]
    pub app_id: String,
    /// Abrir la ventana flotando.
    #[serde(default)]
    pub floating: bool,
    /// Abrir la ventana en pantalla completa.
    #[serde(default)]
    pub fullscreen: bool,
    /// Puerto ssh del host remoto. `0` (o 22) = el de por defecto. Sólo aplica
    /// si [`Self::remote`] está puesto.
    #[serde(default)]
    pub ssh_port: u16,
    /// Ruta a la clave privada ssh (`-i`) para el host remoto. Vacía = la que
    /// elija ssh por defecto (agente/`~/.ssh/id_*`). Sólo aplica si
    /// [`Self::remote`] está puesto.
    #[serde(default)]
    pub ssh_key: String,
    /// Compresión del túnel waypipe — baja la latencia/ancho de banda. Vacío =
    /// el default de waypipe; `"none"`, `"lz4"` (rápido) o `"zstd"`/`"zstd=N"`
    /// (comprime más, mejor para enlaces flacos). Sólo aplica a sesiones remotas.
    #[serde(default)]
    pub compress: String,
    /// Codificar las superficies como vídeo (H.264/VP9 vía VAAPI): mucho menos
    /// ancho de banda en ventanas grandes/vídeo, a costa de nitidez. Sólo aplica
    /// a sesiones remotas.
    #[serde(default)]
    pub video: bool,
    /// Hilos de (de)compresión de waypipe. `0` = el default de waypipe. Sólo
    /// aplica a sesiones remotas.
    #[serde(default)]
    pub threads: u32,
}

impl StartupApp {
    /// El afinado de waypipe que declara esta entrada (compresión/vídeo/hilos).
    pub fn tuning(&self) -> WaypipeTuning {
        WaypipeTuning {
            compress: self.compress.clone(),
            video: self.video,
            threads: self.threads,
        }
    }

    /// El comando de shell ya resuelto: el `command` tal cual si es local, o
    /// envuelto en `waypipe [afinado] ssh [-p][-i] <host> …` si [`Self::remote`]
    /// está puesto.
    pub fn shell_command(&self) -> String {
        let host = self.remote.trim();
        if host.is_empty() {
            self.command.clone()
        } else {
            waypipe_command(&self.tuning(), self.ssh_port, &self.ssh_key, host, &self.command)
        }
    }

    /// La regla de ubicación equivalente (escritorio/flotante/fullscreen por
    /// `app_id`), o `None` si la entrada no fija ubicación o no trae `app_id`
    /// con el que reconocer la ventana al abrirse.
    pub fn placement_rule(&self) -> Option<crate::rules::Rule> {
        let pins = self.workspace > 0 || self.floating || self.fullscreen;
        let app_id = self.app_id.trim();
        if !pins || app_id.is_empty() {
            return None;
        }
        Some(crate::rules::Rule {
            app_id: app_id.to_string(),
            title: String::new(),
            workspace: self.workspace,
            floating: self.floating,
            fullscreen: self.fullscreen,
            size: (0, 0),
        })
    }
}

/// Afinado del túnel waypipe: baja latencia/ancho de banda comprimiendo o
/// codificando como vídeo el stream Wayland. Son **opciones globales** de
/// waypipe (van ANTES del subcomando `ssh`). El default no agrega banderas —
/// deja el comportamiento de fábrica de waypipe intacto.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WaypipeTuning {
    /// `--compress`: `""`/`"none"`/`"lz4"`/`"zstd"`/`"zstd=N"`. Vacío = default
    /// de waypipe (no se pasa la bandera).
    #[serde(default)]
    pub compress: String,
    /// `--video`: codifica las superficies como H.264/VP9 (VAAPI).
    #[serde(default)]
    pub video: bool,
    /// `--threads=N`: hilos de (de)compresión. `0` = default (no se pasa).
    #[serde(default)]
    pub threads: u32,
}

impl WaypipeTuning {
    /// Las banderas globales de waypipe que representa este afinado, en orden.
    /// Vacío = sin afinado (comportamiento de fábrica).
    pub fn flags(&self) -> Vec<String> {
        let mut v = Vec::new();
        let c = self.compress.trim();
        if !c.is_empty() {
            v.push(format!("--compress={c}"));
        }
        if self.video {
            v.push("--video".to_string());
        }
        if self.threads > 0 {
            v.push(format!("--threads={}", self.threads));
        }
        v
    }
}

/// Arma `waypipe [afinado] ssh [-p PORT] [-i KEY] <host> <command>`: corre
/// `command` en `host` y reenvía su protocolo Wayland por el túnel ssh. `host`
/// puede traer `user@`; `port == 0` o `22` y `key` vacía omiten esas banderas.
/// El afinado ([`WaypipeTuning`]) baja la latencia. Pura y testeable; es el
/// armador único que comparten `mirada-ctl remote` y el autoarranque `startup`.
pub fn waypipe_command(
    tuning: &WaypipeTuning,
    port: u16,
    key: &str,
    host: &str,
    command: &str,
) -> String {
    let mut s = String::from("waypipe");
    for f in tuning.flags() {
        s.push(' ');
        s.push_str(&f);
    }
    s.push_str(" ssh");
    if port != 0 && port != 22 {
        s.push_str(&format!(" -p {port}"));
    }
    let key = key.trim();
    if !key.is_empty() {
        s.push_str(&format!(" -i {key}"));
    }
    s.push(' ');
    s.push_str(host);
    let command = command.trim();
    if !command.is_empty() {
        s.push(' ');
        s.push_str(command);
    }
    s
}

/// Atajo de [`waypipe_command`] sin afinado ni opciones ssh: `waypipe ssh
/// <host> <command>`. Lo usa `mirada-ctl remote` (el camino simple).
pub fn waypipe_ssh_command(host: &str, command: &str) -> String {
    waypipe_command(&WaypipeTuning::default(), 0, "", host, command)
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
            titlebar_gradient: dec.titlebar_gradient,
            font_path: String::new(),
            wallpaper_path: String::new(),
            wallpaper_fit: WallpaperFit::default(),
            wallpaper_dir: String::new(),
            wallpaper_interval_secs: 0,
            wallpaper_source: default_wallpaper_source(),
            wallpaper_color: default_wallpaper_color(),
            wallpaper_gradient: Vec::new(),
            wallpaper_pattern: default_wallpaper_pattern(),
            wallpaper_palette: Vec::new(),
            menu: default_root_menu(),
            zones: default_zones(),
            zone_presets: default_zone_presets(),
            output_direction: "horizontal".to_string(),
            outputs: Vec::new(),
            startup: Vec::new(),
            overview_enabled: true,
            overview_columns: 0,
            overview_anim_ms: 260,
            overview_show_titles: true,
            background_frame_divisor: 1,
            theme: default_theme(),
            workspace_switch_mode: WorkspaceSwitchMode::default(),
            overview_geometry: Vec::new(),
            overview_places: Vec::new(),
            xkb_layout: String::new(),
            xkb_variant: String::new(),
            natural_scroll: false,
            tap_to_click: true,
            pointer_speed: 0.0,
            slide_ms: default_slide_ms(),
            cursor_theme: default_cursor_theme(),
            cursor_size: default_cursor_size(),
            idle_screen_off_secs: 0,
            idle_lock_secs: 0,
            idle_respect_inhibitors: true,
            window_open_ms: default_window_open_ms(),
            window_open_easing: Easing::default(),
            window_open_scale_pct: default_window_open_scale_pct(),
            focus_glow_ms: default_focus_glow_ms(),
            reduce_motion: false,
        }
    }
}

/// Menú raíz por defecto cuando el usuario no configura `menu`: un set mínimo
/// de acciones que cualquier escritorio espera al hacer click-derecho sobre el
/// fondo (terminal, navegador, lanzador, recargar config, cerrar sesión). Los
/// comandos usan fallbacks `||` para que funcionen sin saber qué tiene el
/// sistema instalado.
/// Fuente de wallpaper por defecto: `"auto"` — preserva el comportamiento
/// histórico (imagen/slideshow/gradiente según los campos legacy).
fn default_wallpaper_source() -> String {
    "auto".to_string()
}
/// Color sólido por defecto (azul-noche sobrio) para la fuente `"color"`.
fn default_wallpaper_color() -> [u8; 3] {
    [0x14, 0x16, 0x28]
}
/// Patrón procedural por defecto.
fn default_wallpaper_pattern() -> String {
    "waves".to_string()
}

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
    // La suite tawasuyu sale del registro único de `app-bus` (no de fallbacks
    // genéricos): cada app con su binario real. Antes el menú sólo ofrecía
    // `rofi`/`xdg-open`, así que las apps de la suite no se alcanzaban desde acá.
    let suite_apps: Vec<MenuEntry> = app_bus::default_entries()
        .into_iter()
        .filter_map(|e| match e.launch {
            app_bus::Launch::Exec { program, .. } => {
                let label = match &e.icon {
                    Some(ic) => format!("{ic}  {}", e.label),
                    None => e.label.clone(),
                };
                Some(leaf(&label, &program))
            }
            _ => None,
        })
        .collect();
    vec![
        leaf("Terminal", "shuma || kitty || alacritty || foot || xterm"),
        leaf("Navegador", "xdg-open https://duckduckgo.com"),
        leaf("Archivos", "xdg-open \"$HOME\""),
        sub("Apps de la suite", suite_apps),
        leaf(
            "Otras apps…",
            "rofi -show drun || wofi --show drun || dmenu_run",
        ),
        sub(
            "Vista",
            vec![
                leaf("mirada (nativo)", "mirada-ctl vista use mirada"),
                leaf("Windows XP", "mirada-ctl vista use windows-xp"),
                leaf("Windows 3.1", "mirada-ctl vista use windows-3.1"),
                leaf("macOS", "mirada-ctl vista use mac"),
                leaf("KDE Plasma", "mirada-ctl vista use kde"),
                leaf("Solaris (CDE)", "mirada-ctl vista use solaris"),
                leaf("Hyprland", "mirada-ctl vista use hyprland"),
                leaf("dwm", "mirada-ctl vista use dwm"),
            ],
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
            titlebar_gradient: self.titlebar_gradient,
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

    /// La celda `(col, fila)` de cada escritorio en el Prezi. Usa la geometría
    /// 2D configurada (`overview_geometry`) si tiene una entrada por escritorio;
    /// si no, cae a la grilla automática (`i % cols`, `i / cols`). Es la fuente
    /// única que consume el overview y el editor del panel.
    pub fn overview_geometry_for(&self, count: usize) -> Vec<(i32, i32)> {
        if self.overview_geometry.len() == count && count > 0 {
            return self.overview_geometry.clone();
        }
        let cols = self.overview_grid_columns(count).max(1) as i32;
        (0..count as i32).map(|i| (i % cols, i / cols)).collect()
    }

    /// La geometría 2D resultante de mover el escritorio `desktop` por
    /// `(dx, dy)` celdas — para el editor de Prezi del panel. Si otra celda ya
    /// ocupa el destino, **intercambian** posiciones (no se pisan). Clampa a
    /// coordenadas ≥ 0. Función pura: parte de [`overview_geometry_for`] y
    /// devuelve la geometría nueva (lista para guardar en `overview_geometry`).
    pub fn overview_geometry_moved(
        &self,
        count: usize,
        desktop: usize,
        dx: i32,
        dy: i32,
    ) -> Vec<(i32, i32)> {
        let mut geo = self.overview_geometry_for(count);
        let Some(&(c, r)) = geo.get(desktop) else {
            return geo;
        };
        let target = ((c + dx).max(0), (r + dy).max(0));
        if target == (c, r) {
            return geo;
        }
        if let Some(other) = (0..geo.len()).find(|&i| i != desktop && geo[i] == target) {
            geo[other] = (c, r); // swap: el ocupante toma la celda vieja
        }
        geo[desktop] = target;
        geo
    }

    /// La colocación **rica** de cada escritorio para `count` escritorios. Usa
    /// [`overview_places`](Self::overview_places) si tiene una entrada por
    /// escritorio; si no, deriva una colocación 1×1 sin giro desde la grilla
    /// [`overview_geometry_for`](Self::overview_geometry_for). Es la fuente única
    /// que consumen el overview (vista espacial) y el editor de recorrido del
    /// panel cuando quieren posición libre + rotación.
    pub fn overview_places_for(&self, count: usize) -> Vec<OverviewPlace> {
        if count == 0 {
            return Vec::new();
        }
        // Si el usuario arregló el mapa a mano (`overview_places`), RESPETAMOS su
        // arreglo aunque el conteo de escritorios haya cambiado: usamos las
        // colocaciones guardadas para los que existan y completamos los que
        // falten con la grilla por defecto (en vez de tirar todo —posición +
        // rotación— a la basura al primer desajuste de conteo, que reordenaba el
        // mapa y «perdía» lo que el panel mostraba guardado).
        if !self.overview_places.is_empty() {
            let mut v = self.overview_places.clone();
            if v.len() < count {
                let grid = self.overview_geometry_for(count);
                for (i, &(c, r)) in grid.iter().enumerate().take(count).skip(v.len()) {
                    v.push(OverviewPlace::new(c as f32, r as f32, 1.0, 1.0, 0.0));
                }
            }
            v.truncate(count);
            return v;
        }
        self.overview_geometry_for(count)
            .into_iter()
            .map(|(c, r)| OverviewPlace::new(c as f32, r as f32, 1.0, 1.0, 0.0))
            .collect()
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

    /// Las apps de **autoarranque rico** declaradas en la config. El archivo
    /// `autostart` (una línea por comando) sigue siendo la vía simple; esto es
    /// el complemento estructurado (remoto + escritorio). Ambos coexisten.
    pub fn startup(&self) -> &[StartupApp] {
        &self.startup
    }

    /// Las reglas de ubicación que derivan de las apps de autoarranque que
    /// fijan escritorio/flotante/fullscreen — se agregan a las reglas del
    /// usuario para que cada app de `startup` aterrice donde se declaró.
    pub fn startup_rules(&self) -> Vec<crate::rules::Rule> {
        self.startup.iter().filter_map(StartupApp::placement_rule).collect()
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

    // Imagen de fondo del escritorio (PNG/JPEG/WebP). Vacía = el wallpaper de
    // MARCA de la suite (chakana + cuatro cuadrantes), embebido vía el crate
    // `marca`. Ej: \"/home/yo/.config/mirada/fondo.png\".
    wallpaper_path: \"\",
    // Cómo encaja la imagen en la salida:
    //   stretch — deforma para cubrir exactamente (default).
    //   fit     — la imagen entra entera, con barras negras (letterbox).
    //   fill    — la imagen cubre la salida, los bordes se recortan.
    //   center  — tamaño nativo centrado (padding negro o recorte si es grande).
    //   tile    — repetida en su tamaño nativo desde la esquina superior-izquierda.
    wallpaper_fit: \"stretch\",

    // Menú raíz (estilo openbox): aparece al click DERECHO sobre el fondo.
    // Si NO ponés la clave `menu`, mirada usa su menú por defecto, que lista
    // la SUITE tawasuyu real (nahual, pluma, cosmos, media, nakui…) desde el
    // registro único de apps, más los submenús Vista/Mirada/Sesión. Una app
    // nueva de la suite aparece sola, sin tocar este archivo.
    //
    // Descomentá y editá esto SÓLO si querés un menú propio (reemplaza al
    // default por completo). Una entrada es hoja (lanza `command` con `sh -c`)
    // o submenú (si trae `submenu`, anidable). `menu: []` = sin menú.
    //
    // menu: [
    //     (label: \"Terminal\",  command: \"shuma || kitty || foot || xterm\"),
    //     (label: \"Archivos\",  command: \"nahual-shell-llimphi\"),
    //     (label: \"Apagar\",    command: \"systemctl poweroff\"),
    // ],

    // Zonas: blancos de arrastre (fracciones 0..=1 de la pantalla). Al arrastrar
    // una ventana sobre una zona se resalta; al soltarla encima, aterriza en ese
    // rect; soltarla fuera la deja flotando donde cae (overflow). El `name` es
    // opcional (sólo tu referencia). Vacío = sin zonas. Por defecto: mitades
    // izquierda/derecha (arrastrá una ventana al borde para anclarla a esa mitad).
    zones: [
        (name: \"izquierda\", x: 0.0, y: 0.0, w: 0.5, h: 1.0),
        (name: \"derecha\",   x: 0.5, y: 0.0, w: 0.5, h: 1.0),
    ],

    // Presets adicionales de zonas. `mirada-ctl cycle-zones` (bindealo a un
    // atajo) cicla zones → preset 0 → preset 1 → … → zones. Por defecto:
    // cuadrantes (esquinas) y tercios verticales.
    zone_presets: [
        [
            (name: \"sup-izq\", x: 0.0, y: 0.0, w: 0.5, h: 0.5),
            (name: \"sup-der\", x: 0.5, y: 0.0, w: 0.5, h: 0.5),
            (name: \"inf-izq\", x: 0.0, y: 0.5, w: 0.5, h: 0.5),
            (name: \"inf-der\", x: 0.5, y: 0.5, w: 0.5, h: 0.5),
        ],
        [
            (name: \"tercio-1\", x: 0.0,     y: 0.0, w: 0.3333, h: 1.0),
            (name: \"tercio-2\", x: 0.3333,  y: 0.0, w: 0.3333, h: 1.0),
            (name: \"tercio-3\", x: 0.6666,  y: 0.0, w: 0.3334, h: 1.0),
        ],
    ],

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

    // Autoarranque RICO: apps que mirada lanza al iniciar la sesión. Es el
    // complemento estructurado del archivo `autostart` (una línea por comando,
    // local y sin escritorio) — acá cada entrada puede, además del comando:
    //   remote:    \"[user@]host\"  → la envuelve en `waypipe ssh` (app de OTRA
    //                                máquina; la ventana llega como cliente local).
    //   workspace: 1..9            → la fija a ese escritorio al abrir.
    //   app_id:    \"foot\"          → con qué app_id reconocer la ventana para
    //                                ubicarla (NECESARIO para workspace/floating).
    //   floating / fullscreen      → cómo abrirla.
    //   ssh_port / ssh_key         → puerto e identidad ssh del host remoto.
    //   compress / video / threads → AFINADO de waypipe para bajar la latencia:
    //                                compress \"lz4\"|\"zstd\", video (H.264/VP9),
    //                                threads N. Sólo afectan a sesiones remotas.
    // Una sesión waypipe se integra al diseño de escritorios igual que una local.
    // Ej:
    //   startup: [
    //       (command: \"foot\", remote: \"sergio@servidor\", app_id: \"foot\",
    //        workspace: 3, ssh_port: 2222, compress: \"zstd\", video: true),
    //       (command: \"mpv\",  app_id: \"mpv\", fullscreen: true),
    //   ],
    startup: [],
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
    fn easing_ancla_los_extremos_y_curva_el_medio() {
        for e in [Easing::Linear, Easing::EaseOutCubic, Easing::EaseOutBack] {
            assert!((e.apply(0.0) - 0.0).abs() < 1e-5, "{e:?} en 0");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-5, "{e:?} en 1");
            // Fuera de rango se recorta a los extremos.
            assert!((e.apply(-1.0) - 0.0).abs() < 1e-5);
            assert!((e.apply(2.0) - 1.0).abs() < 1e-5);
        }
        // EaseOut adelanta el progreso a mitad de camino (frena al final).
        assert!(Easing::EaseOutCubic.apply(0.5) > 0.5);
        // EaseOutBack sobre-impulsa por encima de 1 antes de asentar.
        assert!(Easing::EaseOutBack.apply(0.8) > 1.0);
        // Linear es la identidad.
        assert!((Easing::Linear.apply(0.37) - 0.37).abs() < 1e-5);
    }

    #[test]
    fn easing_slug_round_trip() {
        for e in [Easing::Linear, Easing::EaseOutCubic, Easing::EaseOutBack] {
            assert_eq!(Easing::from_slug(e.slug()), Some(e));
        }
        assert_eq!(Easing::from_slug("nope"), None);
    }

    #[test]
    fn overview_geometry_moved_clampa_y_swapea() {
        // 4 escritorios → grilla 2×2 automática: 0=(0,0) 1=(1,0) 2=(0,1) 3=(1,1).
        let c = Config::default();
        // Mover el 0 a la derecha → cae sobre el 1 → intercambian.
        let g = c.overview_geometry_moved(4, 0, 1, 0);
        assert_eq!(g[0], (1, 0));
        assert_eq!(g[1], (0, 0), "el ocupante toma la celda vieja del 0");
        // Mover el 0 a la izquierda desde (0,0) → clampa, no cambia nada.
        let g = c.overview_geometry_moved(4, 0, -1, 0);
        assert_eq!(g[0], (0, 0));
        // Mover el 3 a una celda libre (abajo) → sin swap.
        let g = c.overview_geometry_moved(4, 3, 0, 1);
        assert_eq!(g[3], (1, 2));
    }

    #[test]
    fn overview_places_for_deriva_de_la_grilla_si_no_hay_plano_rico() {
        // Sin overview_places: cada celda de la grilla 2×2 → tile 1×1 sin giro.
        let c = Config::default();
        let p = c.overview_places_for(4);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0], OverviewPlace::new(0.0, 0.0, 1.0, 1.0, 0.0));
        assert_eq!(p[1], OverviewPlace::new(1.0, 0.0, 1.0, 1.0, 0.0));
        assert_eq!(p[2], OverviewPlace::new(0.0, 1.0, 1.0, 1.0, 0.0));
        assert_eq!(p[3], OverviewPlace::new(1.0, 1.0, 1.0, 1.0, 0.0));
    }

    #[test]
    fn overview_places_for_respeta_el_plano_rico_y_lo_adapta_al_conteo() {
        // Con un plano rico de N entradas (posición libre + giro), tiene
        // prioridad sobre la grilla. Si el conteo cambia, se RESPETA el arreglo
        // guardado: se completa con grilla lo que falte y se trunca lo que sobre
        // (en vez de tirar posición+rotación a la basura, que reordenaba el mapa).
        let mut c = Config::default();
        c.overview_places = vec![
            OverviewPlace::new(0.5, 0.0, 1.0, 1.0, std::f32::consts::FRAC_PI_4),
            OverviewPlace::new(2.0, 1.3, 1.5, 1.0, -0.2),
        ];
        // Conteo exacto → el plano rico tal cual.
        assert_eq!(c.overview_places_for(2), c.overview_places);
        // Conteo MAYOR → conserva los 2 guardados (con su giro) + completa el 3º
        // con grilla por defecto (sin giro).
        let p3 = c.overview_places_for(3);
        assert_eq!(p3.len(), 3);
        assert_eq!(p3[0], c.overview_places[0]);
        assert_eq!(p3[1], c.overview_places[1]);
        assert_eq!(p3[2].w, 1.0);
        assert_eq!(p3[2].rot, 0.0);
        // Conteo MENOR → trunca conservando los primeros (con su giro).
        let p1 = c.overview_places_for(1);
        assert_eq!(p1, vec![c.overview_places[0]]);
    }

    #[test]
    fn overview_places_round_trip_por_ron() {
        let mut c = Config::default();
        c.overview_places = vec![
            OverviewPlace::new(0.0, 0.0, 1.0, 1.0, 0.0),
            OverviewPlace::new(1.25, 0.5, 1.0, 1.0, 0.7853982),
        ];
        let text = c.to_ron().expect("serializa");
        let back = Config::from_ron(&text).expect("re-parsea");
        assert_eq!(back.overview_places, c.overview_places);
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

    #[test]
    fn startup_local_se_lanza_tal_cual() {
        let a = StartupApp { command: "foot".into(), ..Default::default() };
        assert_eq!(a.shell_command(), "foot");
    }

    #[test]
    fn startup_remoto_se_envuelve_en_waypipe() {
        let a = StartupApp {
            command: "foot --title diario".into(),
            remote: "sergio@servidor".into(),
            ..Default::default()
        };
        assert_eq!(a.shell_command(), "waypipe ssh sergio@servidor foot --title diario");
    }

    #[test]
    fn startup_remoto_con_afinado_y_opciones_ssh() {
        let a = StartupApp {
            command: "foot".into(),
            remote: "sergio@servidor".into(),
            ssh_port: 2222,
            ssh_key: "~/.ssh/id_ed25519".into(),
            compress: "zstd".into(),
            video: true,
            threads: 4,
            ..Default::default()
        };
        // Afinado (global) ANTES de `ssh`; opciones ssh entre `ssh` y el host.
        assert_eq!(
            a.shell_command(),
            "waypipe --compress=zstd --video --threads=4 ssh -p 2222 -i ~/.ssh/id_ed25519 sergio@servidor foot"
        );
    }

    #[test]
    fn waypipe_puerto_22_y_clave_vacia_se_omiten() {
        // El puerto por defecto (0 o 22) y la clave vacía no agregan banderas.
        let cmd = waypipe_command(&WaypipeTuning::default(), 22, "  ", "host", "foot");
        assert_eq!(cmd, "waypipe ssh host foot");
    }

    #[test]
    fn waypipe_tuning_flags_respeta_orden_y_omite_vacios() {
        assert!(WaypipeTuning::default().flags().is_empty());
        let t = WaypipeTuning { compress: "lz4".into(), video: false, threads: 0 };
        assert_eq!(t.flags(), vec!["--compress=lz4"]);
    }

    #[test]
    fn startup_pin_a_escritorio_produce_una_regla() {
        let a = StartupApp {
            command: "foot".into(),
            remote: "host".into(),
            workspace: 3,
            app_id: "foot".into(),
            ..Default::default()
        };
        let r = a.placement_rule().expect("con app_id + workspace hay regla");
        // 3 (1-based) se resuelve a índice 2 por el mismo camino que rules.ron.
        assert_eq!(r.workspace, 3);
        let outcome = crate::rules::Rules::new(vec![r]).resolve("foot", "");
        assert_eq!(outcome.workspace, Some(2));
    }

    #[test]
    fn startup_sin_app_id_no_produce_regla() {
        // Pin sin app_id no se puede ubicar: nada de regla (sólo se lanza).
        let a = StartupApp { command: "foot".into(), workspace: 3, ..Default::default() };
        assert!(a.placement_rule().is_none());
    }

    #[test]
    fn startup_sin_pin_no_produce_regla() {
        let a = StartupApp { command: "foot".into(), app_id: "foot".into(), ..Default::default() };
        assert!(a.placement_rule().is_none());
    }

    #[test]
    fn startup_round_trip_por_ron() {
        let c = Config::from_ron(
            r#"( startup: [
                (command: "foot", remote: "sergio@servidor", app_id: "foot", workspace: 3),
                (command: "mpv", app_id: "mpv", fullscreen: true),
            ] )"#,
        )
        .unwrap();
        assert_eq!(c.startup().len(), 2);
        assert_eq!(c.startup()[0].shell_command(), "waypipe ssh sergio@servidor foot");
        // Sólo la primera fija escritorio; ambas fijan ubicación → 2 reglas.
        assert_eq!(c.startup_rules().len(), 2);
        let text = c.to_ron().expect("serializa");
        assert_eq!(Config::from_ron(&text).unwrap(), c);
    }
}
