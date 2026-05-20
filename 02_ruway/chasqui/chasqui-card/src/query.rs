//! Wire types para consultar al daemon `chasqui` por sus Mónadas.
//!
//! El daemon expone un Unix socket (cuyo path se publica en
//! `Card.service_socket` y se descubre vía broker MatchEvent). Cada
//! conexión es single-shot: una request JSON terminada en `\n`,
//! una response JSON terminada en `\n`, cierre.
//!
//! Mismo patrón que `chasqui-nous` (mock/real ↔ chasqui-core), reusado
//! ahora para que la UI (`chasqui-explorer`) descubra y consulte al
//! daemon sin hardcodear sockets ni pasar por brahman-admin.
//!
//! ## Contrato
//!
//! ```text
//! C → S: {"kind":"list_monads"}\n
//! S → C: {"engine":{...},"monads":[...]}\n
//! ```
//!
//! En caso de error:
//!
//! ```text
//! S → C: {"error":"unsupported kind"}\n
//! ```

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ulid::Ulid;

use crate::{Lens, MonadId, MonadManifest};

// =====================================================================
// Constants compartidos para el broker brahman
// =====================================================================

/// Nombre del flow output del daemon (input del consumer/explorer).
pub const FLOW_MONAD_LIST: &str = "monad-list";

/// Tipo del flow: el wire es JSON, así que el TypeRef es `primitive::json`.
pub const FLOW_TYPE_NAME: &str = "json";

// =====================================================================
// Wire request
// =====================================================================

/// Request al daemon. El wire es JSON line-delimited (un objeto + `\n`
/// por conexión).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueryRequest {
    /// Lista todas las Mónadas vivas del daemon, junto con metadata
    /// del engine. Pensado para que la UI haga snapshot polling.
    ListMonads,
}

// =====================================================================
// Wire response
// =====================================================================

/// Response a `ListMonads`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMonadsResponse {
    /// Datos del engine (la Card que es "dueña" de las Mónadas).
    pub engine: EngineInfo,
    /// Mónadas vivas en este momento. Vista slim sin centroide ni
    /// member set para que el wire sea liviano: una Mónada con 50k
    /// archivos no debe transmitir 50k ULIDs cada poll.
    pub monads: Vec<MonadView>,
}

/// Identidad del engine (Card kind=Ente que owns las Mónadas).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineInfo {
    pub id: Ulid,
    pub label: String,
    /// Path del directorio que el daemon está observando. `None` si
    /// el daemon corre sin watcher.
    #[serde(default)]
    pub watching: Option<String>,
}

/// Vista slim de una Mónada — los campos que la UI necesita para
/// renderizar una card sin pull del centroide ni del member set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonadView {
    pub id: MonadId,
    pub label: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub cardinality: u32,
    #[serde(default)]
    pub entropy: f32,
    #[serde(default)]
    pub dominant_lens: Lens,
    #[serde(default)]
    pub path_hint: Option<String>,
    #[serde(default)]
    pub centroid_model: Option<String>,
}

impl MonadView {
    /// Proyecta un MonadManifest completo a su vista slim para wire.
    pub fn from_manifest(m: &MonadManifest) -> Self {
        Self {
            id: m.id,
            label: m.label.clone(),
            summary: m.summary.clone(),
            keywords: m.keywords.clone(),
            cardinality: m.cardinality,
            entropy: m.entropy,
            dominant_lens: m.dominant_lens,
            path_hint: m.path_hint.clone(),
            centroid_model: m.centroid_model.clone(),
        }
    }
}

/// Error de protocolo retornado en lugar de la response normal.
#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("chasqui-engine: {error}")]
pub struct ErrorResponse {
    pub error: String,
}

// =====================================================================
// Transport
// =====================================================================

pub mod transport {
    use std::path::PathBuf;

    /// Variable de entorno para sobreescribir la ruta del socket del
    /// daemon (útil para tests / multi-daemon).
    pub const SOCKET_ENV: &str = "NOUSER_ENGINE_SOCKET";

    /// Nombre por defecto del socket.
    pub const SOCKET_NAME: &str = "chasqui-engine.sock";

    /// Ruta canónica al socket del daemon. Honra `NOUSER_ENGINE_SOCKET`
    /// si está set, sino arma sobre `$XDG_RUNTIME_DIR` (con fallback
    /// `$TMPDIR`).
    pub fn default_socket_path() -> PathBuf {
        if let Ok(p) = std::env::var(SOCKET_ENV) {
            return PathBuf::from(p);
        }
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(SOCKET_NAME)
    }
}

// =====================================================================
// Cliente blocking — vive con los wire types para que un consumer
// (UI, CLI, otro módulo) pueda hablar con el daemon importando sólo
// `chasqui-card`, sin arrastrar `chasqui-core` (notify/walkdir/sled/blake3).
// =====================================================================

/// Cliente síncrono para el query socket del daemon. Sólo Unix (el
/// resto del ecosistema brahman es Unix-only de facto).
#[cfg(unix)]
pub mod client {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::time::Duration;

    use super::{ErrorResponse, ListMonadsResponse, QueryRequest};

    #[derive(Debug, thiserror::Error)]
    pub enum QueryError {
        #[error("conectar a {path}: {source}")]
        Connect {
            path: std::path::PathBuf,
            #[source]
            source: std::io::Error,
        },
        #[error("I/O: {0}")]
        Io(#[from] std::io::Error),
        #[error("serializacion: {0}")]
        Serde(#[from] serde_json::Error),
        #[error("daemon: {0}")]
        Daemon(String),
        #[error("response vacía del daemon")]
        Empty,
    }

    /// Envía `ListMonads` al daemon en `socket` y devuelve la response.
    /// `timeout` se aplica tanto al read como al write del stream.
    pub fn list_monads(
        socket: &Path,
        timeout: Duration,
    ) -> Result<ListMonadsResponse, QueryError> {
        let mut stream = UnixStream::connect(socket).map_err(|e| QueryError::Connect {
            path: socket.to_path_buf(),
            source: e,
        })?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;

        let req = QueryRequest::ListMonads;
        let line = serde_json::to_string(&req)?;
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        let n = reader.read_line(&mut response)?;
        if n == 0 {
            return Err(QueryError::Empty);
        }

        if let Ok(resp) = serde_json::from_str::<ListMonadsResponse>(response.trim()) {
            return Ok(resp);
        }
        let err: ErrorResponse = serde_json::from_str(response.trim())?;
        Err(QueryError::Daemon(err.error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrips_json_with_tag() {
        let req = QueryRequest::ListMonads;
        let s = serde_json::to_string(&req).unwrap();
        assert_eq!(s, r#"{"kind":"list_monads"}"#);
        let back: QueryRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn response_roundtrip_preserves_view() {
        let m = MonadManifest::new("x/src");
        let view = MonadView::from_manifest(&m);
        let resp = ListMonadsResponse {
            engine: EngineInfo {
                id: Ulid::new(),
                label: "brahman.nouser_engine".into(),
                watching: Some("/tmp/x".into()),
            },
            monads: vec![view.clone()],
        };
        let s = serde_json::to_string(&resp).unwrap();
        let back: ListMonadsResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(back.monads.len(), 1);
        assert_eq!(back.monads[0].label, view.label);
        assert_eq!(back.engine.label, "brahman.nouser_engine");
    }

    #[test]
    fn view_is_slim_no_centroid_no_members() {
        // Construimos una Mónada con centroid + members "pesados",
        // proyectamos a view, verificamos que esos campos no viajan.
        let mut m = MonadManifest::new("test");
        m.centroid = vec![0.1; 384]; // peso "real-fastembed"
        m.members.insert(Ulid::new());
        m.members.insert(Ulid::new());
        m.cardinality = 2;
        let view = MonadView::from_manifest(&m);
        let s = serde_json::to_string(&view).unwrap();
        // Chequeo con `:` para distinguir el field "centroid" del
        // field "centroid_model" (que sí es metadata liviana y debe ir).
        assert!(
            !s.contains("\"centroid\":"),
            "MonadView no debe serializar el vector centroid: {s}"
        );
        assert!(
            !s.contains("\"members\":"),
            "MonadView no debe serializar members: {s}"
        );
        assert!(s.contains("\"cardinality\":2"), "cardinality sí va: {s}");
    }
}
