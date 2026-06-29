//! `wawa-config` — bus de configuración del SO wawa.
//!
//! Dos capas de archivos JSON canónicos actúan como medio:
//!
//! 1. **Sistema** — `/etc/wawa/config.json` (Linux). Defaults
//!    machine-wide; lo escribe el admin con `wawactl --system set …`
//!    (requiere root) o un instalador.
//! 2. **Usuario** — `$XDG_CONFIG_HOME/wawa/config.json` (Linux:
//!    `~/.config/wawa/config.json`). Lo que escribe el panel y las
//!    apps; **sobreescribe** campo por campo a la capa de sistema.
//!
//! El panel de control y los daemons escriben; las apps Llimphi leen y
//! se suscriben a cambios vía [`notify::RecommendedWatcher`] sobre
//! **ambos** paths.
//!
//! ## Nota sobre `/etc/wawa`
//!
//! `/etc/` es una convención de Unix/Linux. Cuando wawa sea su propio
//! SO (no un userland sobre Linux), esta capa se reemplazará por el
//! mecanismo nativo de "config de sistema" que defina arje — la API
//! pública (`load`, `system_path`, `user_path`) se mantiene; sólo
//! cambia lo que devuelve `system_path()` adentro.
//!
//! Por qué archivo + `notify` y no un daemon pub-sub:
//!
//! * **Cero dependencias en runtime**: ninguna app necesita que un
//!   daemon esté arrancado para leer la config; basta con que el
//!   archivo exista (y si no existe, devuelve defaults).
//! * **Auditable y editable a mano**: el archivo es JSON con `pretty`,
//!   el admin lo abre con cualquier editor o lo edita por sed/jq.
//! * **Atomicidad simple**: `save()` escribe a `config.json.tmp` y
//!   `rename()` — los watchers ven un único evento de creación que
//!   contiene la versión completa.
//! * **Compatible con apps existentes**: el modelo de Llimphi ya
//!   reentra al `update` vía `Handle::dispatch`; el watcher dispara
//!   un Msg del consumidor cuando llega el evento.
//!
//! ## Forma del archivo
//!
//! ```json
//! {
//!   "theme_variant": "dark",
//!   "accent": "default",
//!   "lang": "es-PE",
//!   "timefmt_24h": true,
//!   "modules": {
//!     "mirada": true,
//!     "shuma": true,
//!     "chasqui": true,
//!     "akasha": true,
//!     "minga": true,
//!     "agora": true
//!   }
//! }
//! ```
//!
//! Campos desconocidos se ignoran al deserializar; campos ausentes
//! caen al default. Esto permite agregar nuevas keys sin romper
//! consumidores antiguos.
//!
//! ## Productor
//!
//! ```ignore
//! use wawa_config::WawaConfig;
//!
//! let mut cfg = WawaConfig::load();
//! cfg.theme_variant = "aurora".into();
//! cfg.save()?;
//! ```
//!
//! ## Consumidor (app Llimphi)
//!
//! ```ignore
//! use wawa_config::{WawaConfig, ConfigWatcher};
//!
//! // En `App::init`:
//! let handle = handle.clone();
//! let watcher = ConfigWatcher::spawn(move |cfg| {
//!     handle.dispatch(Msg::ConfigChanged(cfg));
//! })?;
//! // Guardar `watcher` en el Model para que viva todo lo que vive la app.
//! ```

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

/// Nombre del subdirectorio dentro de XDG_CONFIG_HOME donde vive el
/// archivo. Exporto la constante para que tests y herramientas
/// externas lo puedan inspeccionar.
pub const CONFIG_DIR: &str = "wawa";
/// Nombre del archivo canónico.
pub const CONFIG_FILE: &str = "config.json";
/// Directorio de la capa de sistema en Linux. Cuando wawa sea su
/// propio SO esta ruta se reemplaza por lo que defina arje; la API
/// pública (`system_config_path`) se mantiene.
pub const SYSTEM_CONFIG_DIR_LINUX: &str = "/etc/wawa";

/// Capa de la cual se cargó/escribió una config. Útil para herramientas
/// que necesiten distinguir explícitamente entre sistema y usuario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// `/etc/wawa/config.json` — defaults machine-wide, requiere root.
    System,
    /// `$XDG_CONFIG_HOME/wawa/config.json` — override por usuario.
    User,
}

/// Mapea el `theme_variant` de la config (lowercase, libre) al nombre
/// canónico que reconoce `llimphi_theme::Theme::by_name` (capitalizado).
/// Devuelve `None` si el variant no es uno de los presets conocidos —
/// el consumidor decide qué hacer (fallback a dark, error, etc.).
///
/// Los presets de Llimphi tienen `name: &'static str` capitalizado;
/// los users del CLI y el panel escriben en lowercase. Este shim
/// mantiene a `wawa-config` UI-agnóstico (no depende de
/// `llimphi-theme`) y a la vez evita que cada consumidor reimplemente
/// el casing.
pub fn canonical_theme_name(variant: &str) -> Option<&'static str> {
    match variant.to_ascii_lowercase().as_str() {
        "dark" => Some("Dark"),
        "light" => Some("Light"),
        "aurora" => Some("Aurora"),
        "sunset" => Some("Sunset"),
        "tawa" => Some("Tawa"),
        _ => None,
    }
}

/// Devuelve el color RGB de un acento por id. `default` retorna `None`
/// para que el consumidor no toque el accent del theme base. La paleta
/// es la misma del web (`tawasuyu-web/styles.css`): tinte por cuadrante
/// + accent tawasuyu por default.
///
/// Es un trio RGB (no un tipo de `peniko`) para no obligar a depender
/// de `llimphi-raster` desde acá. Los consumidores Llimphi hacen:
///
/// ```ignore
/// if let Some([r,g,b]) = wawa_config::accent_rgb(&cfg.accent) {
///     let c = llimphi_theme::Color::from_rgba8(r, g, b, 255);
///     theme.accent = c;
///     theme.border_focus = c;
/// }
/// ```
pub fn accent_rgb(accent: &str) -> Option<[u8; 3]> {
    match accent {
        "default" => None,
        "tawasuyu" => Some([0x6E, 0x8C, 0xDC]),
        "unanchay" => Some([0xB9, 0xC9, 0xE8]),
        "yachay" => Some([0xE8, 0xC9, 0x7A]),
        "ruway" => Some([0xE8, 0x9B, 0x6E]),
        "ukupacha" => Some([0x8F, 0xB5, 0x8C]),
        _ => None,
    }
}

/// Lista de variants de theme reconocidas — útil para validadores y
/// generadores de docs/UI. Orden estable.
pub const THEME_VARIANTS: &[&str] = &["dark", "light", "aurora", "sunset", "tawa"];

/// Lista de acentos reconocidos. `"default"` significa "no override".
pub const ACCENTS: &[&str] = &["default", "tawasuyu", "unanchay", "yachay", "ruway", "ukupacha"];

/// Identificadores estables de los módulos del SO conocidos. Las apps
/// son libres de leer/escribir otros, pero estos son los que el panel
/// expone por default — mantenerlos como `const` ayuda a no escribir
/// el string mal en sitios distintos.
pub mod modules {
    pub const MIRADA: &str = "mirada";
    pub const SHUMA: &str = "shuma";
    pub const CHASQUI: &str = "chasqui";
    pub const AKASHA: &str = "akasha";
    pub const MINGA: &str = "minga";
    pub const AGORA: &str = "agora";

    pub fn defaults() -> [(&'static str, bool); 6] {
        [
            (MIRADA, true),
            (SHUMA, true),
            (CHASQUI, true),
            (AKASHA, true),
            (MINGA, true),
            (AGORA, true),
        ]
    }
}

/// **IA + semántica globales** del SO: el backend del LLM (instrumento de
/// asistentes como `:?`/`:explica` de shuma, RAG de paloma…) y la búsqueda
/// semántica por embeddings (`:buscar` de shuma, etc.). Una sola fuente de
/// verdad, editable en wawa-panel; las apps la leen de acá (no per-app). Tipos
/// planos (`""`/`0` = "sin fijar, usar el default") para no acoplar este crate a
/// pluma-llm ni rimay-verbo.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiConfig {
    #[serde(default)]
    pub llm: LlmSettings,
    #[serde(default)]
    pub semantic: SemanticSettings,
    #[serde(default)]
    pub voz: VozSettings,
}

/// Selección de backend LLM. `backend` vacío = resolver por entorno (`from_env`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmSettings {
    /// `""` = auto (from_env). Si no: `anthropic`/`gemini`/`deepseek`/`cohere`/`ollama`/`mock`.
    #[serde(default)]
    pub backend: String,
    /// Modelo; `""` = default del backend.
    #[serde(default)]
    pub model: String,
    /// API key; `""` = leer del entorno (recomendado, no guardar la clave en claro).
    #[serde(default)]
    pub api_key: String,
    /// Endpoint custom (p.ej. Ollama remoto); `""` = default.
    #[serde(default)]
    pub endpoint: String,
}

impl LlmSettings {
    /// `true` si fija un backend explícito (si no, el consumidor cae a `from_env`).
    pub fn is_set(&self) -> bool {
        !self.backend.trim().is_empty()
    }
}

/// Ajustes de la búsqueda semántica por embeddings.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SemanticSettings {
    /// Habilita la búsqueda semántica. Apagada por defecto.
    #[serde(default)]
    pub enabled: bool,
    /// Socket del daemon de embeddings; `""` = socket por defecto.
    #[serde(default)]
    pub socket: String,
    /// Dimensión del provider mock cuando no hay daemon; `0` = 384.
    #[serde(default)]
    pub dim: usize,
}

impl SemanticSettings {
    /// La dimensión efectiva del fallback mock (default 384).
    pub fn effective_dim(&self) -> usize {
        if self.dim == 0 { 384 } else { self.dim }
    }
}

/// Selección de motores de **voz** (STT/TTS) + palabra de llamada y wake-word.
/// Espeja `rimay_voz::VozConfig` con tipos planos, para no acoplar este crate a
/// rimay-voz. Lo edita wawa-panel; lo leen los hosts de voz (shuma, mirada…).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct VozSettings {
    /// Backend de reconocimiento (STT). `""` = mock (sin modelo); `"local"` =
    /// daemon local; `"nube:openai:whisper-1"` = nube. Formato de
    /// `rimay_voz::Backend::parse` / `RIMAY_VOZ_STT`.
    #[serde(default)]
    pub stt: String,
    /// Backend de síntesis (TTS). Mismo formato (ej. `"nube:openai:tts-1"`).
    #[serde(default)]
    pub tts: String,
    /// Palabra de llamada (wake-word). `""` = `"shuma"`.
    #[serde(default)]
    pub llamado: String,
    /// Compuerta wake-word (F1): si está, estando dormido sólo se transcribe lo
    /// que suena al llamado (privacidad: el resto no llega al STT). Apagada por
    /// defecto (F0: transcribe toda utterance).
    #[serde(default)]
    pub wake: bool,
}

impl VozSettings {
    /// Palabra de llamada efectiva (default `"shuma"`).
    pub fn effective_llamado(&self) -> &str {
        let l = self.llamado.trim();
        if l.is_empty() { "shuma" } else { l }
    }
}

/// Configuración del sistema operativo wawa. Serializada como el JSON
/// del módulo. Campos nuevos se agregan con `#[serde(default = "…")]`
/// para preservar compatibilidad hacia atrás.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WawaConfig {
    /// Variante del theme global. Coincide con
    /// `llimphi_theme::Theme::name`: `"dark"`, `"light"`, `"aurora"`,
    /// `"sunset"`.
    #[serde(default = "default_theme_variant")]
    pub theme_variant: String,

    /// Acento. `"default"` deja el accent del theme; cualquier otro
    /// id (tawasuyu/unanchay/yachay/ruway/ukupacha) lo sobreescribe.
    #[serde(default = "default_accent")]
    pub accent: String,

    /// Locale activo. Acepta lo mismo que `rimay_localize::set_locale`:
    /// `"es-PE"`, `"en-US"`, `"qu-PE"`.
    #[serde(default = "default_lang")]
    pub lang: String,

    /// Formato del reloj (true = 24h, false = 12h con am/pm).
    #[serde(default = "default_timefmt")]
    pub timefmt_24h: bool,

    /// Estado on/off de los módulos del SO. Usa los ids de
    /// [`modules`]. BTreeMap → serializa con orden estable y diffs
    /// limpios en git.
    #[serde(default = "default_modules")]
    pub modules: BTreeMap<String, bool>,

    /// **Decisión global** de dónde van los rails de dientes (sidebars
    /// acoplables) respecto al área de trabajo: `false` (default) = DENTRO
    /// (overlay pegado al borde interno, como cosmos); `true` = FUERA (reservan
    /// su franja, achicando el contenido). TODAS las apps con dientes deben
    /// regirse por esto — una sola fuente de verdad, no por app.
    #[serde(default)]
    pub dientes_outside: bool,

    /// Proveedor de **fondo automático** elegido en el panel: `"bing"` (foto del
    /// día), `"nasa"` (APOD), `"folder"` (carpeta local), `"solar"` (por hora).
    /// El panel escribe `~/.config/mirada/wallpaper.ron` desde esto y lanza el
    /// daemon `mirada-wallpaper`. Vacío = sin fondo automático.
    #[serde(default)]
    pub wallpaper_provider: String,
    /// Cada cuántas **horas** refresca el fondo automático (default 6).
    #[serde(default = "default_wallpaper_hours")]
    pub wallpaper_interval_hours: u32,

    /// IA + semántica globales (LLM + embeddings). Vacío = LLM por entorno y
    /// búsqueda semántica apagada. Lo edita wawa-panel; lo leen shuma, paloma, …
    #[serde(default)]
    pub ai: AiConfig,
}

fn default_wallpaper_hours() -> u32 {
    6
}

fn default_theme_variant() -> String {
    "dark".into()
}
fn default_accent() -> String {
    "default".into()
}
fn default_lang() -> String {
    "es-PE".into()
}
fn default_timefmt() -> bool {
    true
}
fn default_modules() -> BTreeMap<String, bool> {
    modules::defaults()
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

impl Default for WawaConfig {
    fn default() -> Self {
        Self {
            theme_variant: default_theme_variant(),
            accent: default_accent(),
            lang: default_lang(),
            timefmt_24h: default_timefmt(),
            modules: default_modules(),
            dientes_outside: false,
            wallpaper_provider: String::new(),
            wallpaper_interval_hours: default_wallpaper_hours(),
            ai: AiConfig::default(),
        }
    }
}

impl WawaConfig {
    /// `true` si el módulo `id` está activo (default: activo si no se
    /// conoce — convención conservadora).
    pub fn module_enabled(&self, id: &str) -> bool {
        self.modules.get(id).copied().unwrap_or(true)
    }

    /// Conmuta el módulo `id`. Si no existía, lo agrega con `false`.
    pub fn toggle_module(&mut self, id: &str) {
        let v = self.modules.entry(id.to_string()).or_insert(true);
        *v = !*v;
    }

    /// Path canónico del archivo de usuario (alias de [`user_config_path`]).
    /// `None` si la plataforma no expone un config dir (extremadamente
    /// raro fuera de embebidos).
    pub fn path() -> Option<PathBuf> {
        user_config_path()
    }

    /// Path canónico del archivo de la capa indicada. `None` si la
    /// capa no aplica en esta plataforma (p. ej. `Layer::System` fuera
    /// de Linux).
    pub fn path_for(layer: Layer) -> Option<PathBuf> {
        match layer {
            Layer::System => system_config_path(),
            Layer::User => user_config_path(),
        }
    }

    /// Carga la config efectiva: defaults → capa de sistema → capa de
    /// usuario. Cada capa **sobreescribe campo por campo** lo que
    /// definió la anterior; campos ausentes preservan el valor
    /// previo. Para `modules`, el merge es key-by-key (no reemplazo
    /// total del mapa).
    ///
    /// Si ningún archivo existe, o están corruptos, devuelve defaults
    /// — nunca falla. Los errores se loggean a `tracing::warn`.
    pub fn load() -> Self {
        let mut acc = serde_json::to_value(Self::default())
            .expect("WawaConfig::default siempre serializa");
        for layer in [Layer::System, Layer::User] {
            if let Some(v) = load_layer_value(layer) {
                merge_json(&mut acc, v);
            }
        }
        serde_json::from_value(acc).unwrap_or_default()
    }

    /// Carga **sólo** la capa indicada, sin mergear con la otra. Útil
    /// para herramientas como `wawactl --system show` que necesitan
    /// inspeccionar una capa concreta. Si el archivo no existe,
    /// devuelve `None` (no defaults — distingue "ausente" de
    /// "presente con defaults"). Errores de parseo loggean warn y
    /// también devuelven `None`.
    pub fn load_layer(layer: Layer) -> Option<Self> {
        let path = Self::path_for(layer)?;
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                warn!(?path, error = %e, "wawa-config: read failed");
                return None;
            }
        };
        match serde_json::from_slice::<WawaConfig>(&bytes) {
            Ok(c) => Some(c),
            Err(e) => {
                warn!(?path, error = %e, "wawa-config: parse failed");
                None
            }
        }
    }

    /// Persiste atómicamente en la capa de **usuario** (compat con
    /// callers existentes): serializa a `config.json.tmp` y renombra
    /// sobre `config.json`. Crea el directorio padre si no existe.
    pub fn save(&self) -> Result<PathBuf, ConfigError> {
        self.save_to(Layer::User)
    }

    /// Persiste en la capa indicada. `Layer::System` apunta a
    /// `/etc/wawa/config.json` y típicamente requiere root — devuelve
    /// `ConfigError::Io` con `PermissionDenied` si no.
    pub fn save_to(&self, layer: Layer) -> Result<PathBuf, ConfigError> {
        let path = Self::path_for(layer).ok_or(ConfigError::NoProjectDirs)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }
}

/// Lee y parsea una capa como `serde_json::Value`. Devuelve `None`
/// (no defaults) si el archivo no existe o falla el parse — esto es
/// distinto del `WawaConfig::load_layer` que también devuelve Option,
/// pero acá trabajamos con Value para mergear sin perder "campo
/// ausente vs explícito".
fn load_layer_value(layer: Layer) -> Option<serde_json::Value> {
    let path = WawaConfig::path_for(layer)?;
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            warn!(?path, error = %e, "wawa-config: read failed");
            return None;
        }
    };
    match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(v) => Some(v),
        Err(e) => {
            warn!(?path, error = %e, "wawa-config: parse failed");
            None
        }
    }
}

/// Merge profundo: `over` sobreescribe `base` hoja por hoja, recursivo
/// sobre objetos JSON. Para arrays y escalares, `over` reemplaza
/// completamente. Esto preserva la semántica "campo ausente → no
/// modifica la capa inferior" y permite que un user override sólo
/// algunas keys de `modules`.
fn merge_json(base: &mut serde_json::Value, over: serde_json::Value) {
    use serde_json::Value;
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => merge_json(existing, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

/// Errores de IO o serialización al persistir la config. La carga
/// nunca falla — devuelve defaults en su lugar.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("no hay ProjectDirs en esta plataforma")]
    NoProjectDirs,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("notify: {0}")]
    Notify(#[from] notify::Error),
}

/// Path del archivo de **usuario**. El qualifier "" + organization ""
/// se mapea a `$XDG_CONFIG_HOME/wawa/` en Linux (típicamente
/// `~/.config/wawa/`), `~/Library/Application Support/wawa/` en
/// macOS, `%APPDATA%/wawa/` en Windows. `None` si la plataforma no
/// expone un config dir.
pub fn user_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", CONFIG_DIR)
        .map(|d| d.config_dir().join(CONFIG_FILE))
}

/// Path del archivo de **sistema**. `Some("/etc/wawa/config.json")`
/// en Linux; `None` en otras plataformas (no hay convención
/// equivalente y no vale la pena inventarla). Cuando wawa sea su
/// propio SO, esta función devolverá el equivalente nativo y la API
/// pública no cambia.
pub fn system_config_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        Some(PathBuf::from(SYSTEM_CONFIG_DIR_LINUX).join(CONFIG_FILE))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

// =====================================================================
// Watcher
// =====================================================================

/// Suscripción al bus. Mantenelo vivo (guardalo en el Model de tu app)
/// para seguir recibiendo notificaciones; al dropearlo, los callbacks
/// dejan de dispararse.
///
/// Observa **ambas capas** (sistema y usuario): un cambio en
/// cualquiera dispara `on_change` con la config efectiva ya mergeada.
/// Cada capa escucha el directorio padre con
/// `RecursiveMode::NonRecursive` y filtra por `config.json` — así
/// detecta tanto modificaciones in-place como reemplazos atómicos por
/// `rename`. Si la capa de sistema no aplica en la plataforma (no
/// Linux), o no se puede crear/observar (p. ej. `/etc/wawa` sin
/// permisos de lectura — improbable porque `/etc/` es world-readable
/// por convención), se ignora con un warn y el watcher sigue activo
/// sólo sobre la capa de usuario.
///
/// Para evitar disparar dos veces seguidas cuando un editor escribe
/// con la secuencia `truncate → write → close`, el watcher debouncea
/// internamente con un timeout de ~200 ms: agrupa eventos consecutivos
/// y emite un único callback con la última versión leída.
pub struct ConfigWatcher {
    _watchers: Vec<RecommendedWatcher>,
    _debounce_thread: Option<thread::JoinHandle<()>>,
}

impl ConfigWatcher {
    /// Arranca el watcher. `on_change` se llama cada vez que **alguna**
    /// de las capas cambia, ya con la nueva config efectiva mergeada
    /// (sistema ← usuario). Si el parseo falla, no se invoca (se
    /// loggea como warn y se ignora hasta el próximo cambio).
    ///
    /// `on_change` corre en un thread propio del watcher — para
    /// reentrar al loop de Llimphi, capturá un `Handle<Msg>` clonado
    /// y llamá `handle.dispatch(...)` dentro de la closure.
    pub fn spawn<F>(on_change: F) -> Result<Self, ConfigError>
    where
        F: FnMut(WawaConfig) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<()>();
        let mut watchers = Vec::with_capacity(2);

        // Capa de usuario es obligatoria; si no hay ProjectDirs es un
        // entorno raro y devolvemos error como antes.
        let user_path = user_config_path().ok_or(ConfigError::NoProjectDirs)?;
        watchers.push(spawn_layer_watcher(&user_path, tx.clone(), /*must_exist=*/ true)?);

        // Capa de sistema es best-effort: si no aplica (no Linux), o
        // no se puede observar (sin permiso de lectura de `/etc/wawa`,
        // que es muy raro pero posible si el admin la chmodea), no
        // rompe — sólo no se entera de cambios de sistema.
        if let Some(sys_path) = system_config_path() {
            match spawn_layer_watcher(&sys_path, tx.clone(), /*must_exist=*/ false) {
                Ok(w) => watchers.push(w),
                Err(e) => warn!(?sys_path, error = %e, "wawa-config: system layer watch skipped"),
            }
        }

        // Debounce: junta señales durante ~200 ms y al cierre llama
        // `on_change` con la lectura más reciente (mergeada). Acepta
        // que perdamos ráfagas intermedias — solo importa el estado
        // final.
        let debounce = thread::Builder::new()
            .name("wawa-config-debounce".into())
            .spawn(move || debounce_loop(rx, Box::new(on_change)))
            .ok();

        Ok(Self {
            _watchers: watchers,
            _debounce_thread: debounce,
        })
    }
}

fn spawn_layer_watcher(
    path: &Path,
    tx: mpsc::Sender<()>,
    must_exist: bool,
) -> Result<RecommendedWatcher, ConfigError> {
    let parent = path
        .parent()
        .ok_or_else(|| {
            ConfigError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                "config path sin parent",
            ))
        })?
        .to_path_buf();

    // Para la capa de usuario creamos el dir si falta — notify puede
    // watchear un dir vacío. Para la de sistema no lo creamos: si
    // `/etc/wawa` no existe, probablemente esta máquina no usa la capa
    // de sistema y mejor no requerir permisos de root para correr una
    // app de usuario.
    if must_exist {
        std::fs::create_dir_all(&parent)?;
    } else if !parent.exists() {
        return Err(ConfigError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "directorio de capa de sistema ausente",
        )));
    }

    let target_name = path.file_name().map(|n| n.to_owned());
    let mut watcher =
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let event = match res {
                Ok(e) => e,
                Err(e) => {
                    warn!(error = %e, "wawa-config: watcher error");
                    return;
                }
            };
            let is_target = match &target_name {
                Some(name) => event
                    .paths
                    .iter()
                    .any(|p| p.file_name().map(|f| f == name.as_os_str()).unwrap_or(false)),
                None => true,
            };
            if !is_target {
                return;
            }
            if !matches!(
                event.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
            ) {
                return;
            }
            let _ = tx.send(());
        })?;
    watcher.watch(&parent, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

fn debounce_loop(rx: mpsc::Receiver<()>, mut on_change: Box<dyn FnMut(WawaConfig) + Send>) {
    const QUIET: Duration = Duration::from_millis(200);
    loop {
        // Esperar al primer evento sin timeout.
        if rx.recv().is_err() {
            return;
        }
        // Drenar lo que se acumule en la ventana de quiet.
        loop {
            match rx.recv_timeout(QUIET) {
                Ok(()) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        let cfg = WawaConfig::load();
        on_change(cfg);
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trip() {
        let c = WawaConfig::default();
        let s = serde_json::to_string(&c).unwrap();
        let back: WawaConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn partial_json_uses_defaults() {
        // Sólo se aporta theme; el resto debe caer al default.
        let s = r#"{"theme_variant":"aurora"}"#;
        let c: WawaConfig = serde_json::from_str(s).unwrap();
        assert_eq!(c.theme_variant, "aurora");
        assert_eq!(c.accent, "default");
        assert_eq!(c.lang, "es-PE");
        assert!(c.timefmt_24h);
        assert!(c.module_enabled(modules::MIRADA));
    }

    #[test]
    fn unknown_fields_ignored() {
        // Un campo extra no rompe la deserialización.
        let s = r#"{"theme_variant":"dark","unknown":42}"#;
        let _c: WawaConfig = serde_json::from_str(s).unwrap();
    }

    #[test]
    fn voz_settings_round_trip() {
        // Lo que escribe wawa-panel (ai.voz) debe sobrevivir el ciclo a disco.
        let mut c = WawaConfig::default();
        c.ai.voz.stt = "nube:openai:whisper-1".into();
        c.ai.voz.tts = "local".into();
        c.ai.voz.llamado = "shuma".into();
        c.ai.voz.wake = true;
        let s = serde_json::to_string(&c).unwrap();
        let back: WawaConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c.ai.voz, back.ai.voz);
    }

    #[test]
    fn voz_llamado_efectivo_cae_a_shuma() {
        let mut v = VozSettings::default();
        assert_eq!(v.effective_llamado(), "shuma"); // vacío
        v.llamado = "  ".into();
        assert_eq!(v.effective_llamado(), "shuma"); // sólo espacios
        v.llamado = "wawa".into();
        assert_eq!(v.effective_llamado(), "wawa");
    }

    #[test]
    fn voz_ausente_en_json_cae_a_default() {
        // Config vieja sin `ai.voz` no debe romper: F0, mock, sin llamado fijado.
        let c: WawaConfig = serde_json::from_str(r#"{"ai":{}}"#).unwrap();
        assert_eq!(c.ai.voz, VozSettings::default());
        assert!(!c.ai.voz.wake);
        assert_eq!(c.ai.voz.effective_llamado(), "shuma");
    }

    #[test]
    fn toggle_module_persists_value() {
        let mut c = WawaConfig::default();
        assert!(c.module_enabled(modules::MIRADA));
        c.toggle_module(modules::MIRADA);
        assert!(!c.module_enabled(modules::MIRADA));
        c.toggle_module("inexistente");
        assert!(!c.module_enabled("inexistente"));
    }

    #[test]
    fn canonical_theme_maps_variants() {
        assert_eq!(canonical_theme_name("dark"), Some("Dark"));
        assert_eq!(canonical_theme_name("LIGHT"), Some("Light"));
        assert_eq!(canonical_theme_name("Aurora"), Some("Aurora"));
        assert_eq!(canonical_theme_name("sunset"), Some("Sunset"));
        assert_eq!(canonical_theme_name("hyperdark"), None);
    }

    #[test]
    fn accent_rgb_default_is_none() {
        assert_eq!(accent_rgb("default"), None);
        assert_eq!(accent_rgb("tawasuyu"), Some([0x6E, 0x8C, 0xDC]));
        assert_eq!(accent_rgb("ukupacha"), Some([0x8F, 0xB5, 0x8C]));
        assert_eq!(accent_rgb("desconocido"), None);
    }

    #[test]
    fn merge_user_over_system_overrides_field_by_field() {
        // Sistema define theme aurora y lang qu-PE; usuario sólo
        // sobreescribe lang. Resultado: theme aurora (de sistema),
        // lang en-US (de usuario).
        let mut base = serde_json::to_value(WawaConfig::default()).unwrap();
        let system: serde_json::Value =
            serde_json::from_str(r#"{"theme_variant":"aurora","lang":"qu-PE"}"#).unwrap();
        let user: serde_json::Value =
            serde_json::from_str(r#"{"lang":"en-US"}"#).unwrap();
        merge_json(&mut base, system);
        merge_json(&mut base, user);
        let final_cfg: WawaConfig = serde_json::from_value(base).unwrap();
        assert_eq!(final_cfg.theme_variant, "aurora");
        assert_eq!(final_cfg.lang, "en-US");
        // El resto cae al default.
        assert_eq!(final_cfg.accent, "default");
    }

    #[test]
    fn merge_modules_is_deep_per_key() {
        // Sistema apaga mirada; usuario apaga shuma. Esperado: ambos
        // off, el resto en su default true.
        let mut base = serde_json::to_value(WawaConfig::default()).unwrap();
        let system: serde_json::Value =
            serde_json::from_str(r#"{"modules":{"mirada":false}}"#).unwrap();
        let user: serde_json::Value =
            serde_json::from_str(r#"{"modules":{"shuma":false}}"#).unwrap();
        merge_json(&mut base, system);
        merge_json(&mut base, user);
        let final_cfg: WawaConfig = serde_json::from_value(base).unwrap();
        assert!(!final_cfg.module_enabled(modules::MIRADA));
        assert!(!final_cfg.module_enabled(modules::SHUMA));
        assert!(final_cfg.module_enabled(modules::CHASQUI));
    }

    #[test]
    fn system_path_only_on_linux() {
        let p = system_config_path();
        if cfg!(target_os = "linux") {
            assert_eq!(p, Some(PathBuf::from("/etc/wawa/config.json")));
        } else {
            assert!(p.is_none());
        }
    }

    #[test]
    fn constants_match_helpers() {
        // THEME_VARIANTS y ACCENTS deben coincidir con lo que aceptan
        // los helpers — guarda contra agregar uno y olvidar el otro.
        for v in THEME_VARIANTS {
            assert!(canonical_theme_name(v).is_some(), "variant {v} sin mapeo");
        }
        for a in ACCENTS {
            // accent_rgb("default") es None por diseño; el resto debe
            // tener color asignado.
            if *a == "default" {
                assert_eq!(accent_rgb(a), None);
            } else {
                assert!(accent_rgb(a).is_some(), "accent {a} sin color");
            }
        }
    }
}

/// Path absoluto del config dir de **usuario** — alias por compat.
/// `None` si no hay ProjectDirs disponibles.
pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", CONFIG_DIR).map(|d| d.config_dir().to_path_buf())
}

/// Path absoluto del config dir de **sistema** (`/etc/wawa` en Linux).
/// `None` en otras plataformas.
pub fn system_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        Some(PathBuf::from(SYSTEM_CONFIG_DIR_LINUX))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Helper opcional: agrega el `path` provisto a una lista de
/// watchers. No es parte del flujo normal — está expuesto para
/// herramientas que quieran observar un directorio externo (p. ej.
/// `/etc/wawa/` para configuración del sistema vs el del usuario).
/// El default (`spawn`) ya cubre el caso típico.
pub fn watch_path(
    p: &Path,
    on_event: impl FnMut(notify::Event) + Send + 'static,
) -> Result<RecommendedWatcher, ConfigError> {
    let mut on_event = on_event;
    let mut w =
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
            Ok(ev) => on_event(ev),
            Err(e) => warn!(error = %e, "wawa-config: external watch error"),
        })?;
    w.watch(p, RecursiveMode::NonRecursive)?;
    Ok(w)
}
