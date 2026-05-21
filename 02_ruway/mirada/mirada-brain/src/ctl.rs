//! `ctl` — el API de control externo del Cerebro.
//!
//! Mientras el keymap ([`crate::keymap`]) es la cara *configurable* de las
//! acciones, este módulo es su cara *programable*: deja que otro proceso
//! —un script, una taskbar, el binario `mirada-ctl`— dispare una
//! [`DesktopAction`] o consulte el estado, sin tocar el teclado.
//!
//! Todo converge igualmente en `Desktop::apply`: una petición de control
//! no es más que otro front-end del mismo embudo. El transporte es un
//! socket Unix de petición/respuesta, con el marco `postcard` que ya usa
//! [`mirada_protocol`]; `DesktopAction` viaja como enum serializado (no
//! como cadena), así que el contrato es tipado de punta a punta.
//!
//! - El Cerebro abre un [`CtlServer`] y atiende [`CtlConn`]s en su bucle.
//! - El cliente usa [`send_request`] — una petición, una respuesta, cierra.

use std::io::{self, ErrorKind};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mirada_layout::WindowId;
use mirada_protocol::{read_frame, write_frame};

use crate::action::DesktopAction;

/// Una orden de un cliente de control al Cerebro.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CtlRequest {
    /// Aplica una acción de escritorio — el equivalente a pulsar su atajo.
    Do(DesktopAction),
    /// Pide la lista de ventanas conocidas, en todos los escritorios.
    ListWindows,
}

/// La respuesta del Cerebro a un [`CtlRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CtlReply {
    /// La orden se aplicó.
    Ok,
    /// La orden no se pudo aplicar; el motivo, para mostrar al usuario.
    Error(String),
    /// La lista pedida con [`CtlRequest::ListWindows`].
    Windows(Vec<WindowLine>),
}

/// Una ventana en la vista de `mirada-ctl windows`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowLine {
    /// Id de la ventana — el que se pasa a `focus-window:N`.
    pub id: WindowId,
    pub app_id: String,
    pub title: String,
    /// Escritorio virtual donde está (1-based); `0` = guardada en el
    /// scratchpad, en ningún escritorio.
    pub workspace: usize,
    /// `true` si es la ventana enfocada del escritorio activo.
    pub focused: bool,
}

/// La ruta del socket de control: `$XDG_RUNTIME_DIR/mirada-ctl.sock`, o
/// el directorio temporal si esa variable no está.
pub fn default_socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join("mirada-ctl.sock")
}

/// El extremo servidor del API de control — lo abre el dueño del
/// [`Desktop`](crate::Desktop) (la app `mirada`, o `mirada-compositor`
/// con el Cerebro embebido).
pub struct CtlServer {
    listener: UnixListener,
    path: PathBuf,
}

impl CtlServer {
    /// Abre el socket de control en `path`. Si ya hay un Cerebro vivo
    /// escuchando ahí, falla; si encuentra un socket muerto (de un
    /// compositor anterior), lo retira y se queda con él.
    pub fn bind(path: &Path) -> io::Result<Self> {
        if path.exists() {
            if UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    ErrorKind::AddrInUse,
                    "ya hay un Cerebro escuchando en el socket de control",
                ));
            }
            let _ = std::fs::remove_file(path);
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, path: path.to_path_buf() })
    }

    /// Acepta una conexión pendiente sin bloquear. `None` si no hay
    /// ninguna — pensado para llamarse cada vuelta del bucle de eventos.
    pub fn poll(&self) -> Option<CtlConn> {
        match self.listener.accept() {
            Ok((stream, _)) => Some(CtlConn { stream }),
            Err(_) => None,
        }
    }
}

impl Drop for CtlServer {
    fn drop(&mut self) {
        // Dejar el socket limpio para el próximo arranque.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Una conexión de control aceptada: una petición y una respuesta.
pub struct CtlConn {
    stream: UnixStream,
}

impl CtlConn {
    /// Lee la petición del cliente (bloquea hasta el marco completo; es
    /// uno solo y llega enseguida).
    pub fn read_request(&mut self) -> io::Result<Option<CtlRequest>> {
        self.stream.set_nonblocking(false)?;
        read_frame(&mut self.stream)
    }

    /// Envía la respuesta. El cliente cierra al recibirla.
    pub fn reply(&mut self, reply: &CtlReply) -> io::Result<()> {
        write_frame(&mut self.stream, reply)
    }
}

/// Envía una petición al Cerebro y espera su respuesta. Es el camino que
/// usa el binario `mirada-ctl`: conecta, pregunta, cierra.
pub fn send_request(path: &Path, request: &CtlRequest) -> io::Result<CtlReply> {
    let mut stream = UnixStream::connect(path)?;
    write_frame(&mut stream, request)?;
    read_frame(&mut stream)?
        .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "el Cerebro cerró sin responder"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Una ruta de socket única para un test (los sockets no se pueden
    /// reabrir; cada test necesita la suya).
    fn temp_socket(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("mirada-ctl-test-{tag}-{nanos}.sock"))
    }

    #[test]
    fn default_socket_path_lives_under_a_runtime_dir() {
        let p = default_socket_path();
        assert_eq!(p.file_name().unwrap(), "mirada-ctl.sock");
    }

    #[test]
    fn a_request_round_trips_over_the_socket() {
        let path = temp_socket("roundtrip");
        let server = CtlServer::bind(&path).unwrap();

        // El "Cerebro": atiende una petición y responde.
        let srv = thread::spawn(move || loop {
            if let Some(mut conn) = server.poll() {
                let req = conn.read_request().unwrap().unwrap();
                let reply = match req {
                    CtlRequest::Do(DesktopAction::FocusNext) => CtlReply::Ok,
                    other => CtlReply::Error(format!("inesperado: {other:?}")),
                };
                conn.reply(&reply).unwrap();
                return;
            }
            thread::yield_now();
        });

        let reply = send_request(&path, &CtlRequest::Do(DesktopAction::FocusNext)).unwrap();
        assert_eq!(reply, CtlReply::Ok);
        srv.join().unwrap();
    }

    #[test]
    fn list_windows_carries_the_window_lines() {
        let path = temp_socket("windows");
        let server = CtlServer::bind(&path).unwrap();
        let lines = vec![WindowLine {
            id: 7,
            app_id: "org.brahman.shuma".into(),
            title: "shell".into(),
            workspace: 2,
            focused: true,
        }];
        let expected = lines.clone();

        let srv = thread::spawn(move || loop {
            if let Some(mut conn) = server.poll() {
                assert_eq!(conn.read_request().unwrap().unwrap(), CtlRequest::ListWindows);
                conn.reply(&CtlReply::Windows(lines)).unwrap();
                return;
            }
            thread::yield_now();
        });

        let reply = send_request(&path, &CtlRequest::ListWindows).unwrap();
        assert_eq!(reply, CtlReply::Windows(expected));
        srv.join().unwrap();
    }

    #[test]
    fn binding_twice_on_a_live_socket_is_refused() {
        let path = temp_socket("dup");
        let _first = CtlServer::bind(&path).unwrap();
        // El primero sigue vivo: el segundo debe rechazarse.
        assert!(CtlServer::bind(&path).is_err());
    }
}
