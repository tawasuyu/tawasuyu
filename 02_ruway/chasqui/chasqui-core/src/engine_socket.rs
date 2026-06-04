//! Listener Unix-socket que sirve [`chasqui_card::query::QueryRequest`].
//!
//! El daemon `chasqui` lo monta para que cualquier consumer (UI, CLI,
//! otro módulo) pueda preguntarle por sus Mónadas sin pasar por
//! brahman-admin. El path del socket viaja en el `Card.service_socket`
//! del engine; el broker brahman lo enseña vía MatchEvent::Available
//! cuando un consumer declara `flow.input = monad-list:json`.
//!
//! Wire: line-delimited JSON, single-shot por conexión. Mismo patrón
//! que `chasqui-nous` (mock/real ↔ chasqui-core), reutilizado.
//!
//! Threading: un thread dedicado, blocking I/O. No vale la pena traer
//! tokio acá — la frecuencia esperada es muy baja (UI poll cada 2s)
//! y el handler es trivial (lock db → snapshot → write).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chasqui_card::query::{
    EngineInfo, ErrorResponse, FileView, ListMonadsResponse, MonadView, QueryRequest,
    ResolveMonadResponse,
};
use chasqui_card::MonadId;
use chasqui_card::ulid::Ulid;

use crate::db::MonadDb;

/// Configuración del listener.
pub struct ListenerConfig {
    pub socket_path: PathBuf,
    pub engine_id: Ulid,
    pub engine_label: String,
    /// Path del directorio que el daemon está observando, para incluir
    /// en `EngineInfo.watching`. `None` si el daemon no observa nada.
    pub watching: Option<PathBuf>,
}

/// Bind del socket + spawn de un thread con accept loop. Devuelve el
/// path final (útil para confirmar) y un `JoinHandle` para shutdown
/// explícito (drop = thread sigue, listener queda).
///
/// Si el socket ya existe (sesión anterior crasheada), se intenta
/// removerlo antes del bind. Errores de bind se propagan al caller.
pub fn spawn_listener(
    config: ListenerConfig,
    db: Arc<Mutex<MonadDb>>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    if config.socket_path.exists() {
        let _ = std::fs::remove_file(&config.socket_path);
    }
    if let Some(parent) = config.socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(&config.socket_path)?;

    let handle = std::thread::Builder::new()
        .name("chasqui-engine-listener".into())
        .spawn(move || {
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        // Handler sincrónico inline. La frecuencia
                        // esperada (UI poll cada N segundos) no
                        // amerita spawn-per-connection; si en el
                        // futuro hay carga, agregar un threadpool.
                        if let Err(e) = handle_conn(stream, &db, &config) {
                            eprintln!("[engine-socket] conn falló: {e}");
                        }
                    }
                    Err(e) => {
                        eprintln!("[engine-socket] accept falló: {e}");
                    }
                }
            }
        })?;

    Ok(handle)
}

fn handle_conn(
    mut stream: UnixStream,
    db: &Arc<Mutex<MonadDb>>,
    config: &ListenerConfig,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    let n = reader.read_line(&mut line)?;
    if n == 0 {
        return Ok(());
    }

    let resp_bytes = match serde_json::from_str::<QueryRequest>(line.trim()) {
        Ok(QueryRequest::ListMonads) => match handle_list_monads(db, config) {
            Ok(json) => json,
            Err(e) => encode_error(format!("list_monads falló: {e}")),
        },
        Ok(QueryRequest::ResolveMonad { id }) => match handle_resolve_monad(db, id) {
            Ok(json) => json,
            Err(e) => encode_error(format!("resolve_monad falló: {e}")),
        },
        Err(e) => encode_error(format!("JSON inválido: {e}")),
    };

    stream.write_all(resp_bytes.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let _ = stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}

fn handle_list_monads(
    db: &Arc<Mutex<MonadDb>>,
    config: &ListenerConfig,
) -> Result<String, String> {
    let db_lock = db.lock().map_err(|_| "mutex envenenado".to_string())?;
    let monads: Vec<MonadView> = db_lock.monads().map(MonadView::from_manifest).collect();
    let resp = ListMonadsResponse {
        engine: EngineInfo {
            id: config.engine_id,
            label: config.engine_label.clone(),
            watching: config.watching.as_ref().map(|p| p.display().to_string()),
        },
        monads,
    };
    serde_json::to_string(&resp).map_err(|e| format!("encode: {e}"))
}

fn handle_resolve_monad(db: &Arc<Mutex<MonadDb>>, id: MonadId) -> Result<String, String> {
    let db_lock = db.lock().map_err(|_| "mutex envenenado".to_string())?;
    let members: Vec<FileView> = db_lock
        .resolve_members(id)
        .into_iter()
        .map(FileView::from_entry)
        .collect();
    let resp = ResolveMonadResponse { monad: id, members };
    serde_json::to_string(&resp).map_err(|e| format!("encode: {e}"))
}

fn encode_error(msg: String) -> String {
    let err = ErrorResponse { error: msg };
    serde_json::to_string(&err).unwrap_or_else(|_| "{\"error\":\"encode\"}".into())
}

// El cliente blocking vive en `chasqui_card::query::client` — junto a
// los wire types — para que un consumer pueda hablar con el daemon
// importando sólo `chasqui-card`, sin arrastrar el peso de
// `chasqui-core` (scanner / db / sled / notify / walkdir / blake3).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::MonadDb;
    use chasqui_card::query::client as query_client;
    use chasqui_card::MonadManifest;
    use std::time::Duration;

    fn fresh_socket_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        let unique = format!("{}-{}-{}.sock", name, std::process::id(), Ulid::new());
        dir.join(unique)
    }

    #[test]
    fn list_monads_roundtrip_empty() {
        let socket = fresh_socket_path("chasqui-engine-test");
        let db = Arc::new(Mutex::new(MonadDb::new()));
        let engine_id = Ulid::new();
        let _h = spawn_listener(
            ListenerConfig {
                socket_path: socket.clone(),
                engine_id,
                engine_label: "test-engine".into(),
                watching: Some(PathBuf::from("/tmp/x")),
            },
            db.clone(),
        )
        .unwrap();

        // Pequeña espera para que el bind se asiente (en práctica el
        // socket existe inmediatamente tras el bind, pero algunos FS
        // necesitan un tick). Si esto resulta flaky, agregar un loop
        // de wait_for(socket.exists()).
        std::thread::sleep(Duration::from_millis(50));

        let resp = query_client::list_monads(&socket, Duration::from_secs(2)).unwrap();
        assert_eq!(resp.engine.id, engine_id);
        assert_eq!(resp.engine.label, "test-engine");
        assert_eq!(resp.engine.watching.as_deref(), Some("/tmp/x"));
        assert!(resp.monads.is_empty());

        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn list_monads_returns_views() {
        let socket = fresh_socket_path("chasqui-engine-test-views");
        let db = Arc::new(Mutex::new(MonadDb::new()));
        let m1 = MonadManifest::new("alpha");
        let m2 = MonadManifest::new("beta");
        {
            let mut g = db.lock().unwrap();
            g.replace_monads(vec![m1.clone(), m2.clone()]);
        }
        let _h = spawn_listener(
            ListenerConfig {
                socket_path: socket.clone(),
                engine_id: Ulid::new(),
                engine_label: "test".into(),
                watching: None,
            },
            db.clone(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let resp = query_client::list_monads(&socket, Duration::from_secs(2)).unwrap();
        assert_eq!(resp.monads.len(), 2);
        let labels: Vec<_> = resp.monads.iter().map(|m| m.label.as_str()).collect();
        assert!(labels.contains(&"alpha"));
        assert!(labels.contains(&"beta"));

        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn resolve_monad_returns_member_files() {
        use chasqui_card::FileEntry;
        use std::path::PathBuf;

        let socket = fresh_socket_path("chasqui-engine-test-resolve");
        let db = Arc::new(Mutex::new(MonadDb::new()));

        // Dos archivos y una Mónada que los tiene de miembros.
        let f1 = FileEntry {
            id: Ulid::new(),
            path: PathBuf::from("/proj/a.rs"),
            content_hash: None,
            size: 10,
            mtime_ms: 1,
            extension: Some("rs".into()),
        };
        let f2 = FileEntry {
            id: Ulid::new(),
            path: PathBuf::from("/proj/b.rs"),
            content_hash: None,
            size: 20,
            mtime_ms: 2,
            extension: Some("rs".into()),
        };
        let mut m = MonadManifest::new("proj");
        m.members.insert(f1.id);
        m.members.insert(f2.id);
        m.cardinality = 2;
        let monad_id = m.id;
        {
            let mut g = db.lock().unwrap();
            g.ingest_files(vec![f1, f2]);
            g.replace_monads(vec![m]);
        }

        let _h = spawn_listener(
            ListenerConfig {
                socket_path: socket.clone(),
                engine_id: Ulid::new(),
                engine_label: "test".into(),
                watching: None,
            },
            db.clone(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let resp =
            query_client::resolve_monad(&socket, monad_id, Duration::from_secs(2)).unwrap();
        assert_eq!(resp.monad, monad_id);
        assert_eq!(resp.members.len(), 2);
        let paths: Vec<_> = resp.members.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/proj/a.rs"));
        assert!(paths.contains(&"/proj/b.rs"));

        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn invalid_request_returns_error_response() {
        let socket = fresh_socket_path("chasqui-engine-test-bad");
        let db = Arc::new(Mutex::new(MonadDb::new()));
        let _h = spawn_listener(
            ListenerConfig {
                socket_path: socket.clone(),
                engine_id: Ulid::new(),
                engine_label: "test".into(),
                watching: None,
            },
            db.clone(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(50));

        // Bypass del cliente tipado: mandamos JSON inválido a mano.
        use std::io::{BufRead, BufReader, Write};
        let mut stream = UnixStream::connect(&socket).unwrap();
        stream.write_all(b"not json\n").unwrap();
        stream.flush().unwrap();
        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader.read_line(&mut response).unwrap();

        assert!(
            response.contains("\"error\""),
            "esperaba ErrorResponse, got: {response}"
        );

        let _ = std::fs::remove_file(&socket);
    }
}
