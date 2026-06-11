//! Adapter [`Source`] sobre las **Mónadas vivas del daemon de nouser**
//! (vía el query socket de `chasqui-card`).
//!
//! Hermano de [`crate::nouser::NouserSource`] pero con la fuente de verdad
//! distinta: aquél escanea un directorio *in-process* (determinista, sin
//! daemon); éste consulta el **daemon vivo** —el que vigila el filesystem y
//! re-clusteriza con embeddings reales—. Es la misma ruta que `pata` ya usa
//! para su sidebar de Mónadas (`list_monads` / `resolve_monad`); tenerla detrás
//! del trait [`Source`] unifica el dato: una sola espina, y los exploradores
//! sueltos (chasqui-explorer, el `nouser.rs` de pata) pueden consumirla en vez
//! de re-implementar el cliente.
//!
//! El árbol es de DOS niveles, igual que el scan local: la raíz sintética
//! `@monadas` lista las Mónadas (contenedores), y cada Mónada resuelve sus
//! archivos miembro bajo demanda. La diferencia clave de identidad: como los
//! miembros son **archivos POSIX reales**, el [`NodeId`] de una hoja es su
//! ruta absoluta — así se leen por `std::fs` y heredan el vocabulario de
//! acciones POSIX (abrir-con, reveal) sin puente de tempfile.
//!
//! Detrás de la feature `nouser-daemon`, mucho más liviana que `nouser`: sólo
//! arrastra `chasqui-card` (wire types + cliente) y `card-sidecar`
//! (descubrimiento del socket), nada del engine.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use chasqui_card::query::{client as qclient, transport, MonadView, FLOW_MONAD_LIST, FLOW_TYPE_NAME};
use chasqui_card::MonadId;
use ulid::Ulid;

use crate::{Node, NodeId, NodeKind, Source};

/// Id de la raíz sintética que lista las Mónadas.
const RAIZ: &str = "@monadas";
/// Prefijo de id de una Mónada (contenedor semántico). Las hojas NO llevan
/// prefijo — su id es la ruta POSIX real, para que sean leíbles y openables.
const PREF_MONADA: &str = "m:";

/// Timeout de un query single-shot al daemon (read + write del socket).
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
/// Timeout para descubrir el provider por el broker brahman.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

/// Fuente que navega las Mónadas vivas que sirve el daemon de nouser.
pub struct NouserDaemonSource {
    socket: PathBuf,
    etiqueta: String,
}

impl NouserDaemonSource {
    /// Conecta a un socket de daemon **explícito**. Valida la conexión con un
    /// `list_monads` de prueba (así un socket muerto falla acá y no a mitad de
    /// la navegación). El `label` muestra el directorio observado por el engine
    /// si lo reporta.
    pub fn connect(socket: impl Into<PathBuf>) -> io::Result<Self> {
        let socket = socket.into();
        let resp = qclient::list_monads(&socket, QUERY_TIMEOUT).map_err(io::Error::other)?;
        let etiqueta = resp
            .engine
            .watching
            .clone()
            .map(|w| format!("nouser · {w}"))
            .unwrap_or_else(|| format!("nouser · {}", resp.engine.label));
        Ok(Self { socket, etiqueta })
    }

    /// Descubre el socket del daemon (broker brahman → fallback al path por
    /// defecto) y conecta. Mismo patrón que `pata`/`chasqui-explorer`: registra
    /// una Card consumer y espera el provider; si el broker no responde, cae al
    /// `default_socket_path` si existe.
    pub fn discover() -> io::Result<Self> {
        let socket = descubrir_socket()?;
        Self::connect(socket)
    }

    fn nodo_monada(mv: &MonadView) -> Node {
        let etiqueta = if mv.label.is_empty() {
            mv.path_hint.clone().unwrap_or_else(|| mv.id.to_string())
        } else {
            mv.label.clone()
        };
        // Contenedor sintético: no existe en disco como entidad.
        Node::new(
            format!("{PREF_MONADA}{}", mv.id),
            format!("{etiqueta} ({})", mv.cardinality),
            true,
        )
        .with_kind(NodeKind::Synthetic)
    }

    fn parse_monada(id: &str) -> io::Result<MonadId> {
        let raw = id.strip_prefix(PREF_MONADA).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("id de Mónada inválido: {id}"))
        })?;
        Ulid::from_string(raw).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("ULID de Mónada inválido: {id}"))
        })
    }
}

impl Source for NouserDaemonSource {
    fn label(&self) -> String {
        self.etiqueta.clone()
    }

    fn root(&self) -> Node {
        Node::new(RAIZ, self.etiqueta.clone(), true).with_kind(NodeKind::Synthetic)
    }

    fn children(&self, id: &NodeId) -> io::Result<Vec<Node>> {
        if id == RAIZ {
            let resp = qclient::list_monads(&self.socket, QUERY_TIMEOUT).map_err(io::Error::other)?;
            return Ok(resp.monads.iter().map(Self::nodo_monada).collect());
        }
        if id.starts_with(PREF_MONADA) {
            let mid = Self::parse_monada(id)?;
            let resp = qclient::resolve_monad(&self.socket, mid, QUERY_TIMEOUT)
                .map_err(io::Error::other)?;
            return Ok(resp
                .members
                .iter()
                .map(|f| {
                    let nombre = f
                        .path
                        .rsplit('/')
                        .next()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&f.path)
                        .to_string();
                    // El id es la ruta POSIX real: leíble por fs, openable.
                    let mut nodo = Node::new(f.path.clone(), nombre, false);
                    if f.size > 0 {
                        nodo = nodo.with_size(f.size);
                    }
                    if f.mtime_ms > 0 {
                        nodo = nodo.with_mtime(f.mtime_ms);
                    }
                    nodo
                })
                .collect());
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("una hoja (archivo) no tiene hijos: {id}"),
        ))
    }

    fn read(&self, id: &NodeId) -> io::Result<Vec<u8>> {
        if id == RAIZ || id.starts_with(PREF_MONADA) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("sólo los archivos miembro son leíbles, no {id}"),
            ));
        }
        // El id de una hoja ES su ruta POSIX.
        std::fs::read(id)
    }
}

/// Descubre el socket del daemon: primero el broker brahman (Card consumer +
/// `await_provider_blocking`), luego el `default_socket_path` si el broker no
/// responde y el socket existe. Espejo de `pata::nouser::resolve_socket`.
fn descubrir_socket() -> io::Result<PathBuf> {
    let card = card_sidecar::build_consumer_card(
        "nahual-source-core",
        FLOW_MONAD_LIST,
        FLOW_TYPE_NAME,
    );
    match card_sidecar::await_provider_blocking(card, DISCOVERY_TIMEOUT) {
        Ok(p) => Ok(p),
        Err(broker_err) => {
            let fallback = transport::default_socket_path();
            if fallback.exists() {
                Ok(fallback)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("broker: {broker_err}; fallback {} no existe", fallback.display()),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::thread;

    /// Levanta un **daemon-mock** sobre un socket Unix real: acepta `n`
    /// conexiones, lee la línea de request y responde el JSON canónico según el
    /// `kind`. Devuelve el path del socket (vive dentro del TempDir).
    fn mock_daemon(dir: &std::path::Path, monada_id: Ulid, conns: usize) -> PathBuf {
        let socket = dir.join("engine.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let mid = monada_id;
        thread::spawn(move || {
            for _ in 0..conns {
                let Ok((stream, _)) = listener.accept() else { break };
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                // El wire es `{"kind":"list_monads"}` / `{"kind":"resolve_monad",…}`.
                // Discriminar por substring evita un dev-dep de serde_json.
                let resp = if line.contains("resolve_monad") {
                    format!(
                        r#"{{"monad":"{mid}","members":[{{"id":"{f1}","path":"/proj/src/lib.rs","size":920,"mtime_ms":1000}},{{"id":"{f2}","path":"/proj/src/main.rs","size":1840,"mtime_ms":2000}}]}}"#,
                        mid = mid,
                        f1 = Ulid::nil(),
                        f2 = Ulid::nil(),
                    )
                } else {
                    format!(
                        r#"{{"engine":{{"id":"{eng}","label":"test","watching":"/proj"}},"monads":[{{"id":"{mid}","label":"src","cardinality":2}}]}}"#,
                        eng = Ulid::nil(),
                        mid = mid,
                    )
                };
                let mut w = reader.into_inner();
                let _ = w.write_all(resp.as_bytes());
                let _ = w.write_all(b"\n");
                let _ = w.flush();
            }
        });
        // Pequeña espera a que el bind quede listo no hace falta: bind ya
        // ocurrió arriba (sincrónico) antes de spawnear el accept loop.
        socket
    }

    #[test]
    fn navega_monadas_del_daemon_mock() {
        let dir = tempfile::tempdir().unwrap();
        let mid = Ulid::from_string("00000000000000000000000001").unwrap();
        // 2 conexiones: una para connect()/list inicial reusada por root.children
        // — connect hace 1 list, children(root) hace otro list, children(monada)
        // hace 1 resolve → 3 en total.
        let socket = mock_daemon(dir.path(), mid, 3);

        let src = NouserDaemonSource::connect(&socket).unwrap();
        assert!(src.label().contains("/proj"));

        let root = src.root();
        assert_eq!(root.id, RAIZ);
        assert!(root.is_container);

        let monadas = src.children(&root.id).unwrap();
        assert_eq!(monadas.len(), 1);
        assert_eq!(monadas[0].id, format!("{PREF_MONADA}{mid}"));
        assert!(monadas[0].is_container);
        assert!(monadas[0].name.starts_with("src"));

        let archivos = src.children(&monadas[0].id).unwrap();
        assert_eq!(archivos.len(), 2);
        let lib = archivos.iter().find(|n| n.name == "lib.rs").unwrap();
        assert!(!lib.is_container);
        assert_eq!(lib.id, "/proj/src/lib.rs"); // id = ruta POSIX real
        assert_eq!(lib.size, Some(920));
        assert_eq!(lib.mtime, Some(1000));
    }

    #[test]
    fn read_de_monada_o_raiz_es_error() {
        let dir = tempfile::tempdir().unwrap();
        let mid = Ulid::from_string("00000000000000000000000002").unwrap();
        let socket = mock_daemon(dir.path(), mid, 1);
        let src = NouserDaemonSource::connect(&socket).unwrap();
        // La hoja se lee por su path POSIX (no por el daemon): un path
        // inexistente da error de fs; la raíz/Mónada dan InvalidInput.
        assert!(src.read(&RAIZ.to_string()).is_err());
        assert!(src.read(&format!("{PREF_MONADA}{mid}")).is_err());
    }

    #[test]
    fn connect_a_socket_muerto_es_error() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("no-existe.sock");
        assert!(NouserDaemonSource::connect(&socket).is_err());
    }

    #[test]
    fn parse_monada_rechaza_basura() {
        assert!(NouserDaemonSource::parse_monada("sin-prefijo").is_err());
        assert!(NouserDaemonSource::parse_monada("m:no-es-ulid").is_err());
    }
}
