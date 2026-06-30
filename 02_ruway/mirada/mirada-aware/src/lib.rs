//! `mirada-aware` — el protocolo por el que una app **consciente de mirada**
//! contribuye botones a **su propia** barra de título y se entera de los clicks.
//!
//! El compositor pinta y rutea los botones de sistema (cerrar, etc.); esto deja
//! que una app sume **acciones propias** (p. ej. un cuaderno: «correr todo») sin
//! que el compositor sepa nada de su dominio.
//!
//! **Transporte:** un socket Unix de **petición/respuesta** (marco `postcard`),
//! igual que `mirada-ctl` — sin conexiones persistentes ni framing
//! no-bloqueante. Es **stateless por conexión**: el compositor guarda las
//! contribuciones y los clicks pendientes indexados por `app_id`, y el cliente
//! conecta-pregunta-cierra cada vez.
//!
//! **Identidad:** la contribución se asocia al `app_id` que el cliente declara
//! (el mismo que fija por `xdg_toplevel.set_app_id`). Es un feature
//! **cooperativo**: el peor caso es que una app le ponga botones a otra. Se
//! puede endurecer más adelante casando el ejecutable real (`SO_PEERCRED`).
//!
//! **Lazo:** el cliente (1) `register(app_id, items)` al abrir; (2) cada tanto
//! `poll_clicks(app_id)` y reacciona a los [`AwareClick`]; (3) `unregister` al
//! cerrar (o el compositor los descarta cuando no quedan ventanas de ese
//! `app_id`).

use std::io::{self, ErrorKind};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use mirada_protocol::{read_frame, write_frame};

/// El grupo de la barra donde cae un botón aportado por la app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AwareSide {
    /// Pegado al grupo izquierdo (tras los botones de sistema de ese lado).
    Left,
    /// Pegado al grupo derecho, **a la izquierda** de los de sistema
    /// (cerrar/maximizar quedan siempre al borde). El default.
    #[default]
    Right,
}

/// Un botón que una app aporta a su barra de título.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwareItem {
    /// Id estable del botón (lo elige la app); vuelve en el [`AwareClick`].
    pub id: String,
    /// Glifo/emoji a pintar (la app lo elige; p. ej. "▶", "★", "⟳").
    pub glyph: String,
    /// Texto para tooltip/accesibilidad (hoy informativo).
    pub label: String,
    /// En qué grupo de la barra va.
    #[serde(default)]
    pub side: AwareSide,
}

impl AwareItem {
    /// Un botón a la derecha (el lado por defecto) con glifo y label.
    pub fn new(id: impl Into<String>, glyph: impl Into<String>, label: impl Into<String>) -> Self {
        Self { id: id.into(), glyph: glyph.into(), label: label.into(), side: AwareSide::default() }
    }
}

/// Un click sobre un botón aportado, que el compositor encola para la app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwareClick {
    /// El [`AwareItem::id`] clickeado.
    pub item_id: String,
    /// El id de ventana del compositor donde se clickeó (token opaco para la
    /// app: estable mientras la ventana viva).
    pub window: u64,
    /// El título de esa ventana al momento del click — para que la app
    /// identifique **cuál** de sus documentos/ventanas fue, sin conocer ids del
    /// compositor.
    pub window_title: String,
}

/// Petición de un cliente mirada-aware al compositor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AwareRequest {
    /// Fija (reemplaza) los botones que la app `app_id` aporta a su barra.
    Register { app_id: String, items: Vec<AwareItem> },
    /// Retira las contribuciones de `app_id`.
    Unregister { app_id: String },
    /// Pide (y vacía) los clicks pendientes para `app_id`.
    PollClicks { app_id: String },
}

/// Respuesta del compositor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AwareReply {
    /// La petición se aplicó.
    Ok,
    /// Los clicks pendientes (respuesta a [`AwareRequest::PollClicks`]).
    Clicks(Vec<AwareClick>),
}

/// La ruta del socket mirada-aware. Espeja la lógica de `mirada-ctl`:
/// `$MIRADA_AWARE_SOCK` > `$XDG_RUNTIME_DIR/mirada-aware.sock` >
/// `/run/user/<uid>/mirada-aware.sock`.
pub fn default_socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("MIRADA_AWARE_SOCK") {
        return PathBuf::from(p);
    }
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("mirada-aware.sock");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(meta) = std::fs::metadata("/proc/self") {
            let run = PathBuf::from(format!("/run/user/{}", meta.uid()));
            if run.is_dir() {
                return run.join("mirada-aware.sock");
            }
        }
    }
    std::env::temp_dir().join("mirada-aware.sock")
}

/// Extremo servidor — lo abre el compositor. Acepta una conexión por vuelta del
/// bucle (sin bloquear) y la atiende: una petición, una respuesta, cierra.
pub struct AwareServer {
    listener: UnixListener,
    path: PathBuf,
}

impl AwareServer {
    /// Abre el socket en `path`. Si hay otro servidor vivo ahí, falla; si el
    /// socket está muerto (de un compositor anterior), lo retira y se queda.
    pub fn bind(path: &Path) -> io::Result<Self> {
        if path.exists() {
            if UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(ErrorKind::AddrInUse, "ya hay un servidor mirada-aware"));
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

    /// Acepta una conexión pendiente sin bloquear. `None` si no hay ninguna —
    /// para llamarse cada vuelta del bucle.
    pub fn poll(&self) -> Option<AwareConn> {
        match self.listener.accept() {
            Ok((stream, _)) => Some(AwareConn { stream }),
            Err(_) => None,
        }
    }
}

impl Drop for AwareServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Una conexión aceptada: una petición y una respuesta.
pub struct AwareConn {
    stream: UnixStream,
}

impl AwareConn {
    /// Lee la petición (bloquea hasta el marco completo; es uno solo).
    pub fn read_request(&mut self) -> io::Result<Option<AwareRequest>> {
        self.stream.set_nonblocking(false)?;
        read_frame(&mut self.stream)
    }

    /// Envía la respuesta. El cliente cierra al recibirla.
    pub fn reply(&mut self, reply: &AwareReply) -> io::Result<()> {
        write_frame(&mut self.stream, reply)
    }
}

/// Cliente mirada-aware: conecta-pregunta-cierra. Guardá el `app_id` (el mismo
/// que `xdg_toplevel.set_app_id`) y la ruta una vez; llamá `register` al abrir y
/// `poll_clicks` en tu lazo.
pub struct Aware {
    path: PathBuf,
    app_id: String,
}

impl Aware {
    /// Prepara un cliente para `app_id`, apuntando al socket por defecto.
    pub fn new(app_id: impl Into<String>) -> Self {
        Self { path: default_socket_path(), app_id: app_id.into() }
    }

    /// Cambia la ruta del socket (para tests o despliegues no estándar).
    pub fn with_socket(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = path.into();
        self
    }

    fn enviar(&self, req: &AwareRequest) -> io::Result<AwareReply> {
        let mut stream = UnixStream::connect(&self.path)?;
        write_frame(&mut stream, req)?;
        read_frame(&mut stream)?
            .ok_or_else(|| io::Error::new(ErrorKind::UnexpectedEof, "el compositor cerró sin responder"))
    }

    /// Registra (reemplaza) los botones de la app. Idempotente.
    pub fn register(&self, items: Vec<AwareItem>) -> io::Result<()> {
        self.enviar(&AwareRequest::Register { app_id: self.app_id.clone(), items })?;
        Ok(())
    }

    /// Retira las contribuciones de la app.
    pub fn unregister(&self) -> io::Result<()> {
        self.enviar(&AwareRequest::Unregister { app_id: self.app_id.clone() })?;
        Ok(())
    }

    /// Devuelve (y vacía) los clicks pendientes sobre los botones de la app.
    pub fn poll_clicks(&self) -> io::Result<Vec<AwareClick>> {
        match self.enviar(&AwareRequest::PollClicks { app_id: self.app_id.clone() })? {
            AwareReply::Clicks(c) => Ok(c),
            AwareReply::Ok => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_socket(tag: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("mirada-aware-test-{tag}-{nanos}.sock"))
    }

    #[test]
    fn register_y_poll_round_trip_por_el_socket() {
        let path = temp_socket("rt");
        let server = AwareServer::bind(&path).unwrap();
        let cliente = Aware::new("com.test.app").with_socket(&path);

        // El cliente registra desde otro hilo (conecta-pregunta-cierra).
        let p2 = path.clone();
        let h = std::thread::spawn(move || {
            let c = Aware::new("com.test.app").with_socket(&p2);
            c.register(vec![AwareItem::new("run", "▶", "Correr")]).unwrap();
            c.poll_clicks().unwrap()
        });

        // El servidor atiende dos peticiones (register + poll).
        let mut visto_register = false;
        let mut clicks_enviados = false;
        let inicio = std::time::Instant::now();
        while (!visto_register || !clicks_enviados) && inicio.elapsed().as_secs() < 5 {
            if let Some(mut conn) = server.poll() {
                match conn.read_request().unwrap() {
                    Some(AwareRequest::Register { app_id, items }) => {
                        assert_eq!(app_id, "com.test.app");
                        assert_eq!(items[0].id, "run");
                        visto_register = true;
                        conn.reply(&AwareReply::Ok).unwrap();
                    }
                    Some(AwareRequest::PollClicks { app_id }) => {
                        assert_eq!(app_id, "com.test.app");
                        conn.reply(&AwareReply::Clicks(vec![AwareClick {
                            item_id: "run".into(),
                            window: 7,
                            window_title: "doc.ipynb".into(),
                        }]))
                        .unwrap();
                        clicks_enviados = true;
                    }
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let clicks = h.join().unwrap();
        assert!(visto_register, "el servidor recibió el register");
        assert_eq!(clicks.len(), 1);
        assert_eq!(clicks[0].item_id, "run");
        assert_eq!(clicks[0].window_title, "doc.ipynb");
        let _ = cliente; // mantiene el socket path vivo hasta acá
    }

    #[test]
    fn item_side_default_es_derecha() {
        assert_eq!(AwareItem::new("x", "x", "x").side, AwareSide::Right);
    }
}
