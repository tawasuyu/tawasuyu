//! `EnlaceTcp` — el transporte LAN directo de P1.
//!
//! Un hilo aceptador escucha conexiones entrantes; `conectar` abre salientes.
//! Cada conexión engendra un hilo lector que decodifica [`Sobre`]s y los empuja
//! por el canal de eventos. La escritura (difundir/enviar) recorre el mapa de
//! peers vivos. Framing: `[longitud u32 LE][postcard(Sobre)]` —el mismo trazado
//! length-prefixed que usa el resto de la suite—.
//!
//! Es deliberadamente simple (un hilo por conexión, `Mutex` sobre el mapa de
//! peers): un chat LAN tiene un puñado de pares, no un millón. La escalabilidad
//! seria llega con `EnlaceMinga` (P3), no aquí.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::{ErrorSync, EventoRed, PeerId, Sobre, Transporte};

/// Techo de un sobre serializado: 16 MiB. Acota los búferes y descarta un frame
/// disparatado sin intentar reservar memoria absurda.
const MAX_SOBRE: usize = 16 * 1024 * 1024;

type MapaPeers = Arc<Mutex<HashMap<String, TcpStream>>>;

/// Transporte TCP directo entre pares de la LAN.
pub struct EnlaceTcp {
    local: SocketAddr,
    peers: MapaPeers,
    eventos: Sender<EventoRed>,
}

impl EnlaceTcp {
    /// Empieza a escuchar en `bind` (p. ej. `"0.0.0.0:7700"` o `"127.0.0.1:0"`
    /// para puerto efímero). Devuelve el enlace y el extremo receptor del canal
    /// de eventos —que el llamador drena (un hilo que reenvía a la UI vía
    /// `Handle::dispatch`, o un bucle en el CLI)—.
    pub fn escuchar(bind: &str) -> Result<(Self, Receiver<EventoRed>), ErrorSync> {
        let listener = TcpListener::bind(bind)?;
        let local = listener.local_addr()?;
        let (tx, rx) = channel();
        let peers: MapaPeers = Arc::new(Mutex::new(HashMap::new()));

        let peers_acc = peers.clone();
        let tx_acc = tx.clone();
        thread::spawn(move || {
            for conexion in listener.incoming() {
                match conexion {
                    Ok(stream) => registrar(stream, &peers_acc, &tx_acc),
                    Err(_) => break,
                }
            }
        });

        Ok((
            EnlaceTcp {
                local,
                peers,
                eventos: tx,
            },
            rx,
        ))
    }

    /// Abre una conexión saliente a un peer (`"192.168.1.5:7700"`). Tras volver,
    /// el peer ya está registrado y un `EventoRed::Conectado` se emitió.
    pub fn conectar(&self, addr: &str) -> Result<(), ErrorSync> {
        let stream = TcpStream::connect(addr)?;
        registrar(stream, &self.peers, &self.eventos);
        Ok(())
    }

    /// La dirección local efectiva (con el puerto ya resuelto si se pidió `:0`).
    pub fn direccion_local(&self) -> SocketAddr {
        self.local
    }

    /// Cuántos peers conectados hay ahora.
    pub fn num_peers(&self) -> usize {
        self.peers.lock().map(|p| p.len()).unwrap_or(0)
    }
}

impl Transporte for EnlaceTcp {
    fn difundir(&self, sobre: &Sobre) -> Result<(), ErrorSync> {
        let mut peers = self.peers.lock().expect("mutex de peers envenenado");
        // escribe a cada peer; los que fallan (conexión muerta) se purgan.
        let mut muertos = Vec::new();
        for (addr, stream) in peers.iter_mut() {
            if escribir_sobre(stream, sobre).is_err() {
                muertos.push(addr.clone());
            }
        }
        for addr in muertos {
            peers.remove(&addr);
            let _ = self.eventos.send(EventoRed::Desconectado(PeerId(addr)));
        }
        Ok(())
    }

    fn enviar(&self, peer: &PeerId, sobre: &Sobre) -> Result<(), ErrorSync> {
        let mut peers = self.peers.lock().expect("mutex de peers envenenado");
        match peers.get_mut(&peer.0) {
            Some(stream) => escribir_sobre(stream, sobre),
            None => Err(ErrorSync::PeerDesconocido),
        }
    }
}

/// Registra un stream como peer (guarda un clon para escritura) y lanza su hilo
/// lector. Emite `Conectado` al entrar y `Desconectado` al cerrarse.
fn registrar(stream: TcpStream, peers: &MapaPeers, tx: &Sender<EventoRed>) {
    let addr = match stream.peer_addr() {
        Ok(a) => a.to_string(),
        Err(_) => return,
    };
    let escritor = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    peers
        .lock()
        .expect("mutex de peers envenenado")
        .insert(addr.clone(), escritor);
    let _ = tx.send(EventoRed::Conectado(PeerId(addr.clone())));

    let peers_lector = peers.clone();
    let tx_lector = tx.clone();
    thread::spawn(move || {
        let mut lector = stream;
        loop {
            match leer_sobre(&mut lector) {
                Ok(sobre) => {
                    if tx_lector
                        .send(EventoRed::Sobre(PeerId(addr.clone()), sobre))
                        .is_err()
                    {
                        break; // el receptor de eventos se cerró: app terminando.
                    }
                }
                Err(_) => break, // EOF o frame corrupto: cierra el peer.
            }
        }
        peers_lector
            .lock()
            .expect("mutex de peers envenenado")
            .remove(&addr);
        let _ = tx_lector.send(EventoRed::Desconectado(PeerId(addr)));
    });
}

/// Lee un sobre del stream: longitud `u32` LE + payload postcard.
fn leer_sobre(lector: &mut impl Read) -> Result<Sobre, ErrorSync> {
    let mut cab = [0u8; 4];
    lector.read_exact(&mut cab)?;
    let n = u32::from_le_bytes(cab) as usize;
    if n == 0 || n > MAX_SOBRE {
        return Err(ErrorSync::FrameInvalido);
    }
    let mut buf = vec![0u8; n];
    lector.read_exact(&mut buf)?;
    postcard::from_bytes(&buf).map_err(|_| ErrorSync::FrameInvalido)
}

/// Escribe un sobre: longitud `u32` LE + payload postcard, y hace flush.
fn escribir_sobre(escritor: &mut impl Write, sobre: &Sobre) -> Result<(), ErrorSync> {
    let bytes = postcard::to_allocvec(sobre).map_err(|_| ErrorSync::FrameInvalido)?;
    if bytes.len() > MAX_SOBRE {
        return Err(ErrorSync::FrameInvalido);
    }
    escritor.write_all(&(bytes.len() as u32).to_le_bytes())?;
    escritor.write_all(&bytes)?;
    escritor.flush()?;
    Ok(())
}
