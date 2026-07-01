//! El esquema declarativo de `pata`.
//!
//! El archivo de config manda; el código no asume superficies ni widgets que
//! el config no nombre. Un [`Config`] es **una lista de [`Surface`]s** —no un
//! único panel—: el usuario despliega tantas barras, paneles y docks como
//! quiera, ancla cada uno a un borde, y reparte los widgets en sus slots
//! (`start` / `center` / `end`) con total libertad.
//!
//! El modelo es agnóstico del formato en disco: en Linux un loader TOML
//! deserializa directo a estos tipos (vía `serde`); en wawa el config llega por
//! akasha. Los valores de propiedad de cada widget se guardan como [`Prop`],
//! un enum cerrado y `no_std` —ni `toml::Value` ni nada atado a una
//! plataforma—.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// El borde de la pantalla al que se ancla una superficie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Anchor {
    /// Pegada al borde superior — la posición clásica de una barra de menú.
    #[default]
    Top,
    /// Pegada al borde inferior — donde suele ir el dock o el shell.
    Bottom,
    /// Pegada al borde izquierdo (superficie vertical).
    Left,
    /// Pegada al borde derecho (superficie vertical).
    Right,
}

impl Anchor {
    /// `true` si la superficie se extiende horizontalmente (top/bottom): su
    /// grosor es alto y sus slots se reparten en X. `false` para left/right.
    pub fn es_horizontal(&self) -> bool {
        matches!(self, Anchor::Top | Anchor::Bottom)
    }

    /// Si una superficie anclada a este borde **crece hacia el interior de la
    /// pantalla en sentido contrario al borde**: una barra abajo que se agranda
    /// (al abrir su menú) gana alto hacia arriba, así que su contenido fijo (la
    /// barra) debe quedar al final y el desplegable arriba. Idem `Right`.
    pub fn crece_hacia_el_borde_inicial(&self) -> bool {
        matches!(self, Anchor::Bottom | Anchor::Right)
    }
}

/// Qué clase de superficie del marco se está describiendo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum SurfaceKind {
    /// Barra fina pegada a un borde (estilo waybar/eww): tres slots
    /// (`start` / `center` / `end`) con widgets en línea. Reserva su franja al
    /// compositor para que las ventanas no la tapen.
    #[default]
    Bar,
    /// Panel: un área donde flotan [`FloatingCard`]s posicionadas en píxeles
    /// (estilo conky). No reserva franja; vive sobre el escritorio.
    Panel,
    /// Dock: lanzadores y/o ventanas abiertas, centrado en su borde y del
    /// tamaño justo de su contenido. Puede autoesconderse.
    Dock,
    /// Sidebar acoplable: un **rail de dientes** vertical pegado a un borde
    /// (left/right). Cada diente ([`SidebarTab`]) despliega un panel con su
    /// widget de contenido (lógica de launcher: colapsado = sólo el rail,
    /// activar un diente despliega su panel sobre el escritorio). El rail
    /// reserva su grosor como una barra; si `autohide`, no reserva y reaparece
    /// al rozar el borde. El panel desplegado flota, no reserva.
    Sidebar,
    /// Fondo de escritorio: una superficie a **pantalla completa** que vive
    /// DETRÁS de las ventanas (capa Background, sin zona exclusiva). Su(s)
    /// widget(s) llenan la pantalla. Punto de extensión para fondos con
    /// contenido; no reserva espacio ni roba foco.
    Background,
}

/// El valor de una propiedad de widget, agnóstico del formato en disco. El
/// loader de cada plataforma (TOML, RON, akasha) deserializa a esto; los
/// widgets lo leen con los helpers de [`WidgetSpec`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Prop {
    /// Booleano (`true` / `false`).
    Bool(bool),
    /// Entero — se prueba antes que [`Prop::Num`] para no perder la
    /// distinción int/float de formatos como TOML.
    Int(i64),
    /// Número de punto flotante.
    Num(f64),
    /// Cadena de texto.
    Str(String),
}

impl Prop {
    /// La prop como `&str`, si lo es.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }

    /// La prop como número: un [`Prop::Int`] se promueve a `f64`.
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Prop::Num(n) => Some(*n),
            Prop::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// La prop como booleano, si lo es.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Prop::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

/// Una entrada de widget: `kind` (qué widget) + props arbitrarias. El conjunto
/// de `kind`s es **abierto** —el frontend despacha por string y cae a un
/// placeholder si no lo conoce—, así que agregar un widget no toca el core.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct WidgetSpec {
    /// El identificador del widget builtin: `"clock"`, `"volume"`,
    /// `"astro"`, `"start_button"`, `"window_list"`, `"tray"`,
    /// `"shuma_input"`, …
    pub kind: String,
    /// Props que cada widget interpreta a su gusto (formato del reloj, hotkey
    /// del quake, etc.). Las claves no reconocidas se conservan.
    #[cfg_attr(feature = "serde", serde(default, flatten))]
    pub props: BTreeMap<String, Prop>,
}

impl WidgetSpec {
    /// Un widget sin props.
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            props: BTreeMap::new(),
        }
    }

    /// El mismo widget con una prop añadida (encadenable).
    pub fn with(mut self, key: impl Into<String>, value: Prop) -> Self {
        self.props.insert(key.into(), value);
        self
    }

    /// Lee una prop string. Devuelve `default` si falta o no es string.
    pub fn str_prop<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.props.get(key).and_then(Prop::as_str).unwrap_or(default)
    }

    /// Lee una prop numérica. Devuelve `default` si falta o no es número.
    pub fn num_prop(&self, key: &str, default: f64) -> f64 {
        self.props.get(key).and_then(Prop::as_num).unwrap_or(default)
    }

    /// Lee una prop booleana. Devuelve `default` si falta o no es booleana.
    pub fn bool_prop(&self, key: &str, default: bool) -> bool {
        self.props.get(key).and_then(Prop::as_bool).unwrap_or(default)
    }
}

/// Una entrada del **catálogo de módulos/widgets**: qué widget es, cómo se
/// llama/icono para el compositor, y en qué superficies puede MONTARSE. Es la
/// pieza que faltaba para "armar una fila de widgets": el panel de control lista
/// el catálogo filtrado por el tipo de superficie y deja agregar el elegido.
///
/// Un "módulo" como shuma aporta DOS entradas: `shuma_input` (widget chico para
/// una barra) y `shuma` (contenido rico para un diente de sidebar). Las barras
/// llevan widgets en línea (chicos); los dientes de sidebar llevan contenidos
/// con más capacidad (navegador, shell, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidgetCatalogEntry {
    /// El `kind` que va al [`WidgetSpec`].
    pub kind: &'static str,
    /// Nombre legible para el compositor.
    pub label: &'static str,
    /// Glifo/icono para el compositor.
    pub icon: &'static str,
    /// Puede ir como widget EN LÍNEA en una barra o dock.
    pub on_bar: bool,
    /// Puede ir como CONTENIDO de un diente de sidebar (panel desplegado).
    pub on_sidebar: bool,
}

/// El catálogo de widgets/módulos montables. Conjunto abierto: el frontend cae a
/// un placeholder si no conoce un `kind`, así que esto sólo guía al compositor.
/// `shuma` aparece dos veces (barra: el input; sidebar: el shell completo) — es
/// el caso que motivó el catálogo: shuma se conecta a pata COMO widget.
pub fn widget_catalog() -> &'static [WidgetCatalogEntry] {
    use WidgetCatalogEntry as W;
    &[
        // --- chicos, de barra (estado / control en línea) ---
        W { kind: "clock", label: "Reloj", icon: "◷", on_bar: true, on_sidebar: false },
        W { kind: "workspaces", label: "Escritorios", icon: "▦", on_bar: true, on_sidebar: false },
        W { kind: "keyboard_layout", label: "Distribución de teclado", icon: "⌨", on_bar: true, on_sidebar: false },
        W { kind: "window_list", label: "Ventanas (taskbar)", icon: "▭", on_bar: true, on_sidebar: false },
        W { kind: "control", label: "Control (volumen/brillo)", icon: "🔊", on_bar: true, on_sidebar: false },
        W { kind: "cava", label: "Visualizador (cava)", icon: "♪", on_bar: true, on_sidebar: false },
        W { kind: "tray", label: "Bandeja (tray)", icon: "▽", on_bar: true, on_sidebar: false },
        W { kind: "clipboard", label: "Portapapeles", icon: "❏", on_bar: true, on_sidebar: false },
        W { kind: "weather", label: "Clima", icon: "☁", on_bar: true, on_sidebar: false },
        W { kind: "astro", label: "Astro", icon: "✶", on_bar: true, on_sidebar: false },
        W { kind: "start_button", label: "Botón inicio", icon: "◉", on_bar: true, on_sidebar: false },
        // --- shuma: módulo que se conecta a pata como widget ---
        W { kind: "shuma_input", label: "Shuma (barra)", icon: "❯", on_bar: true, on_sidebar: false },
        W { kind: "shuma", label: "Shuma (shell completo)", icon: "❯", on_bar: false, on_sidebar: true },
        // --- ricos, de sidebar (paneles con más capacidad) ---
        W { kind: "navigator", label: "Navegador de archivos", icon: "❖", on_bar: false, on_sidebar: true },
        W { kind: "search", label: "Buscar", icon: "🔍", on_bar: false, on_sidebar: true },
        W { kind: "rag", label: "Correo IA (RAG)", icon: "✨", on_bar: false, on_sidebar: true },
    ]
}

/// El subconjunto del catálogo montable en una superficie de este `kind`.
pub fn widgets_for_surface(kind: SurfaceKind) -> Vec<WidgetCatalogEntry> {
    widget_catalog()
        .iter()
        .copied()
        .filter(|w| match kind {
            SurfaceKind::Bar | SurfaceKind::Dock => w.on_bar,
            SurfaceKind::Sidebar => w.on_sidebar,
            // Panel (cards flotantes) y Background aceptan cualquiera por ahora.
            SurfaceKind::Panel | SurfaceKind::Background => true,
        })
        .collect()
}

/// Una tarjeta posicionada en píxeles dentro de un panel (estilo conky).
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FloatingCard {
    /// Origen X en píxeles desde la esquina superior-izquierda del panel.
    pub x: f32,
    /// Origen Y en píxeles.
    pub y: f32,
    /// Ancho en píxeles.
    pub w: f32,
    /// Alto en píxeles.
    pub h: f32,
    /// Título opcional (chip arriba a la izquierda).
    #[cfg_attr(feature = "serde", serde(default))]
    pub title: Option<String>,
    /// Widgets apilados dentro de la tarjeta.
    #[cfg_attr(feature = "serde", serde(default))]
    pub widgets: Vec<WidgetSpec>,
}

/// Un diente de un [`SurfaceKind::Sidebar`]: una pestaña vertical del rail que,
/// al activarse, despliega un panel con su widget de contenido.
///
/// El `icon` es un identificador que el frontend mapea a su glifo/dibujo (igual
/// que el `kind` de un widget: conjunto abierto, cae a un default si no lo
/// conoce). El `label` rotula el panel desplegado y el tooltip del diente. El
/// `content` es un [`WidgetSpec`] —típicamente `kind = "navigator"`— que el
/// frontend pinta en el panel; el modelo no asume qué es.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SidebarTab {
    /// Identificador del icono del diente (`"files"`, `"monads"`, `"search"`…).
    #[cfg_attr(feature = "serde", serde(default))]
    pub icon: String,
    /// Rótulo del panel desplegado y tooltip del diente.
    #[cfg_attr(feature = "serde", serde(default))]
    pub label: String,
    /// El widget que se pinta en el panel cuando el diente está activo.
    #[cfg_attr(feature = "serde", serde(default))]
    pub content: WidgetSpec,
}

impl SidebarTab {
    /// Un diente con icono, rótulo y widget de contenido.
    pub fn new(icon: impl Into<String>, label: impl Into<String>, content: WidgetSpec) -> Self {
        Self {
            icon: icon.into(),
            label: label.into(),
            content,
        }
    }
}

/// Una superficie del marco: una barra, un panel, un dock o un sidebar anclado
/// a un borde.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Surface {
    /// Nombre legible de la barra (para el panel de control). Vacío = sin
    /// nombre; el panel cae a un rótulo derivado del tipo/borde.
    #[cfg_attr(feature = "serde", serde(default))]
    pub name: String,
    /// Si `false`, la superficie NO se crea (apagada). El panel la deja en la
    /// lista pero el backend la omite. Default `true` (encendida).
    #[cfg_attr(feature = "serde", serde(default = "default_enabled"))]
    pub enabled: bool,
    /// Bar, Panel o Dock.
    #[cfg_attr(feature = "serde", serde(default))]
    pub kind: SurfaceKind,
    /// A qué borde se ancla.
    #[cfg_attr(feature = "serde", serde(default))]
    pub anchor: Anchor,
    /// Grosor en píxeles: alto para superficies horizontales (top/bottom),
    /// ancho para verticales (left/right). Un dock puede ignorarlo y crecer
    /// con su contenido.
    #[cfg_attr(feature = "serde", serde(default = "default_thickness"))]
    pub thickness: f32,
    /// Si `true`, la superficie se esconde y reaparece al hover/hotkey. El
    /// shell (`shuma_input`) vive típicamente en una barra inferior con esto.
    #[cfg_attr(feature = "serde", serde(default))]
    pub autohide: bool,
    /// **Eje DOCKED** (`kind = sidebar`): ¿reserva su franja del escritorio
    /// («supeditada al desktop», las ventanas no la tapan, `exclusive_zone`) o
    /// flota como overlay encima? `None` = sigue el global `sidebar_docked`;
    /// `Some(true)` = reserva siempre; `Some(false)` = flota siempre. Es
    /// INDEPENDIENTE de [`Self::rail_outside`] (posición del rail). Permite tener
    /// UN sidebar (p. ej. el derecho) acoplado al desktop sin tocar el resto.
    #[cfg_attr(feature = "serde", serde(default))]
    pub reserve: Option<bool>,
    /// **Eje POSICIÓN del rail** (`kind = sidebar`), puramente visual: `None` =
    /// sigue el global `dientes_outside`; `Some(false)` = rail DENTRO (overlay
    /// sobre el panel, estilo cosmos); `Some(true)` = rail FUERA (sobresale como
    /// franja al costado del panel). No afecta la reserva de espacio (eso es
    /// [`Self::reserve`] / `sidebar_docked`).
    #[cfg_attr(feature = "serde", serde(default))]
    pub rail_outside: Option<bool>,
    /// Padding interno (px) entre el borde y el primer widget.
    #[cfg_attr(feature = "serde", serde(default = "default_padding"))]
    pub padding: f32,
    /// Separación (px) entre widgets adyacentes.
    #[cfg_attr(feature = "serde", serde(default = "default_gap"))]
    pub gap: f32,
    /// Opacidad del fondo de la superficie `0.0..=1.0` (default `1.0` = opaco).
    /// El frontend la aplica al color de fondo de la barra — una barra
    /// translúcida que deja ver el escritorio detrás. Sólo afecta el pincel; la
    /// reserva de franja no cambia.
    #[cfg_attr(feature = "serde", serde(default = "default_opacity"))]
    pub opacity: f32,
    /// Radio de las esquinas del fondo de la superficie (px, default `0.0` =
    /// rectas). Con [`Surface::margin`] > 0 da el look de barra flotante con
    /// esquinas redondeadas.
    #[cfg_attr(feature = "serde", serde(default))]
    pub radius: f32,
    /// Margen (px) entre la barra y el borde de pantalla — el look de barra
    /// "flotante". Default `0.0` = pegada al borde. Sólo es pincel: la reserva
    /// de franja sigue siendo `thickness` (las ventanas no entran en el margen).
    #[cfg_attr(feature = "serde", serde(default))]
    pub margin: f32,
    /// Si `true`, el fondo de la barra se pinta con un degradé vertical sutil
    /// (claro arriba → oscuro abajo) en vez de un color plano. Embellecimiento.
    #[cfg_attr(feature = "serde", serde(default))]
    pub gradient: bool,
    /// Unidad de **cuantización de ancho** (px). Si `> 0`, cada widget reserva
    /// un ancho múltiplo de `cell` (mínimo), así el racimo de indicadores queda
    /// alineado a una grilla en vez de bailar con cada cambio de dígitos.
    /// Default `0.0` = ancho automático (sin grilla). Un widget puede pedir N
    /// celdas con la prop `cells`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub cell: f32,
    /// Slot inicial: pegado al inicio del eje (izquierda / arriba).
    #[cfg_attr(feature = "serde", serde(default))]
    pub start: Vec<WidgetSpec>,
    /// Slot central: centrado en el eje.
    #[cfg_attr(feature = "serde", serde(default))]
    pub center: Vec<WidgetSpec>,
    /// Slot final: pegado al final del eje (derecha / abajo).
    #[cfg_attr(feature = "serde", serde(default))]
    pub end: Vec<WidgetSpec>,
    /// Para `kind = panel`: las tarjetas flotantes que contiene.
    #[cfg_attr(feature = "serde", serde(default))]
    pub cards: Vec<FloatingCard>,
    /// Monitor al que anclar la superficie. **Default `"*"` = la superficie se
    /// replica en CADA monitor conectado** (una barra por pantalla). Un nombre
    /// de conector (`"HDMI-A-1"`, `"DP-1"`) la fija a ese monitor; `""` (vacío)
    /// la deja en el primario que elija el compositor. El backend
    /// `wlr-layer-shell` pasa este `wl_output` a `create_layer_surface`; si el
    /// nombre no matchea ninguno conectado, cae al primario y se loguea un aviso.
    /// Para excluir monitores puntuales del `"*"`, ver [`Surface::exclude_outputs`].
    #[cfg_attr(feature = "serde", serde(default = "default_output"))]
    pub output: String,
    /// Monitores a **excluir** cuando `output = "*"` (nombres de conector). La
    /// barra aparece en todos los monitores conectados MENOS estos. Vacío = en
    /// todos. Sólo tiene efecto con `output = "*"`/`"all"`; con un monitor fijo
    /// o el primario se ignora. Permite el patrón «todas las pantallas salvo la
    /// del proyector / la secundaria».
    #[cfg_attr(feature = "serde", serde(default))]
    pub exclude_outputs: Vec<String>,
    /// Para `kind = sidebar`: los dientes del rail. Cada uno despliega su panel.
    #[cfg_attr(feature = "serde", serde(default))]
    pub tabs: Vec<SidebarTab>,
    /// Para `kind = sidebar`: ancho (px) del panel que despliega un diente. El
    /// rail mismo usa `thickness`; el panel flota a su lado con este ancho.
    #[cfg_attr(feature = "serde", serde(default = "default_panel_width"))]
    pub panel_width: f32,
    /// Para `kind = dock`: apps **fijadas** por id/nombre (`app_bus::AppEntry::id`).
    /// El frontend las pinta como lanzadores antes de las ventanas abiertas; al
    /// click lanzan la app. Vacío = sólo ventanas abiertas. Las que no resuelvan
    /// en el registro se omiten.
    #[cfg_attr(feature = "serde", serde(default))]
    pub dock_pins: Vec<String>,
}

impl Default for Surface {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            kind: SurfaceKind::default(),
            anchor: Anchor::default(),
            thickness: default_thickness(),
            autohide: false,
            reserve: None,
            rail_outside: None,
            padding: default_padding(),
            gap: default_gap(),
            opacity: default_opacity(),
            radius: 0.0,
            margin: 0.0,
            gradient: false,
            cell: 0.0,
            start: Vec::new(),
            center: Vec::new(),
            end: Vec::new(),
            cards: Vec::new(),
            output: default_output(),
            exclude_outputs: Vec::new(),
            tabs: Vec::new(),
            panel_width: default_panel_width(),
            dock_pins: Vec::new(),
        }
    }
}

impl Surface {
    /// Una barra anclada a `anchor`, con los slots vacíos.
    pub fn bar(anchor: Anchor) -> Self {
        Self {
            kind: SurfaceKind::Bar,
            anchor,
            ..Self::default()
        }
    }

    /// Un dock anclado a `anchor`.
    pub fn dock(anchor: Anchor) -> Self {
        Self {
            kind: SurfaceKind::Dock,
            anchor,
            ..Self::default()
        }
    }

    /// Un fondo de escritorio a pantalla completa (detrás de las ventanas).
    pub fn background() -> Self {
        Self {
            kind: SurfaceKind::Background,
            anchor: Anchor::Top, // irrelevante: se ancla a los 4 bordes.
            ..Self::default()
        }
    }

    /// Un sidebar acoplable anclado a `anchor` (left/right), con el rail vacío.
    /// El grosor por defecto es el ancho del rail de dientes.
    pub fn sidebar(anchor: Anchor) -> Self {
        Self {
            kind: SurfaceKind::Sidebar,
            anchor,
            thickness: default_rail_thickness(),
            ..Self::default()
        }
    }
}

/// Settings transversales a todas las superficies.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct General {
    /// Zona horaria del reloj. `"auto"` la detecta del sistema; también acepta
    /// nombres IANA. La sincronización NTP no es de pata: la da el SO.
    #[cfg_attr(feature = "serde", serde(default = "default_timezone"))]
    pub timezone: String,
    /// Color de acento global como hex `"#rrggbb"` (o `"#rrggbbaa"`). Vacío =
    /// el del tema. El frontend lo parsea (el core no conoce colores); tiñe
    /// medidores, glifos del botón de inicio y bordes. Embellecer el marco sin
    /// recompilar.
    #[cfg_attr(feature = "serde", serde(default))]
    pub accent: String,
    /// Estilo del menú de inicio/lanzador (el control único, configurable):
    /// `"list"` (clásico sobrio, default), `"xp"` (dos columnas estilo Windows
    /// XP) o `"grid"`/`"gnome"` (grilla de tiles tipo Activities/Kickoff). Lo
    /// fijan las vistas y lo puede tunear el usuario en launcher.toml.
    #[cfg_attr(feature = "serde", serde(default = "default_menu_style"))]
    pub menu_style: String,
    /// Columnas del menú `"grid"` (0 = automático según el ancho).
    #[cfg_attr(feature = "serde", serde(default))]
    pub menu_columns: u32,
    /// **Shuma (drawer del shell):** fracción del alto de pantalla que despliega
    /// el drawer (0.1..0.95, default 0.45).
    #[cfg_attr(feature = "serde", serde(default = "default_shuma_height"))]
    pub shuma_height: f32,
    /// Color de fondo del drawer de shuma como hex `"#rrggbb"`. Vacío = el del
    /// tema.
    #[cfg_attr(feature = "serde", serde(default))]
    pub shuma_bg: String,
    /// Tecla para abrir/cerrar el drawer de shuma (ej. `"F12"`, default
    /// `"Alt+Enter"`). El grab global lo hace el compositor (atajo de mirada).
    #[cfg_attr(feature = "serde", serde(default = "default_shuma_key"))]
    pub shuma_key: String,
    /// Política del idle de energía (suspender/apagar por inactividad). Va
    /// **al final** de `General`: es una tabla y TOML exige las tablas tras los
    /// valores escalares.
    #[cfg_attr(feature = "serde", serde(default))]
    pub energia: EnergiaCfg,
}

impl Default for General {
    fn default() -> Self {
        Self {
            timezone: default_timezone(),
            accent: String::new(),
            menu_style: default_menu_style(),
            menu_columns: 0,
            shuma_height: default_shuma_height(),
            shuma_bg: String::new(),
            shuma_key: default_shuma_key(),
            energia: EnergiaCfg::default(),
        }
    }
}

/// Política del **idle de energía**: suspender (o apagar) por inactividad sin
/// cortar trabajo importante. El frontend (`pata-llimphi`) la consulta contra
/// el plano de control (sandokan — unidades ocupadas o keep-awake por label) y
/// la carga del sistema antes de actuar; si algo trabaja, **pospone** en vez de
/// cortar. Defaults seguros: sólo con batería, 15 min, apagado automático off.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct EnergiaCfg {
    /// Maestro: si está en `false`, el idle de energía no hace nada.
    #[cfg_attr(feature = "serde", serde(default = "default_true"))]
    pub habilitado: bool,
    /// Segundos de inactividad para **suspender** (`0` = nunca).
    #[cfg_attr(feature = "serde", serde(default = "default_suspender_secs"))]
    pub suspender_secs: u32,
    /// Segundos de inactividad para **apagar** (`0` = nunca; sólo tiene sentido
    /// si es mayor que `suspender_secs`).
    #[cfg_attr(feature = "serde", serde(default))]
    pub apagar_secs: u32,
    /// Sólo actuar con batería: en AC (o escritorio) no suspende solo.
    #[cfg_attr(feature = "serde", serde(default = "default_true"))]
    pub solo_con_bateria: bool,
    /// %CPU por unidad que cuenta como ocupada (veto del plano de control). Nota:
    /// vía arje-bus la telemetría no trae CPU (queda en 0), así que este umbral
    /// pesa con engines que sí la miden; el grueso del «ocupado» lo da la carga.
    #[cfg_attr(feature = "serde", serde(default = "default_cpu_ocupada_pct"))]
    pub cpu_ocupada_pct: f64,
    /// Carga (loadavg 1m) **por core** sobre la cual el sistema se considera
    /// ocupado por procesos que no son unidades gestionadas (un `cargo build`…).
    #[cfg_attr(feature = "serde", serde(default = "default_carga_por_core"))]
    pub carga_ocupada_por_core: f64,
    /// Subcadenas de **label de unidad** (sandokan) a mantener despiertas: si una
    /// unidad cuyo label las contiene corre, no se suspende (backups, sync,
    /// transcodificación…). Es la coordinación explícita con el plano de control,
    /// y funciona vía arje-bus (los labels llegan en `list()`).
    #[cfg_attr(feature = "serde", serde(default))]
    pub etiquetas_despiertas: Vec<String>,
}

impl Default for EnergiaCfg {
    fn default() -> Self {
        Self {
            habilitado: true,
            suspender_secs: default_suspender_secs(),
            apagar_secs: 0,
            solo_con_bateria: true,
            cpu_ocupada_pct: default_cpu_ocupada_pct(),
            carga_ocupada_por_core: default_carga_por_core(),
            etiquetas_despiertas: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_suspender_secs() -> u32 {
    900
}
fn default_cpu_ocupada_pct() -> f64 {
    25.0
}
fn default_carga_por_core() -> f64 {
    0.7
}

fn default_shuma_height() -> f32 {
    0.45
}
fn default_shuma_key() -> String {
    "Alt+Enter".to_string()
}

/// Estilo de menú por defecto: la lista sobria.
fn default_menu_style() -> String {
    "list".to_string()
}

/// Monitor por defecto de una superficie: `"*"` = TODOS los monitores
/// conectados (una barra por pantalla). Para fijarla a uno solo, poné el
/// nombre del conector; para excluir algunos del `"*"`, usá `exclude_outputs`.
fn default_output() -> String {
    "*".to_string()
}

/// El marco completo: settings generales + las superficies a desplegar.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Config {
    #[cfg_attr(feature = "serde", serde(default))]
    pub general: General,
    /// Las superficies del escritorio, en orden de declaración.
    #[cfg_attr(feature = "serde", serde(default))]
    pub surfaces: Vec<Surface>,
}

impl Config {
    /// El marco por defecto cuando no hay config: una barra superior con el
    /// botón de inicio y el reloj a la izquierda, la lista de ventanas al
    /// centro, y el racimo de indicadores (astro, clipboard, volumen, brillo,
    /// tray, medidores) más el input del shell a la derecha; y un dock inferior
    /// autoescondible. Referencia widgets que aún no existen como builtin —el
    /// frontend cae a un placeholder— para encodear la visión completa.
    pub fn preset() -> Self {
        // Barra superior: thickness 44 para que los medidores Medium verticales
        // (~38 px de barrita + caption + padding) entren cómodos. Antes eran
        // 32 y los chips verticales no entraban — el frontend acababa
        // mostrándolos achicados o cayendo a horizontal.
        let mut top = Surface::bar(Anchor::Top);
        top.thickness = 44.0;
        // `output` ya viene en `"*"` por default (todas las pantallas); para
        // dejarla sólo en una, fijá `top.output` al conector o usá
        // `top.exclude_outputs` para sacarla de monitores puntuales.
        top.start = vec![
            WidgetSpec::new("start_button"),
            WidgetSpec::new("clock"),
            // Visualizador de audio (CAVA) en el default — el espectro vivo.
            WidgetSpec::new("cava"),
        ];
        top.center = vec![
            WidgetSpec::new("workspace_switcher"),
            WidgetSpec::new("window_list"),
        ];
        // Medidores con tamaño + orientación EXPLÍCITOS. Antes el default
        // global era horizontal, así que cualquier widget sin `orientation` en
        // su spec acababa horizontal aunque el size fuera small. Ahora los
        // fijamos vertical-medium para que se vean como columnas visibles.
        let meter_v = |kind: &str| {
            WidgetSpec::new(kind)
                .with("size", Prop::Str("medium".to_string()))
                .with("orientation", Prop::Str("vertical".to_string()))
        };
        // CPU/RAM ya no van en la barra: su lugar es el diente «Sistema» del
        // sidebar derecho (monitor de sistema). Los widgets `cpu_meter`/`ram_meter`
        // siguen en el catálogo —quien quiera puede ponerlos en su barra—; sólo
        // salen del default de mirada.
        top.end = vec![
            WidgetSpec::new("astro"),
            WidgetSpec::new("moon"),
            WidgetSpec::new("clipboard"),
            meter_v("volume"),
            meter_v("brightness"),
            WidgetSpec::new("control"),
            WidgetSpec::new("tray"),
        ];

        // Sidebar izquierdo con dientes default (rail acoplable estilo
        // VSCode/Slack). Antes el preset solo traía top + shell — el panel
        // global de pestañas no aparecía hasta que el usuario lo escribiera
        // a mano en el TOML.
        let mut rail = Surface::sidebar(Anchor::Left);
        // Docked y posición del rail quedan en `None`: siguen los globales
        // (`sidebar_docked` default true = reservan franja → SIEMPRE visibles;
        // `dientes_outside` default false = rail overlay). Antes el preset ponía
        // `reserve = Some(true)` HARDCODEADO, que pisaba el global y por eso
        // cambiar la config «no cambiaba nada». Los switches por-sidebar
        // (`reserve` / `rail_outside`) o los globales ahora sí deciden.
        rail.tabs.push(SidebarTab::new(
            "monads",
            "Mónadas",
            WidgetSpec::new("navigator").with("source", Prop::Str("nouser".to_string())),
        ));
        rail.tabs.push(SidebarTab::new(
            "files",
            "Archivos",
            WidgetSpec::new("navigator").with("source", Prop::Str("home".to_string())),
        ));
        rail.tabs.push(SidebarTab::new(
            "rag",
            "Correo IA",
            WidgetSpec::new("rag").with("source", Prop::Str("paloma".to_string())),
        ));
        // Búsqueda en lenguaje natural sobre el centro de eventos (willay):
        // notificaciones, capturas y clipboard. Mismo widget `rag`, otro corpus.
        rail.tabs.push(SidebarTab::new(
            "eventos",
            "Eventos IA",
            WidgetSpec::new("rag").with("source", Prop::Str("willay".to_string())),
        ));

        // Sidebar DERECHO: docked/posición siguen los globales (como el izquierdo).
        // Rail de herramientas a la derecha.
        let mut rrail = Surface::sidebar(Anchor::Right);
        // Ancho generoso: el monitor embebe los panels de CPU/RAM (≈320 px).
        rrail.panel_width = 340.0;
        // Monitor de sistema: CPU (promedio + cores) + RAM. Primer paso del control
        // center de sistema + flota (a futuro: unidades sandokan + flota matilda).
        rrail.tabs.push(SidebarTab::new(
            "monitor",
            "Sistema",
            WidgetSpec::new("monitor"),
        ));
        // Unidades del plano de control (sandokan): estado + telemetría en vivo.
        rrail.tabs.push(SidebarTab::new(
            "unidades",
            "Unidades",
            WidgetSpec::new("unidades"),
        ));
        // Flota baremetal (matilda): inventario read-only de hosts/contenedores.
        rrail.tabs.push(SidebarTab::new(
            "flota",
            "Flota",
            WidgetSpec::new("flota"),
        ));
        rrail.tabs.push(SidebarTab::new(
            "rag",
            "Correo IA",
            WidgetSpec::new("rag").with("source", Prop::Str("paloma".to_string())),
        ));
        rrail.tabs.push(SidebarTab::new(
            "files",
            "Archivos",
            WidgetSpec::new("navigator").with("source", Prop::Str("home".to_string())),
        ));

        let mut shell = Surface::bar(Anchor::Bottom);
        shell.autohide = true;
        shell.thickness = 40.0;
        shell.center = vec![
            WidgetSpec::new("shuma_input").with("hotkey", Prop::Str("F12".to_string()))
        ];

        // **Glass**: el preset nativo nace con el chrome **translúcido** (la
        // superficie ya clarea transparente y aplica `opacity` al fondo). Combinado
        // con el frosted backdrop que el compositor pone DETRÁS de los paneles
        // layer-shell (sólo con el glass del theme «mirada» encendido), da el
        // efecto cristal. El resto de los presets (dwm/mac/…) quedan en `1.0`
        // (opaco) — así el glass es sólo de «mirada», igual que en mirada-brain.
        const GLASS_OPACITY: f32 = 0.62;
        top.opacity = GLASS_OPACITY;
        rail.opacity = GLASS_OPACITY;
        rrail.opacity = GLASS_OPACITY;
        shell.opacity = GLASS_OPACITY;

        Self {
            general: General::default(),
            surfaces: vec![top, rail, rrail, shell],
        }
    }

    /// Preset de **barra** para una vista de escritorio de mirada. Los slugs
    /// casan 1:1 con las vistas de `mirada-brain::Vista` (`"dwm"`, `"hyprland"`,
    /// `"windows-xp"`, `"mac"`, `"kde"`, `"mirada"`). Al aplicar una vista, el
    /// panel escribe este `Config` con [`crate`]`-config::save` y pata recarga en
    /// caliente — así la barra acompaña al look. `mirada` = el preset nativo.
    /// `None` si el slug no existe.
    pub fn vista_preset(name: &str) -> Option<Self> {
        Some(match name {
            "mirada" => Self::preset(),
            "dwm" => Self::vista_dwm(),
            "hyprland" => Self::vista_hyprland(),
            "windows-xp" => Self::vista_windows_xp(),
            "windows-3.1" => Self::vista_windows_31(),
            "mac" => Self::vista_mac(),
            "kde" => Self::vista_kde(),
            "solaris" => Self::vista_solaris(),
            _ => return None,
        })
    }

    /// Barra **dwm**: una franja fina arriba — tags + símbolo de layout a la
    /// izquierda, título de la enfocada al centro, reloj a la derecha.
    fn vista_dwm() -> Self {
        let mut bar = Surface::bar(Anchor::Top);
        bar.thickness = 22.0;
        bar.gap = 8.0;
        bar.padding = 6.0;
        bar.start = vec![
            WidgetSpec::new("workspaces"),
            WidgetSpec::new("layout"),
        ];
        bar.center = vec![WidgetSpec::new("window_title").with("max", Prop::Num(70.0))];
        bar.end = vec![
            WidgetSpec::new("keyboard_layout"),
            WidgetSpec::new("clock").with("format", Prop::Str("%a %d %H:%M".to_string())),
        ];
        Self {
            general: General::default(),
            surfaces: vec![bar],
        }
    }

    /// Barra **Hyprland**: top con aire (gap/radius), tags + título + un grupo de
    /// estado (reloj, volumen, CPU) — al estilo waybar minimalista.
    fn vista_hyprland() -> Self {
        let mut bar = Surface::bar(Anchor::Top);
        bar.thickness = 34.0;
        bar.gap = 14.0;
        bar.padding = 10.0;
        bar.radius = 10.0;
        bar.margin = 6.0;
        bar.gradient = true;
        bar.start = vec![WidgetSpec::new("workspaces"), WidgetSpec::new("layout")];
        bar.center = vec![WidgetSpec::new("window_title").with("max", Prop::Num(80.0))];
        bar.end = vec![
            WidgetSpec::new("keyboard_layout"),
            WidgetSpec::new("volume"),
            WidgetSpec::new("cpu_meter"),
            WidgetSpec::new("clock"),
        ];
        Self {
            general: General::default(),
            surfaces: vec![bar],
        }
    }

    /// Barra **Windows XP**: taskbar abajo — botón Inicio, lista de ventanas
    /// (botones de tarea) al centro, bandeja + reloj a la derecha.
    fn vista_windows_xp() -> Self {
        let mut bar = Surface::bar(Anchor::Bottom);
        bar.thickness = 40.0;
        bar.gradient = true;
        bar.start = vec![WidgetSpec::new("start_button").with("label", Prop::Str("Inicio".to_string()))];
        bar.center = vec![WidgetSpec::new("window_list")];
        bar.end = vec![
            WidgetSpec::new("tray"),
            WidgetSpec::new("volume"),
            WidgetSpec::new("clock").with("format", Prop::Str("%H:%M".to_string())),
        ];
        Self {
            // Menú Inicio estilo XP (dos columnas).
            general: General {
                menu_style: "xp".to_string(),
                ..General::default()
            },
            surfaces: vec![bar],
        }
    }

    /// Vista **Windows 3.1**: una franja superior gris Motif con menú (Archivo),
    /// lista de ventanas y reloj — la barra de menú del escritorio. El *Program
    /// Manager* ya no lo monta pata: es una app cliente real (`mirada-progman`)
    /// que lanza la vista de mirada como autoexec efímero.
    fn vista_windows_31() -> Self {
        let mut bar = Surface::bar(Anchor::Top);
        bar.thickness = 28.0;
        bar.start = vec![
            WidgetSpec::new("start_button").with("label", Prop::Str("Archivo".to_string())),
            WidgetSpec::new("window_title").with("max", Prop::Num(60.0)),
        ];
        bar.center = vec![WidgetSpec::new("window_list")];
        bar.end = vec![WidgetSpec::new("clock").with("format", Prop::Str("%H:%M".to_string()))];
        Self {
            general: General::default(),
            surfaces: vec![bar],
        }
    }

    /// Vista **macOS**: menubar fina arriba (logo + título + estado) y un dock
    /// abajo con las ventanas abiertas.
    fn vista_mac() -> Self {
        let mut menubar = Surface::bar(Anchor::Top);
        menubar.thickness = 26.0;
        menubar.padding = 10.0;
        menubar.start = vec![
            WidgetSpec::new("start_button").with("label", Prop::Str("\u{f8ff}".to_string())), //  (cae a tofu sin la fuente; inocuo)
            WidgetSpec::new("window_title").with("max", Prop::Num(60.0)),
        ];
        menubar.end = vec![
            WidgetSpec::new("volume"),
            WidgetSpec::new("tray"),
            WidgetSpec::new("clock").with("format", Prop::Str("%a %H:%M".to_string())),
        ];
        let mut dock = Surface::dock(Anchor::Bottom);
        // Alto holgado: los íconos magnificados (base 40 × 1.9 ≈ 76) crecen hacia
        // arriba y deben caber sin cortarse contra el borde de la superficie.
        dock.thickness = 96.0;
        dock.radius = 16.0;
        dock.margin = 8.0;
        dock.gradient = true;
        // Apps fijadas (las que no resuelvan en el registro se omiten) + las
        // ventanas abiertas las agrega el frontend al renderizar el dock.
        dock.dock_pins = vec![
            "nahual".into(),
            "puriy".into(),
            "nada".into(),
            "pluma".into(),
            "media".into(),
        ];
        Self {
            // Launchpad: el menú de mac es la grilla (no la lista classic).
            general: General {
                menu_style: "grid".to_string(),
                ..General::default()
            },
            surfaces: vec![menubar, dock],
        }
    }

    /// Barra **KDE Plasma**: panel abajo — lanzador (Kickoff), lista de ventanas
    /// al centro, bandeja + reloj a la derecha.
    fn vista_kde() -> Self {
        let mut bar = Surface::bar(Anchor::Bottom);
        bar.thickness = 36.0;
        bar.start = vec![WidgetSpec::new("start_button").with("label", Prop::Str("\u{2261}".to_string()))]; // ≡ Kickoff
        bar.center = vec![WidgetSpec::new("window_list")];
        bar.end = vec![
            WidgetSpec::new("tray"),
            WidgetSpec::new("volume"),
            WidgetSpec::new("clock"),
        ];
        Self {
            // Lanzador Kickoff = grilla de tiles.
            general: General {
                menu_style: "grid".to_string(),
                ..General::default()
            },
            surfaces: vec![bar],
        }
    }

    /// Vista **Solaris CDE** (era dorada): el *Front Panel* inferior chunky con
    /// el conmutador de escritorios al **centro**, flanqueado por el lanzador y
    /// el reloj a un lado y la lista de ventanas + bandeja al otro — el sello de
    /// CDE/Motif. Menú en grilla (estilo Application Manager).
    fn vista_solaris() -> Self {
        // El Front Panel de CDE: una sola superficie con el widget `front_panel`,
        // que se pinta como la franja chunky biselada entera (lanzadores +
        // switcher recessed + reloj). Grosor alto: CDE era robusto.
        let mut panel = Surface::bar(Anchor::Bottom);
        panel.thickness = 72.0;
        panel.center = vec![WidgetSpec::new("front_panel")];
        Self {
            general: General {
                menu_style: "grid".to_string(),
                ..General::default()
            },
            surfaces: vec![panel],
        }
    }
}

fn default_enabled() -> bool {
    true
}
fn default_thickness() -> f32 {
    32.0
}
fn default_padding() -> f32 {
    12.0
}
fn default_gap() -> f32 {
    16.0
}
fn default_opacity() -> f32 {
    1.0
}
fn default_rail_thickness() -> f32 {
    44.0
}
fn default_panel_width() -> f32 {
    280.0
}
fn default_timezone() -> String {
    "auto".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogo_filtra_por_tipo_de_superficie() {
        let bar: Vec<&str> = widgets_for_surface(SurfaceKind::Bar).iter().map(|w| w.kind).collect();
        let side: Vec<&str> = widgets_for_surface(SurfaceKind::Sidebar).iter().map(|w| w.kind).collect();
        // shuma se conecta a pata como widget en ambas (input en barra, shell en diente).
        assert!(bar.contains(&"shuma_input"));
        assert!(side.contains(&"shuma"));
        // El navegador (rico) es de sidebar, no de barra; el reloj al revés.
        assert!(side.contains(&"navigator") && !bar.contains(&"navigator"));
        assert!(bar.contains(&"clock") && !side.contains(&"clock"));
    }

    #[test]
    fn vista_preset_resuelve_las_vistas_y_difieren() {
        for slug in ["mirada", "dwm", "hyprland", "windows-xp", "windows-3.1", "mac", "kde", "solaris"] {
            let c = Config::vista_preset(slug).unwrap_or_else(|| panic!("vista {slug}"));
            assert!(!c.surfaces.is_empty(), "vista {slug} sin superficies");
        }
        assert!(Config::vista_preset("noexiste").is_none());
        // dwm: una sola barra fina arriba; XP: barra abajo.
        let dwm = Config::vista_preset("dwm").unwrap();
        assert_eq!(dwm.surfaces.len(), 1);
        assert_eq!(dwm.surfaces[0].anchor, Anchor::Top);
        let xp = Config::vista_preset("windows-xp").unwrap();
        assert_eq!(xp.surfaces[0].anchor, Anchor::Bottom);
        assert_eq!(xp.general.menu_style, "xp"); // menú Inicio 2-columnas
        assert_eq!(Config::vista_preset("kde").unwrap().general.menu_style, "grid");
        // mac: menubar + dock (dos superficies), el dock con apps fijadas.
        let mac = Config::vista_preset("mac").unwrap();
        assert_eq!(mac.surfaces.len(), 2);
        let dock = mac
            .surfaces
            .iter()
            .find(|s| s.kind == SurfaceKind::Dock)
            .expect("mac tiene dock");
        assert!(!dock.dock_pins.is_empty(), "el dock de mac trae apps fijadas");
    }

    #[test]
    fn anchor_horizontalidad() {
        assert!(Anchor::Top.es_horizontal());
        assert!(Anchor::Bottom.es_horizontal());
        assert!(!Anchor::Left.es_horizontal());
        assert!(!Anchor::Right.es_horizontal());
    }

    #[test]
    fn defaults_de_anchor_y_kind() {
        assert_eq!(Anchor::default(), Anchor::Top);
        assert_eq!(SurfaceKind::default(), SurfaceKind::Bar);
    }

    #[test]
    fn prop_helpers_y_promocion_int_a_num() {
        let w = WidgetSpec::new("clock")
            .with("format", Prop::Str("%H:%M".to_string()))
            .with("size", Prop::Int(14))
            .with("ratio", Prop::Num(0.5))
            .with("flag", Prop::Bool(true));
        assert_eq!(w.str_prop("format", "?"), "%H:%M");
        // Int se promueve a f64 al leer como número.
        assert_eq!(w.num_prop("size", 0.0), 14.0);
        assert_eq!(w.num_prop("ratio", 0.0), 0.5);
        assert!(w.bool_prop("flag", false));
        // Defaults cuando la clave falta o el tipo no calza.
        assert_eq!(w.str_prop("nope", "def"), "def");
        assert_eq!(w.num_prop("format", -1.0), -1.0);
    }

    #[test]
    fn preset_tiene_barra_top_sidebar_y_shell_bottom() {
        let cfg = Config::preset();
        assert_eq!(cfg.surfaces.len(), 4);
        let top = &cfg.surfaces[0];
        assert_eq!(top.anchor, Anchor::Top);
        assert_eq!(top.kind, SurfaceKind::Bar);
        assert_eq!(top.start[0].kind, "start_button");
        assert!(top.end.iter().any(|w| w.kind == "astro"));
        // CPU/RAM salieron del bar del default (van al diente «Sistema» del
        // sidebar derecho); el catálogo conserva los widgets.
        assert!(!top.end.iter().any(|w| w.kind == "cpu_meter"));
        assert!(!top.end.iter().any(|w| w.kind == "ram_meter"));
        // Los medidores que quedan (volumen) fijan size+orientation explícitos.
        let vol = top.end.iter().find(|w| w.kind == "volume").unwrap();
        assert_eq!(vol.str_prop("size", "?"), "medium");
        assert_eq!(vol.str_prop("orientation", "?"), "vertical");

        let rail = &cfg.surfaces[1];
        assert_eq!(rail.kind, SurfaceKind::Sidebar);
        assert_eq!(rail.anchor, Anchor::Left);
        // Docked y posición siguen los globales (None), no hardcodeados.
        assert_eq!(rail.reserve, None);
        assert_eq!(rail.rail_outside, None);
        assert!(!rail.tabs.is_empty());

        // Sidebar derecho: idem, sigue los globales.
        let rrail = &cfg.surfaces[2];
        assert_eq!(rrail.kind, SurfaceKind::Sidebar);
        assert_eq!(rrail.anchor, Anchor::Right);
        assert_eq!(rrail.reserve, None);
        assert!(!rrail.tabs.is_empty());
        // El sidebar derecho trae el monitor de sistema.
        assert!(rrail.tabs.iter().any(|t| t.content.kind == "monitor"));

        let shell = &cfg.surfaces[3];
        assert_eq!(shell.anchor, Anchor::Bottom);
        assert!(shell.autohide);
        assert_eq!(shell.center[0].kind, "shuma_input");
        assert_eq!(shell.center[0].str_prop("hotkey", "?"), "F12");
    }

    #[test]
    fn config_default_esta_vacio() {
        // Default (derive) = sin superficies; `preset()` es el marco poblado.
        let cfg = Config::default();
        assert!(cfg.surfaces.is_empty());
        assert_eq!(cfg.general.timezone, "auto");
    }

    #[test]
    fn surface_constructores() {
        assert_eq!(Surface::bar(Anchor::Left).anchor, Anchor::Left);
        assert_eq!(Surface::dock(Anchor::Bottom).kind, SurfaceKind::Dock);
    }

    #[test]
    fn sidebar_lleva_dientes_y_ancho_de_panel() {
        let mut sb = Surface::sidebar(Anchor::Left);
        sb.tabs.push(SidebarTab::new(
            "monads",
            "Mónadas",
            WidgetSpec::new("navigator").with("source", Prop::Str("nouser".to_string())),
        ));
        sb.tabs
            .push(SidebarTab::new("files", "Archivos", WidgetSpec::new("navigator")));
        assert_eq!(sb.kind, SurfaceKind::Sidebar);
        assert_eq!(sb.anchor, Anchor::Left);
        // El rail por defecto es fino; el panel desplegado es ancho.
        assert_eq!(sb.thickness, 44.0);
        assert_eq!(sb.panel_width, 280.0);
        assert_eq!(sb.tabs.len(), 2);
        assert_eq!(sb.tabs[0].icon, "monads");
        assert_eq!(sb.tabs[0].content.kind, "navigator");
        assert_eq!(sb.tabs[0].content.str_prop("source", "?"), "nouser");
    }
}
