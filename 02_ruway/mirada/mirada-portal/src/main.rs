//! `mirada-portal` — backend de `xdg-desktop-portal` para el escritorio
//! mirada.
//!
//! Implementa la interfaz `org.freedesktop.impl.portal.Settings` y
//! publica un único namespace: `org.freedesktop.appearance`. Con eso,
//! GTK4/libadwaita, Qt6, Firefox y Chromium leen del sistema —
//! **por protocolo, sin tocar sus archivos de config** — si el
//! escritorio está en modo claro u oscuro, su color de acento y si pide
//! contraste alto. Cuando el tema de `nahual` cambia, el portal emite
//! `SettingChanged` y todas esas apps voltean en vivo.
//!
//! Fuente del tema: el archivo que persiste `nahual-theme`
//! (`$XDG_CONFIG_HOME/nahual/theme`, contiene el nombre del preset
//! activo). El portal lo vigila con `notify` y reexpone sus hechos —
//! ver [`theme_facts`].
//!
//! Este crate es el **backend** del portal: el frontend genérico
//! `xdg-desktop-portal` lo enruta vía el archivo `mirada.portal`. Ver
//! el README para la instalación de los archivos de `data/`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{fdo, interface, SignalContext};

mod theme_facts;
use theme_facts::ThemeFacts;

/// Nombre de bus del backend. El patrón `org.freedesktop.impl.portal.
/// desktop.<id>` es el que espera el frontend `xdg-desktop-portal`; el
/// `<id>` (`mirada`) tiene que coincidir con el `DBusName` del archivo
/// `mirada.portal`.
const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.mirada";

/// Ruta de objeto canónica de los portales del escritorio.
const OBJ_PATH: &str = "/org/freedesktop/portal/desktop";

/// Único namespace que servimos. El estándar moderno que leen GTK, Qt,
/// Firefox y Chromium para claro/oscuro + acento.
const APPEARANCE_NS: &str = "org.freedesktop.appearance";

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    info!("mirada-portal: arrancando backend org.freedesktop.impl.portal.Settings");

    let theme_path = theme_config_path();
    let initial = read_facts(theme_path.as_deref());
    info!(
        ?theme_path,
        color_scheme = initial.color_scheme(),
        contrast = initial.contrast(),
        "tema inicial resuelto"
    );

    let facts = Arc::new(Mutex::new(initial));
    let portal = SettingsPortal {
        facts: Arc::clone(&facts),
    };

    // El portal vive en el bus de **sesión** (no el de sistema): es un
    // servicio del escritorio del usuario, no del sistema.
    let conn_result = zbus::connection::Builder::session()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJ_PATH, portal));

    match conn_result {
        Ok(builder) => match builder.build().await {
            Ok(conn) => {
                info!(name = BUS_NAME, "name adquirido en el bus de sesión");
                run(conn, facts, theme_path).await
            }
            Err(e) => {
                warn!(?e, "no se pudo construir la conexión D-Bus — modo idle");
                wait_for_term().await
            }
        },
        Err(e) => {
            warn!(?e, "builder D-Bus falló (¿hay bus de sesión?) — modo idle");
            wait_for_term().await
        }
    }
}

/// Conectado al bus: monta el watcher del tema y espera la señal de
/// término. El watcher se guarda en `_watcher` para que no se dropee
/// (al dropearse dejaría de vigilar).
async fn run(
    conn: zbus::Connection,
    facts: Arc<Mutex<ThemeFacts>>,
    theme_path: Option<PathBuf>,
) -> anyhow::Result<()> {
    let _watcher = match &theme_path {
        Some(path) => match spawn_theme_watcher(conn.clone(), Arc::clone(&facts), path.clone()) {
            Ok(w) => Some(w),
            Err(e) => {
                warn!(
                    ?e,
                    "watcher del tema no disponible — el portal no actualizará en vivo"
                );
                None
            }
        },
        None => {
            warn!("sin ruta de config de tema — el portal sirve un valor fijo");
            None
        }
    };
    wait_for_term().await
}

// ============================================================================
// Interfaz D-Bus: org.freedesktop.impl.portal.Settings
// ============================================================================

struct SettingsPortal {
    /// Hechos del tema activo. El watcher los reescribe cuando cambia.
    facts: Arc<Mutex<ThemeFacts>>,
}

#[interface(name = "org.freedesktop.impl.portal.Settings")]
impl SettingsPortal {
    /// Versión de la interfaz impl. `ReadOne` se agregó en la 2.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }

    /// `ReadAll(namespaces) -> a{sa{sv}}`. Los `namespaces` son patrones
    /// (sufijo `*` = prefijo); lista vacía = todos. Sólo respondemos
    /// `org.freedesktop.appearance`.
    async fn read_all(
        &self,
        namespaces: Vec<String>,
    ) -> fdo::Result<HashMap<String, HashMap<String, OwnedValue>>> {
        let mut out = HashMap::new();
        if namespace_requested(&namespaces, APPEARANCE_NS) {
            let facts = *self.facts.lock().unwrap();
            out.insert(APPEARANCE_NS.to_string(), appearance_map(&facts)?);
        }
        Ok(out)
    }

    /// `ReadOne(namespace, key) -> v`. Lee un único valor.
    async fn read_one(&self, namespace: String, key: String) -> fdo::Result<OwnedValue> {
        let facts = *self.facts.lock().unwrap();
        lookup(&facts, &namespace, &key)
    }

    /// `Read(namespace, key) -> v`. Deprecado a favor de `ReadOne` desde
    /// la versión 2 del portal, pero apps viejas lo siguen llamando.
    async fn read(&self, namespace: String, key: String) -> fdo::Result<OwnedValue> {
        let facts = *self.facts.lock().unwrap();
        lookup(&facts, &namespace, &key)
    }

    /// `SettingChanged(namespace, key, value)`. Lo emite el watcher
    /// cuando el tema persistido cambia.
    #[zbus(signal)]
    async fn setting_changed(
        ctxt: &SignalContext<'_>,
        namespace: &str,
        key: &str,
        value: Value<'_>,
    ) -> zbus::Result<()>;
}

// ============================================================================
// Mapeo tema → valores del portal
// ============================================================================

/// Construye el mapa `a{sv}` del namespace `org.freedesktop.appearance`.
fn appearance_map(facts: &ThemeFacts) -> fdo::Result<HashMap<String, OwnedValue>> {
    Ok(HashMap::from([
        (
            "color-scheme".to_string(),
            into_owned(Value::U32(facts.color_scheme()))?,
        ),
        (
            "contrast".to_string(),
            into_owned(Value::U32(facts.contrast()))?,
        ),
        ("accent-color".to_string(), into_owned(accent_value(facts))?),
    ]))
}

/// Resuelve una clave concreta dentro de `org.freedesktop.appearance`.
fn lookup(facts: &ThemeFacts, namespace: &str, key: &str) -> fdo::Result<OwnedValue> {
    if namespace != APPEARANCE_NS {
        return Err(fdo::Error::Failed(format!(
            "namespace no servido por mirada-portal: {namespace}"
        )));
    }
    let value = match key {
        "color-scheme" => Value::U32(facts.color_scheme()),
        "contrast" => Value::U32(facts.contrast()),
        "accent-color" => accent_value(facts),
        other => {
            return Err(fdo::Error::Failed(format!(
                "clave desconocida en {APPEARANCE_NS}: {other}"
            )));
        }
    };
    into_owned(value)
}

/// El acento como structure `(ddd)` — tres dobles RGB en 0..1.
fn accent_value(facts: &ThemeFacts) -> Value<'static> {
    let (r, g, b) = facts.accent_rgb();
    Value::from((r, g, b))
}

/// `Value` → `OwnedValue`. Sólo falla con valores que llevan fds; los
/// nuestros (enteros y dobles) nunca lo hacen.
fn into_owned(value: Value<'_>) -> fdo::Result<OwnedValue> {
    OwnedValue::try_from(value).map_err(|e| fdo::Error::Failed(format!("zvariant: {e}")))
}

/// ¿El patrón de namespaces de un `ReadAll` pide `ns`? Lista vacía =
/// todos. Un patrón con sufijo `*` matchea por prefijo; sino, exacto.
fn namespace_requested(patterns: &[String], ns: &str) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => ns.starts_with(prefix),
        None => p == ns,
    })
}

// ============================================================================
// Watcher del tema persistido
// ============================================================================

/// Vigila el archivo de tema de `nahual`; cuando cambia, recomputa los
/// hechos y emite `SettingChanged`. Devuelve el watcher, que el caller
/// debe mantener vivo.
fn spawn_theme_watcher(
    conn: zbus::Connection,
    facts: Arc<Mutex<ThemeFacts>>,
    path: PathBuf,
) -> notify::Result<notify::RecommendedWatcher> {
    use notify::{RecursiveMode, Watcher};

    // Canal acotado: el callback de notify (en su propio hilo) sólo
    // hace `try_send`; si el buffer está lleno ya hay un evento
    // pendiente y da igual perder éste — coalescencia natural.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(8);
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = tx.try_send(());
        }
    })?;

    // Vigilamos el **directorio padre**: así captamos también la
    // creación del archivo si aún no existe.
    let watch_target = path.parent().unwrap_or(&path).to_path_buf();
    watcher.watch(&watch_target, RecursiveMode::NonRecursive)?;
    info!(dir = ?watch_target, "vigilando el directorio del tema");

    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            let fresh = read_facts(Some(&path));
            let changed = {
                let mut guard = facts.lock().unwrap();
                let differs = *guard != fresh;
                *guard = fresh;
                differs
            };
            if changed {
                info!(
                    color_scheme = fresh.color_scheme(),
                    contrast = fresh.contrast(),
                    "el tema cambió — emitiendo SettingChanged"
                );
                if let Err(e) = emit_appearance_changed(&conn, &fresh).await {
                    warn!(?e, "no se pudo emitir SettingChanged");
                }
            }
        }
    });

    Ok(watcher)
}

/// Emite `SettingChanged` para las tres claves de `appearance`.
async fn emit_appearance_changed(conn: &zbus::Connection, facts: &ThemeFacts) -> zbus::Result<()> {
    let ctxt = SignalContext::new(conn, OBJ_PATH)?;
    SettingsPortal::setting_changed(
        &ctxt,
        APPEARANCE_NS,
        "color-scheme",
        Value::U32(facts.color_scheme()),
    )
    .await?;
    SettingsPortal::setting_changed(
        &ctxt,
        APPEARANCE_NS,
        "contrast",
        Value::U32(facts.contrast()),
    )
    .await?;
    SettingsPortal::setting_changed(&ctxt, APPEARANCE_NS, "accent-color", accent_value(facts))
        .await?;
    Ok(())
}

// ============================================================================
// Lectura del tema persistido
// ============================================================================

/// Lee el nombre de tema del archivo y resuelve sus hechos. Si el
/// archivo falta o está vacío, asume `Nebula` — el default de
/// `nahual_theme::install_default`.
fn read_facts(path: Option<&Path>) -> ThemeFacts {
    let name = path
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Nebula".to_string());
    theme_facts::facts_for(&name)
}

/// Ruta del archivo donde `nahual-theme` persiste el nombre del tema
/// activo. Réplica de `nahual_theme::config_path()` — `mirada-portal`
/// no enlaza GPUI, así que no puede llamarla directamente. Convención
/// XDG: `$XDG_CONFIG_HOME/nahual/theme`, sino `$HOME/.config/...`.
fn theme_config_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|s| !s.is_empty())
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("nahual").join("theme"))
}

// ============================================================================
// Plomería
// ============================================================================

async fn wait_for_term() -> anyhow::Result<()> {
    let mut term = signal(SignalKind::terminate())?;
    let mut int_ = signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = term.recv() => info!("SIGTERM"),
        _ = int_.recv() => info!("SIGINT"),
    }
    Ok(())
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("mirada_portal=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_empty_matches_all() {
        assert!(namespace_requested(&[], APPEARANCE_NS));
    }

    #[test]
    fn namespace_exact_match() {
        assert!(namespace_requested(
            &[APPEARANCE_NS.to_string()],
            APPEARANCE_NS
        ));
        assert!(!namespace_requested(
            &["org.example".to_string()],
            APPEARANCE_NS
        ));
    }

    #[test]
    fn namespace_wildcard_prefix() {
        assert!(namespace_requested(
            &["org.freedesktop.*".to_string()],
            APPEARANCE_NS
        ));
        assert!(!namespace_requested(
            &["org.gnome.*".to_string()],
            APPEARANCE_NS
        ));
    }

    #[test]
    fn appearance_map_has_three_keys() {
        let facts = theme_facts::facts_for("Nebula");
        let m = appearance_map(&facts).unwrap();
        assert_eq!(m.len(), 3);
        assert!(m.contains_key("color-scheme"));
        assert!(m.contains_key("accent-color"));
        assert!(m.contains_key("contrast"));
    }

    #[test]
    fn lookup_unknown_namespace_errs() {
        let facts = theme_facts::facts_for("Nebula");
        assert!(lookup(&facts, "org.gnome.desktop.interface", "color-scheme").is_err());
    }

    #[test]
    fn lookup_unknown_key_errs() {
        let facts = theme_facts::facts_for("Nebula");
        assert!(lookup(&facts, APPEARANCE_NS, "no-such-key").is_err());
    }

    #[test]
    fn lookup_color_scheme_ok() {
        let facts = theme_facts::facts_for("Solarized Light");
        assert!(lookup(&facts, APPEARANCE_NS, "color-scheme").is_ok());
    }
}
