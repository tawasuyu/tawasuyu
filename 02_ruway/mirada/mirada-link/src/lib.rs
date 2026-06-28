//! `mirada-link` — el transporte Cerebro↔Cuerpo del compositor.
//!
//! [`mirada_protocol`] define *qué* se dice (los enums y el marco de
//! cable); este crate define *cómo viaja*: un socket Unix con un hilo
//! lector de fondo que entrega los mensajes recibidos por un canal, para
//! que el dueño del [`Link`] sólo tenga que sondear sin bloquearse.
//!
//! Los dos procesos usan el mismo tipo, parametrizado al revés:
//!
//! - El Cerebro tiene un [`BrainLink`]: envía [`BrainCommand`], recibe
//!   [`BodyEvent`].
//! - El Cuerpo tiene un [`BodyLink`]: envía [`BodyEvent`], recibe
//!   [`BrainCommand`].
//!
//! Para arrancar el par hay tres caminos: [`connected_pair`] (un
//! `socketpair`, ideal para heredar un fd al lanzar al hijo o para
//! tests), [`Link::connect`] (conectar a una ruta) y [`Link::listen`]
//! (escuchar en una ruta y aceptar una conexión).

#![forbid(unsafe_code)]

use std::io::{self, BufReader};
use std::marker::PhantomData;
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

use mirada_protocol::{read_frame, write_frame, BodyEvent, BrainCommand};

/// El extremo del Cerebro: envía [`BrainCommand`], recibe [`BodyEvent`].
pub type BrainLink = Link<BrainCommand, BodyEvent>;

/// El extremo del Cuerpo: envía [`BodyEvent`], recibe [`BrainCommand`].
pub type BodyLink = Link<BodyEvent, BrainCommand>;

/// Un extremo del canal: envía mensajes de tipo `Out` y recibe `In`.
///
/// La escritura es síncrona sobre el socket; la lectura la hace un hilo
/// de fondo que deposita lo recibido en un canal interno. Al soltar el
/// `Link` se cierra el socket, lo que termina el hilo lector propio y le
/// señala EOF al otro extremo.
pub struct Link<Out, In> {
    writer: UnixStream,
    incoming: Receiver<In>,
    /// `true` mientras el hilo lector siga vivo; pasa a `false` cuando el otro
    /// extremo cierra (EOF) o el socket falla. Permite al dueño detectar la
    /// desconexión sin consumir el canal — clave para reconectar.
    alive: Arc<AtomicBool>,
    _out: PhantomData<fn(Out)>,
}

impl<Out, In> Link<Out, In>
where
    Out: Serialize,
    In: DeserializeOwned + Send + 'static,
{
    /// Construye un `Link` sobre un socket ya conectado.
    pub fn from_stream(stream: UnixStream) -> io::Result<Self> {
        let reader = stream.try_clone()?;
        let (tx, rx) = mpsc::channel();
        let alive = Arc::new(AtomicBool::new(true));
        let alive_reader = alive.clone();
        thread::spawn(move || {
            let mut r = BufReader::new(reader);
            // Lee marcos hasta EOF limpio o error de socket.
            while let Ok(Some(msg)) = read_frame::<_, In>(&mut r) {
                if tx.send(msg).is_err() {
                    break; // el dueño soltó el Link
                }
            }
            // EOF o error: el otro extremo se fue.
            alive_reader.store(false, Ordering::Relaxed);
        });
        Ok(Self { writer: stream, incoming: rx, alive, _out: PhantomData })
    }

    /// Conecta a un socket Unix en `path` (lado cliente).
    pub fn connect<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Self::from_stream(UnixStream::connect(path)?)
    }

    /// Escucha en `path` y bloquea hasta aceptar una conexión (lado
    /// servidor). El socket de escucha se cierra tras el primer cliente.
    pub fn listen<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let listener = UnixListener::bind(path)?;
        let (stream, _) = listener.accept()?;
        Self::from_stream(stream)
    }

    /// Envía un mensaje. Falla si el otro extremo cerró el canal.
    pub fn send(&mut self, msg: &Out) -> io::Result<()> {
        write_frame(&mut self.writer, msg)
    }

    /// `true` mientras el otro extremo siga conectado. Pasa a `false` cuando
    /// cierra (EOF) o el socket falla — sin consumir mensajes pendientes, así el
    /// dueño puede drenar lo que quedó y recién entonces reconectar.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Recoge un mensaje si hay alguno pendiente, sin bloquear.
    pub fn try_recv(&self) -> Option<In> {
        self.incoming.try_recv().ok()
    }

    /// Vacía todos los mensajes pendientes — un tick del bucle de eventos.
    pub fn drain(&self) -> Vec<In> {
        self.incoming.try_iter().collect()
    }

    /// Bloquea hasta recibir un mensaje. Devuelve `None` si el otro
    /// extremo cerró el canal.
    pub fn recv(&self) -> Option<In> {
        self.incoming.recv().ok()
    }

    /// Bloquea hasta recibir un mensaje o agotar `timeout`. Distingue el
    /// agotamiento del cierre del canal vía [`RecvTimeoutError`], para un bucle
    /// que intercale sondeo de eventos con otras tareas (p. ej. vigilar archivos
    /// de config) y aun así detecte la desconexión del otro extremo.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<In, RecvTimeoutError> {
        self.incoming.recv_timeout(timeout)
    }
}

impl<Out, In> Drop for Link<Out, In> {
    fn drop(&mut self) {
        // Cierra la conexión: el hilo lector propio recibe EOF y termina,
        // y el otro extremo ve EOF en su próxima lectura.
        let _ = self.writer.shutdown(Shutdown::Both);
    }
}

/// El extremo servidor del Cuerpo que **acepta Cerebros sucesivos**: un listener
/// Unix persistente que sigue vivo aunque el Cerebro conectado muera. Es la
/// pieza que habilita reiniciar el Cerebro (a propósito o por crash) sin tirar
/// el Cuerpo ni las conexiones Wayland de los clientes: el Cuerpo conserva este
/// servidor, detecta la muerte del Cerebro ([`Link::is_alive`]) y re-acepta uno
/// nuevo con [`LinkServer::try_accept`], re-sincronizando el estado.
pub type BodyLinkServer = LinkServer<BodyEvent, BrainCommand>;

/// Servidor que escucha en una ruta y produce un [`Link`] nuevo por cada
/// conexión aceptada. A diferencia de [`Link::listen`] (de un solo tiro), el
/// socket de escucha **persiste** entre conexiones.
pub struct LinkServer<Out, In> {
    listener: UnixListener,
    _m: PhantomData<fn(Out, In)>,
}

impl<Out, In> LinkServer<Out, In>
where
    Out: Serialize,
    In: DeserializeOwned + Send + 'static,
{
    /// Vincula el listener a `path` (borrando un socket viejo si quedó) y lo pone
    /// en modo no bloqueante, para que [`try_accept`](Self::try_accept) integre
    /// con un bucle de eventos sin colgarse.
    pub fn bind<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref();
        // Un socket huérfano de una corrida anterior impediría el bind.
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, _m: PhantomData })
    }

    /// Acepta una conexión pendiente sin bloquear. `Ok(None)` = nadie esperando.
    /// El [`Link`] devuelto usa lectura bloqueante en su hilo propio (el socket
    /// aceptado se pone en modo bloqueante).
    pub fn try_accept(&self) -> io::Result<Option<Link<Out, In>>> {
        match self.listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                Ok(Some(Link::from_stream(stream)?))
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Crea un par Cerebro↔Cuerpo conectado en memoria, con un `socketpair`.
///
/// Es el camino de los tests y también el del despliegue real cuando el
/// Cerebro lanza al Cuerpo como proceso hijo y le hereda un extremo.
pub fn connected_pair() -> io::Result<(BrainLink, BodyLink)> {
    let (a, b) = UnixStream::pair()?;
    Ok((Link::from_stream(a)?, Link::from_stream(b)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirada_protocol::{Rect, WindowPlacement};
    use std::time::Duration;

    fn place(id: u64) -> BrainCommand {
        BrainCommand::Place(vec![WindowPlacement {
            id,
            rect: Rect::new(0, 0, 800, 600),
            visible: true,
            focused: true,
            floating: false,
            fullscreen: false,
            suspended: false,
            frame_divisor: 1,
        }])
    }

    #[test]
    fn brain_command_reaches_the_body() {
        let (mut brain, body) = connected_pair().unwrap();
        brain.send(&place(1)).unwrap();
        // Da un instante al hilo lector.
        for _ in 0..100 {
            if let Some(cmd) = body.try_recv() {
                assert_eq!(cmd, place(1));
                return;
            }
            thread::sleep(Duration::from_millis(2));
        }
        panic!("el comando no llegó al Cuerpo");
    }

    #[test]
    fn body_event_reaches_the_brain() {
        let (brain, mut body) = connected_pair().unwrap();
        let ev = BodyEvent::Keybind("Super+Return".into());
        body.send(&ev).unwrap();
        assert_eq!(brain.recv(), Some(ev));
    }

    #[test]
    fn many_messages_keep_their_order() {
        let (brain, mut body) = connected_pair().unwrap();
        for id in 0..20 {
            body.send(&BodyEvent::WindowClosed { id }).unwrap();
        }
        for id in 0..20 {
            assert_eq!(brain.recv(), Some(BodyEvent::WindowClosed { id }));
        }
    }

    #[test]
    fn drain_collects_everything_pending() {
        let (mut brain, body) = connected_pair().unwrap();
        for id in 1..=5 {
            brain.send(&place(id)).unwrap();
        }
        // Espera a que el hilo lector encole los cinco.
        let mut got = Vec::new();
        for _ in 0..100 {
            got.extend(body.drain());
            if got.len() == 5 {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(got.len(), 5);
    }

    #[test]
    fn dropping_one_end_closes_the_other() {
        let (brain, body) = connected_pair().unwrap();
        drop(body);
        // Sin nadie al otro lado, recv termina con None en vez de colgarse.
        assert_eq!(brain.recv(), None);
    }

    #[test]
    fn sending_into_a_closed_link_errors() {
        let (mut brain, body) = connected_pair().unwrap();
        drop(body);
        // La primera escritura puede pasar al búfer del socket; alguna
        // de ellas acaba fallando con tubería rota.
        let mut errored = false;
        for id in 0..1000 {
            if brain.send(&place(id)).is_err() {
                errored = true;
                break;
            }
        }
        assert!(errored, "se esperaba un error de tubería rota");
    }

    #[test]
    fn is_alive_se_cae_cuando_el_otro_extremo_muere() {
        let (brain, body) = connected_pair().unwrap();
        assert!(brain.is_alive());
        drop(body);
        // El hilo lector del Cerebro ve EOF y marca el Link como muerto.
        let mut cayo = false;
        for _ in 0..200 {
            if !brain.is_alive() {
                cayo = true;
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        assert!(cayo, "is_alive debía caer a false al morir el otro extremo");
    }

    #[test]
    fn el_servidor_acepta_cerebros_sucesivos() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("mirada-link-reconnect-{}.sock", std::process::id()));
        let server: BodyLinkServer = LinkServer::bind(&path).unwrap();

        // Primer Cerebro: conecta, habla, se va.
        {
            let mut brain: BrainLink = Link::connect(&path).unwrap();
            let mut body = aceptar(&server);
            brain.send(&BrainCommand::Shutdown).unwrap();
            assert_eq!(esperar(&mut body), Some(BrainCommand::Shutdown));
            drop(brain);
            // El Cuerpo nota que el Cerebro murió.
            let mut murio = false;
            for _ in 0..200 {
                if !body.is_alive() {
                    murio = true;
                    break;
                }
                thread::sleep(Duration::from_millis(2));
            }
            assert!(murio);
        }

        // Segundo Cerebro: el MISMO servidor lo acepta — el listener sobrevivió.
        {
            let mut brain: BrainLink = Link::connect(&path).unwrap();
            let mut body = aceptar(&server);
            brain.send(&BrainCommand::Lock).unwrap();
            assert_eq!(esperar(&mut body), Some(BrainCommand::Lock));
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn soak_muchos_ciclos_de_reconexion() {
        // Espeja el modo `watch` del dev-loop: el Cerebro muere y vuelve muchas
        // veces. El listener debe sobrevivir todos los ciclos sin fugarse ni
        // agotarse, y cada Cerebro nuevo debe poder hablar.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("mirada-link-soak-{}.sock", std::process::id()));
        let server: BodyLinkServer = LinkServer::bind(&path).unwrap();

        for ciclo in 0..12u64 {
            let mut brain: BrainLink = Link::connect(&path).unwrap();
            let mut body = aceptar(&server);
            // El Cerebro habla; el Cuerpo lo recibe en orden.
            brain.send(&BrainCommand::Close(ciclo)).unwrap();
            assert_eq!(esperar(&mut body), Some(BrainCommand::Close(ciclo)));
            // El Cuerpo responde; el Cerebro lo recibe.
            body.send(&BodyEvent::WindowClosed { id: ciclo }).unwrap();
            assert_eq!(brain.recv(), Some(BodyEvent::WindowClosed { id: ciclo }));
            // El Cerebro muere; el Cuerpo lo nota.
            drop(brain);
            let mut murio = false;
            for _ in 0..200 {
                if !body.is_alive() {
                    murio = true;
                    break;
                }
                thread::sleep(Duration::from_millis(2));
            }
            assert!(murio, "ciclo {ciclo}: el Cuerpo no detectó la muerte del Cerebro");
        }

        let _ = std::fs::remove_file(&path);
    }

    fn aceptar(server: &BodyLinkServer) -> BodyLink {
        for _ in 0..200 {
            if let Some(link) = server.try_accept().unwrap() {
                return link;
            }
            thread::sleep(Duration::from_millis(2));
        }
        panic!("el servidor no aceptó la conexión");
    }

    fn esperar(body: &mut BodyLink) -> Option<BrainCommand> {
        for _ in 0..200 {
            if let Some(cmd) = body.try_recv() {
                return Some(cmd);
            }
            thread::sleep(Duration::from_millis(2));
        }
        None
    }

    #[test]
    fn connect_and_listen_round_trip_over_a_path() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("mirada-link-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let server_path = path.clone();
        let server = thread::spawn(move || {
            let mut link: BodyLink = Link::listen(&server_path).unwrap();
            link.send(&BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 })
                .unwrap();
            // Mantén vivo el extremo hasta que el cliente lea.
            link.recv()
        });

        // Espera a que el servidor publique el socket.
        let mut brain: Option<BrainLink> = None;
        for _ in 0..200 {
            if let Ok(l) = Link::connect(&path) {
                brain = Some(l);
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        let mut brain = brain.expect("no se pudo conectar al servidor");
        assert_eq!(
            brain.recv(),
            Some(BodyEvent::OutputAdded { id: 0, width: 1920, height: 1080 })
        );
        brain.send(&BrainCommand::Shutdown).unwrap();
        assert_eq!(server.join().unwrap(), Some(BrainCommand::Shutdown));
        let _ = std::fs::remove_file(&path);
    }
}
