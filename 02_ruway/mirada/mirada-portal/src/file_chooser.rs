//! Interfaz `org.freedesktop.impl.portal.FileChooser` del backend mirada.
//!
//! Implementa `OpenFile` y `SaveFile`: cuando una app ajena pide elegir un
//! archivo vía `xdg-desktop-portal`, el frontend nos rutea acá y nosotros
//! **lanzamos `mirada-filechooser`** (la ventana Llimphi del diálogo) como
//! subproceso. No pintamos nada en este proceso: el portal corre en un
//! runtime `tokio current_thread` y `llimphi_ui::run` quiere el hilo
//! principal con su event loop propio — por eso el diálogo vive aparte y se
//! comunica por un archivo de resultado.
//!
//! Protocolo con el subproceso: le pasamos la petición por flags y un
//! `--out <archivo>`; al cerrarse, ese archivo trae el JSON
//! `{response, uris, current_name}`. Lo leemos y lo devolvemos por D-Bus
//! como `(u response, a{sv} results)` con la clave `uris`.
//!
//! Limitación conocida: la opción `current_folder` (tipo `ay`) todavía no
//! se honra — el diálogo arranca en `$HOME`. El resto de opciones usuales
//! (`multiple`, `accept_label`, `current_name`) sí.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::{info, warn};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{fdo, interface};

/// Estado nulo: cada llamada es independiente y spawnea su propio diálogo.
pub struct FileChooserPortal;

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooserPortal {
    /// Versión de la interfaz impl que soportamos.
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        3
    }

    /// `OpenFile(o handle, s app_id, s parent_window, s title, a{sv} options)
    /// -> (u response, a{sv} results)`. Elegir uno o varios archivos
    /// existentes.
    async fn open_file(
        &self,
        _handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let multiple = opt_bool(&options, "multiple").unwrap_or(false);
        let accept = opt_string(&options, "accept_label").unwrap_or_default();
        info!(%app_id, multiple, "FileChooser.OpenFile");
        run_dialog(false, multiple, &title, &accept, "").await
    }

    /// `SaveFile(...) -> (u response, a{sv} results)`. Tipear un nombre nuevo
    /// para guardar.
    async fn save_file(
        &self,
        _handle: OwnedObjectPath,
        app_id: String,
        _parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> fdo::Result<(u32, HashMap<String, OwnedValue>)> {
        let accept = opt_string(&options, "accept_label").unwrap_or_default();
        let current_name = opt_string(&options, "current_name").unwrap_or_default();
        info!(%app_id, %current_name, "FileChooser.SaveFile");
        run_dialog(true, false, &title, &accept, &current_name).await
    }
}

// ============================================================================
// Lanzamiento del subproceso
// ============================================================================

/// Lanza `mirada-filechooser`, espera a que cierre y traduce su archivo de
/// resultado a la respuesta del portal. Nunca falla "hacia arriba": ante
/// cualquier problema responde como cancelado (response 1).
async fn run_dialog(
    save: bool,
    multiple: bool,
    title: &str,
    accept_label: &str,
    current_name: &str,
) -> fdo::Result<(u32, HashMap<String, OwnedValue>)> {
    let out = unique_out_path();
    let bin = filechooser_bin();

    let mut cmd = tokio::process::Command::new(&bin);
    cmd.arg("--mode").arg(if save { "save" } else { "open" });
    if multiple {
        cmd.arg("--multiple");
    }
    if !title.is_empty() {
        cmd.arg("--title").arg(title);
    }
    if !accept_label.is_empty() {
        cmd.arg("--accept-label").arg(accept_label);
    }
    if !current_name.is_empty() {
        cmd.arg("--current-name").arg(current_name);
    }
    cmd.arg("--out").arg(&out);

    match cmd.status().await {
        Ok(status) => info!(?bin, code = status.code(), "diálogo terminó"),
        Err(e) => {
            warn!(?bin, ?e, "no se pudo lanzar mirada-filechooser");
            return Ok((1, HashMap::new()));
        }
    }

    let result = read_result(&out);
    let _ = std::fs::remove_file(&out);
    Ok(result)
}

/// Lee el JSON de resultado dejado por el subproceso y arma el `(response,
/// results)`. Archivo ausente o ilegible = cancelado.
fn read_result(out: &PathBuf) -> (u32, HashMap<String, OwnedValue>) {
    let bytes = match std::fs::read(out) {
        Ok(b) => b,
        Err(_) => return (1, HashMap::new()),
    };
    let json: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => return (1, HashMap::new()),
    };

    let response = json.get("response").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
    let uris: Vec<String> = json
        .get("uris")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let mut results = HashMap::new();
    if response == 0 && !uris.is_empty() {
        if let Ok(v) = OwnedValue::try_from(Value::from(uris)) {
            results.insert("uris".to_string(), v);
        }
    }
    (response, results)
}

/// Path del binario del diálogo: primero junto al ejecutable del portal
/// (instalación lado a lado), sino confiando en el `PATH`.
fn filechooser_bin() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("mirada-filechooser")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("mirada-filechooser"))
}

/// Archivo de resultado único por invocación (pid + contador en memoria).
fn unique_out_path() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join(format!("mirada-fc-{pid}-{n}.json"))
}

// ============================================================================
// Extracción de opciones a{sv}
// ============================================================================

fn opt_bool(opts: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    opts.get(key).and_then(|o| match &**o {
        Value::Bool(b) => Some(*b),
        _ => None,
    })
}

fn opt_string(opts: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    opts.get(key).and_then(|o| match &**o {
        Value::Str(s) => Some(s.as_str().to_string()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mirada-portal-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn result_ok_with_uris() {
        let p = tmp("ok.json");
        std::fs::write(
            &p,
            br#"{"response":0,"uris":["file:///home/a/x.txt"],"current_name":""}"#,
        )
        .unwrap();
        let (resp, results) = read_result(&p);
        assert_eq!(resp, 0);
        assert!(results.contains_key("uris"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn result_cancelled_carries_no_uris() {
        let p = tmp("cancel.json");
        std::fs::write(&p, br#"{"response":1,"uris":[]}"#).unwrap();
        let (resp, results) = read_result(&p);
        assert_eq!(resp, 1);
        assert!(results.is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn result_missing_file_is_cancelled() {
        let (resp, results) = read_result(&tmp("nope.json"));
        assert_eq!(resp, 1);
        assert!(results.is_empty());
    }

    #[test]
    fn options_extraction() {
        use zbus::zvariant::Value;
        let mut opts: HashMap<String, OwnedValue> = HashMap::new();
        opts.insert("multiple".into(), OwnedValue::try_from(Value::Bool(true)).unwrap());
        opts.insert(
            "accept_label".into(),
            OwnedValue::try_from(Value::from("Elegir")).unwrap(),
        );
        assert_eq!(opt_bool(&opts, "multiple"), Some(true));
        assert_eq!(opt_string(&opts, "accept_label").as_deref(), Some("Elegir"));
        assert_eq!(opt_bool(&opts, "ausente"), None);
    }
}
