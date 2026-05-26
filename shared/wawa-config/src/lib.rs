//! `wawa-config` — bus de configuración del SO wawa.
//!
//! Un único archivo JSON canónico (`$XDG_CONFIG_HOME/wawa/config.json`)
//! actúa como medio: el panel de control y los daemons escriben; las
//! apps Llimphi leen y se suscriben a cambios vía
//! [`notify::RecommendedWatcher`].
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
        _ => None,
    }
}

/// Devuelve el color RGB de un acento por id. `default` retorna `None`
/// para que el consumidor no toque el accent del theme base. La paleta
/// es la misma del web (`gioser-web/styles.css`): tinte por cuadrante
/// + accent gioser por default.
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
        "gioser" => Some([0x6E, 0x8C, 0xDC]),
        "unanchay" => Some([0xB9, 0xC9, 0xE8]),
        "yachay" => Some([0xE8, 0xC9, 0x7A]),
        "ruway" => Some([0xE8, 0x9B, 0x6E]),
        "ukupacha" => Some([0x8F, 0xB5, 0x8C]),
        _ => None,
    }
}

/// Lista de variants de theme reconocidas — útil para validadores y
/// generadores de docs/UI. Orden estable.
pub const THEME_VARIANTS: &[&str] = &["dark", "light", "aurora", "sunset"];

/// Lista de acentos reconocidos. `"default"` significa "no override".
pub const ACCENTS: &[&str] = &["default", "gioser", "unanchay", "yachay", "ruway", "ukupacha"];

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
    /// id (gioser/unanchay/yachay/ruway/ukupacha) lo sobreescribe.
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

    /// Path canónico del archivo. `None` si la plataforma no expone
    /// un config dir (extremadamente raro fuera de embebidos).
    pub fn path() -> Option<PathBuf> {
        config_path()
    }

    /// Carga la config del disco. Si no existe, está corrupta, o no
    /// hay ProjectDirs, devuelve defaults — nunca falla. Los errores
    /// se loggean a `tracing::warn`.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                warn!(?path, error = %e, "wawa-config: read failed, using defaults");
                return Self::default();
            }
        };
        match serde_json::from_slice::<WawaConfig>(&bytes) {
            Ok(c) => c,
            Err(e) => {
                warn!(?path, error = %e, "wawa-config: parse failed, using defaults");
                Self::default()
            }
        }
    }

    /// Persiste atómicamente: serializa a `config.json.tmp` y renombra
    /// sobre `config.json`. Crea el directorio padre si no existe.
    pub fn save(&self) -> Result<PathBuf, ConfigError> {
        let path = config_path().ok_or(ConfigError::NoProjectDirs)?;
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

fn config_path() -> Option<PathBuf> {
    // El qualifier "" + organization "" se mapea a
    // `$XDG_CONFIG_HOME/wawa/` en Linux (típicamente
    // `~/.config/wawa/`), `~/Library/Application Support/wawa/` en
    // macOS, `%APPDATA%/wawa/` en Windows.
    directories::ProjectDirs::from("", "", CONFIG_DIR)
        .map(|d| d.config_dir().join(CONFIG_FILE))
}

// =====================================================================
// Watcher
// =====================================================================

/// Suscripción al bus. Mantenelo vivo (guardalo en el Model de tu app)
/// para seguir recibiendo notificaciones; al dropearlo, los callbacks
/// dejan de dispararse.
///
/// El watcher escucha el directorio padre con `RecursiveMode::
/// NonRecursive` y filtra por `config.json` — así detecta tanto
/// modificaciones in-place como reemplazos atómicos por `rename`.
///
/// Para evitar disparar dos veces seguidas cuando un editor escribe
/// con la secuencia `truncate → write → close`, el watcher debouncea
/// internamente con un timeout de ~200 ms: agrupa eventos consecutivos
/// y emite un único callback con la última versión leída.
pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
    _debounce_thread: Option<thread::JoinHandle<()>>,
}

impl ConfigWatcher {
    /// Arranca el watcher. `on_change` se llama cada vez que el
    /// archivo cambia, ya con la nueva config parseada. Si el parseo
    /// falla, no se invoca (se loggea como warn y se ignora hasta el
    /// próximo cambio).
    ///
    /// `on_change` corre en un thread propio del watcher — para
    /// reentrar al loop de Llimphi, capturá un `Handle<Msg>` clonado
    /// y llamá `handle.dispatch(...)` dentro de la closure.
    pub fn spawn<F>(on_change: F) -> Result<Self, ConfigError>
    where
        F: FnMut(WawaConfig) + Send + 'static,
    {
        let path = config_path().ok_or(ConfigError::NoProjectDirs)?;
        let parent = path
            .parent()
            .ok_or_else(|| {
                ConfigError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "config path sin parent",
                ))
            })?
            .to_path_buf();
        // Crear el dir si falta: si el archivo todavía no existe,
        // notify igual puede watchear el dir vacío.
        std::fs::create_dir_all(&parent)?;

        let target_name = path.file_name().map(|n| n.to_owned());
        let (tx, rx) = mpsc::channel::<()>();

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
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
            // Disparar al debounce sin importar si tiene capacidad —
            // si ya hay uno pendiente no necesitamos otro.
            let _ = tx.send(());
        })?;

        watcher.watch(&parent, RecursiveMode::NonRecursive)?;

        // Debounce: junta señales durante ~200 ms y al cierre llama
        // `on_change` con la lectura más reciente. Acepta que perdamos
        // ráfagas intermedias — solo importa el estado final.
        let debounce = thread::Builder::new()
            .name("wawa-config-debounce".into())
            .spawn(move || debounce_loop(rx, Box::new(on_change)))
            .ok();

        Ok(Self {
            _watcher: watcher,
            _debounce_thread: debounce,
        })
    }
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
        assert_eq!(accent_rgb("gioser"), Some([0x6E, 0x8C, 0xDC]));
        assert_eq!(accent_rgb("ukupacha"), Some([0x8F, 0xB5, 0x8C]));
        assert_eq!(accent_rgb("desconocido"), None);
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

/// Path absoluto de utilidad para que apps externas (no las del
/// monorepo) puedan resolver el config dir sin importar `directories`.
/// `None` si no hay ProjectDirs disponibles.
pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", CONFIG_DIR).map(|d| d.config_dir().to_path_buf())
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
