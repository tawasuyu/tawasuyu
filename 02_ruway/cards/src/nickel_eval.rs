//! Evaluador Nickel para inputs `.ncl`.
//!
//! El brazo de Cards lee Nickel como **fuente** y produce JSON como
//! **representación intermedia** que después dispatcha por los readers
//! estándar. Esto significa que un `.ncl` puede producir cualquier
//! variant del [`super::CardBody`] siempre que evalúe a una shape JSON
//! que alguno de los readers reconozca.
//!
//! # Templates
//!
//! Nickel soporta `import "..."` y el operador `&` de merge nativo. Un
//! Card "concreto" puede ser un template + override:
//!
//! ```nickel
//! let base = import "ente_basic.ncl" in
//! base & { id = "01ARZ...", label = "mi-ente" }
//! ```
//!
//! **Convención obligatoria del template**: las fields que el usuario
//! va a sobrescribir tienen que estar marcadas `| default` (o
//! `| optional`). Nickel rechaza el merge de dos strings/numbers
//! distintos con la misma prioridad — el `| default` baja la prioridad
//! del template y deja que el override del user gane:
//!
//! ```nickel
//! # template ui_module_basic.ncl
//! {
//!   id | String | default = "TEMPLATE_ID",
//!   label | String | default = "TEMPLATE_LABEL",
//!   # ...
//! }
//! ```
//!
//! Resolución de imports (en orden):
//! 1. Relativo al directorio del archivo input (default de Nickel).
//! 2. `BRAHMAN_CARDS_TEMPLATES_DIR` (env). Permite tener un
//!    registry global de templates accesible por nombre desnudo:
//!    `import "ui_module_basic.ncl"`.
//!
//! No agregamos magic resolución por kind — el autor decide qué
//! template importa explícitamente.

use std::ffi::OsString;
use std::path::Path;

use serde_json::Value;
use thiserror::Error;

/// Variable de entorno opcional. Si está set, su path se agrega al
/// search path de imports de Nickel después del parent dir del input,
/// permitiendo `import "<nombre>.ncl"` desde cualquier ubicación.
pub const BRAHMAN_CARDS_TEMPLATES_ENV: &str = "BRAHMAN_CARDS_TEMPLATES_DIR";

/// Errores específicos del pipeline Nickel. Wrap del error de Nickel
/// formateado como texto plano (sin ANSI) + el path del input para
/// contexto.
#[derive(Debug, Error)]
pub enum NickelEvalError {
    #[error("io leyendo {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("evaluación de '{path}' falló:\n{message}")]
    Eval { path: String, message: String },

    #[error("export a JSON de '{path}' falló:\n{message}")]
    Export { path: String, message: String },

    #[error("JSON exportado por Nickel no parsea de vuelta: {source}")]
    JsonReparse {
        #[source]
        source: serde_json::Error,
    },
}

/// Lee `path` (debe ser un `.ncl` válido), lo evalúa profundamente vía
/// `nickel-lang` y devuelve el resultado como `serde_json::Value`
/// listo para dispatch a un reader JSON.
///
/// El parent dir del input se agrega como import path para que
/// imports relativos tipo `import "./template.ncl"` funcionen sin
/// configuración extra. Si `BRAHMAN_CARDS_TEMPLATES_DIR` está set,
/// también se agrega.
pub fn eval_nickel_file(path: &Path) -> Result<Value, NickelEvalError> {
    let path_display = path.display().to_string();
    let source = std::fs::read_to_string(path).map_err(|e| NickelEvalError::Io {
        path: path_display.clone(),
        source: e,
    })?;

    let mut import_paths: Vec<OsString> = Vec::new();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            import_paths.push(parent.into());
        }
    }
    if let Ok(reg) = std::env::var(BRAHMAN_CARDS_TEMPLATES_ENV) {
        if !reg.is_empty() {
            import_paths.push(reg.into());
        }
    }

    let mut ctx = nickel_lang::Context::new()
        .with_added_import_paths(import_paths)
        .with_source_name(path_display.clone());

    let expr = ctx
        .eval_deep_for_export(&source)
        .map_err(|e| NickelEvalError::Eval {
            path: path_display.clone(),
            message: format_nickel_error(&e),
        })?;

    let json_str = ctx
        .expr_to_json(&expr)
        .map_err(|e| NickelEvalError::Export {
            path: path_display.clone(),
            message: format_nickel_error(&e),
        })?;

    serde_json::from_str(&json_str).map_err(|e| NickelEvalError::JsonReparse { source: e })
}

/// Formatea un error de Nickel como texto plano. Usa `ErrorFormat::Text`
/// (sin ANSI) para que sea legible en logs y mensajes de UI sin
/// escape sequences.
fn format_nickel_error(err: &nickel_lang::Error) -> String {
    let mut buf: Vec<u8> = Vec::new();
    if err
        .format(&mut buf, nickel_lang::ErrorFormat::Text)
        .is_err()
    {
        // Si la propia formateación falla, devolvemos el Debug —
        // peor mensaje que el normal pero no perdemos info.
        return format!("{err:?}");
    }
    String::from_utf8(buf).unwrap_or_else(|_| format!("{err:?}"))
}
