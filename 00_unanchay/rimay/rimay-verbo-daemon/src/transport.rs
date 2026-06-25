//! Transporte del daemon verbo, abstraído por plataforma.
//!
//! El encuadre ([`crate::wire`]) es agnóstico del transporte: opera sobre
//! cualquier `AsyncRead + AsyncWrite`. Lo único atado al SO es *cómo* se
//! abre el canal. Por eso este módulo expone una API única por `path` y
//! elige la implementación con `cfg`:
//!
//! - **Unix**: un socket Unix en `path`, dentro de `$XDG_RUNTIME_DIR`. La
//!   seguridad la dan los permisos de filesystem (un socket por usuario).
//!   Comportamiento idéntico al histórico — no se toca.
//! - **No-Unix (Windows…)**: `tokio` no expone sockets Unix ahí, así que
//!   el transporte es TCP en loopback. El servidor toma un puerto efímero
//!   y lo publica en un sidecar `<path>.port`; el cliente lo lee para
//!   conectar. La API sigue siendo por `path`, así que el resto del crate
//!   (server/client) no distingue plataforma.
//!
//! El loopback (`127.0.0.1`) no es tan estanco como los permisos de un
//! socket Unix —cualquier proceso local puede tocar el puerto—, pero en
//! Windows es el equivalente práctico para un daemon per-usuario, y el
//! sidecar de puerto evita colisiones entre daemons (uno por modelo).

#[cfg(unix)]
pub use unix_impl::{connect, Listener, Stream};

#[cfg(not(unix))]
pub use tcp_impl::{connect, Listener, Stream};

#[cfg(unix)]
mod unix_impl {
    use std::io;
    use std::path::{Path, PathBuf};

    use tokio::net::{UnixListener, UnixStream};

    /// El stream por conexión: lo consume [`crate::wire`] sin saber el tipo.
    pub type Stream = UnixStream;

    /// Listener del daemon ligado a un socket Unix.
    pub struct Listener {
        inner: UnixListener,
        path: PathBuf,
    }

    impl Listener {
        /// Bindea el socket Unix en `path`, removiendo un huérfano previo.
        pub fn bind(path: &Path) -> io::Result<Self> {
            let _ = std::fs::remove_file(path);
            Ok(Self {
                inner: UnixListener::bind(path)?,
                path: path.to_path_buf(),
            })
        }

        /// Acepta una conexión entrante.
        pub async fn accept(&self) -> io::Result<Stream> {
            let (stream, _) = self.inner.accept().await?;
            Ok(stream)
        }

        /// Libera el recurso de nombre (el archivo de socket).
        pub fn cleanup(&self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// Abre una conexión cliente al socket en `path`.
    pub async fn connect(path: &Path) -> io::Result<Stream> {
        UnixStream::connect(path).await
    }
}

#[cfg(not(unix))]
mod tcp_impl {
    use std::io;
    use std::path::{Path, PathBuf};

    use tokio::net::{TcpListener, TcpStream};

    /// El stream por conexión: lo consume [`crate::wire`] sin saber el tipo.
    pub type Stream = TcpStream;

    /// Listener del daemon ligado a un puerto TCP de loopback, publicado
    /// en el sidecar `<path>.port`.
    pub struct Listener {
        inner: TcpListener,
        port_file: PathBuf,
    }

    impl Listener {
        /// Toma un puerto efímero en `127.0.0.1` y publica su número en
        /// `<path>.port` para que el cliente lo descubra.
        pub fn bind(path: &Path) -> io::Result<Self> {
            // `Daemon::bind` es síncrono; usamos el listener std (bind
            // síncrono) y lo adoptamos al runtime tokio.
            let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
            std_listener.set_nonblocking(true)?;
            let port = std_listener.local_addr()?.port();
            let port_file = port_file_for(path);
            std::fs::write(&port_file, port.to_string())?;
            Ok(Self {
                inner: TcpListener::from_std(std_listener)?,
                port_file,
            })
        }

        /// Acepta una conexión entrante (con `TCP_NODELAY` para round-trips
        /// chicos sin latencia de Nagle).
        pub async fn accept(&self) -> io::Result<Stream> {
            let (stream, _) = self.inner.accept().await?;
            let _ = stream.set_nodelay(true);
            Ok(stream)
        }

        /// Libera el recurso de nombre (el sidecar de puerto).
        pub fn cleanup(&self) {
            let _ = std::fs::remove_file(&self.port_file);
        }
    }

    /// Abre una conexión cliente leyendo el puerto del sidecar `<path>.port`.
    /// Si el sidecar no existe se reporta `NotFound` — que el cliente trata
    /// como transitorio (daemon ausente → fallback a Mock).
    pub async fn connect(path: &Path) -> io::Result<Stream> {
        let port_file = port_file_for(path);
        let txt = std::fs::read_to_string(&port_file).map_err(|_| {
            io::Error::new(io::ErrorKind::NotFound, "sidecar de puerto verbo ausente")
        })?;
        let port: u16 = txt.trim().parse().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "puerto verbo inválido en sidecar")
        })?;
        let stream = TcpStream::connect(("127.0.0.1", port)).await?;
        let _ = stream.set_nodelay(true);
        Ok(stream)
    }

    /// Deriva la ruta del sidecar de puerto a partir del `path` lógico.
    fn port_file_for(path: &Path) -> PathBuf {
        let mut s = path.as_os_str().to_os_string();
        s.push(".port");
        PathBuf::from(s)
    }
}
