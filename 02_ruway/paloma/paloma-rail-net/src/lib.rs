//! paloma-rail-net — el **transporte TCP** del rail soberano (Eje 3.B, vivo).
//!
//! El salto de red real del correo P2P: entrega [`RailEnvelope`]s entre nodos
//! sobre TCP directo, **ruteado por identidad** (la clave pública del peer), no
//! por IP. Mismo trazado que el resto de la suite (`ayni-sync::EnlaceTcp`):
//! `[longitud u32 LE][postcard(Frame)]`, un hilo aceptador + un hilo lector por
//! conexión, mapa de peers bajo `Mutex`. Deliberadamente simple: un correo
//! soberano tiene un puñado de contactos, no un millón.
//!
//! ## Handshake
//!
//! Al conectar (entrante o saliente), cada lado manda primero un [`Frame::Hello`]
//! con su identidad. Recién entonces el peer queda registrado bajo su pubkey y
//! `send(to, …)` puede rutear. Los [`Frame::Envelope`] que llegan se empujan por
//! el `Receiver<RailEnvelope>` que el anfitrión drena (abre/verifica con
//! `paloma-rail::open` y despacha a la UI).
//!
//! No hace discovery ni NAT traversal (eso vendría de `card-net`/DHT); asume que
//! conocés la dirección del peer (LAN, o un relay). La identidad sí es soberana:
//! aunque el transporte sea por IP, el sobre va firmado y se abre por pubkey.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use paloma_rail::{RailEnvelope, RailError, RailId, RailTransport};
use serde::{Deserialize, Serialize};

/// Techo de un frame serializado: 16 MiB (igual que el resto de la suite).
const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Lo que viaja por el cable: el saludo de identidad o un sobre del rail.
#[derive(Debug, Serialize, Deserialize)]
enum Frame {
    /// Primer frame de cada conexión: la identidad del que lo manda.
    Hello(RailId),
    /// Un sobre del rail.
    Envelope(RailEnvelope),
}

type PeerMap = Arc<Mutex<HashMap<RailId, TcpStream>>>;

/// Transporte TCP del rail. Escucha conexiones, mantiene un stream por identidad
/// y rutea los sobres salientes por pubkey.
pub struct TcpRail {
    me: RailId,
    local: SocketAddr,
    peers: PeerMap,
    /// Canal por el que los hilos lectores empujan los sobres entrantes (también
    /// los de conexiones salientes). El receptor lo drena el anfitrión.
    eventos: Sender<RailEnvelope>,
}

impl TcpRail {
    /// Empieza a escuchar en `bind` (p. ej. `"0.0.0.0:7710"` o `"127.0.0.1:0"`
    /// para puerto efímero). `me` es la identidad propia (se anuncia a cada
    /// peer). Devuelve el transporte y el receptor de sobres entrantes —que el
    /// anfitrión drena (abre/verifica/despacha)—.
    pub fn escuchar(bind: &str, me: RailId) -> Result<(Self, Receiver<RailEnvelope>), RailError> {
        let listener = TcpListener::bind(bind).map_err(io)?;
        let local = listener.local_addr().map_err(io)?;
        let (tx, rx) = channel();
        let peers: PeerMap = Arc::new(Mutex::new(HashMap::new()));

        let peers_acc = peers.clone();
        let tx_acc = tx.clone();
        thread::spawn(move || {
            for conexion in listener.incoming() {
                match conexion {
                    Ok(stream) => registrar(stream, me, &peers_acc, &tx_acc),
                    Err(_) => break,
                }
            }
        });

        Ok((TcpRail { me, local, peers, eventos: tx }, rx))
    }

    /// Abre una conexión saliente a `addr` (`"192.168.1.5:7710"`). Tras el
    /// handshake el peer queda ruteable por su identidad.
    pub fn conectar(&self, addr: &str) -> Result<(), RailError> {
        let stream = TcpStream::connect(addr).map_err(io)?;
        registrar(stream, self.me, &self.peers, &self.eventos);
        Ok(())
    }

    /// La dirección local efectiva (con el puerto resuelto si se pidió `:0`).
    pub fn direccion_local(&self) -> SocketAddr {
        self.local
    }

    /// ¿Hay un peer registrado con esa identidad? (tras el handshake).
    pub fn tiene_peer(&self, id: &RailId) -> bool {
        self.peers.lock().map(|p| p.contains_key(id)).unwrap_or(false)
    }

    /// Cuántos peers conectados.
    pub fn num_peers(&self) -> usize {
        self.peers.lock().map(|p| p.len()).unwrap_or(0)
    }
}

impl RailTransport for TcpRail {
    fn send(&self, to: RailId, envelope: &RailEnvelope) -> Result<(), RailError> {
        let mut peers = self.peers.lock().expect("mutex de peers envenenado");
        match peers.get_mut(&to) {
            Some(stream) => escribir(stream, &Frame::Envelope(envelope.clone())),
            None => Err(RailError::Transport("peer desconocido o desconectado".into())),
        }
    }
}

/// Registra un stream: manda el `Hello` propio, y lanza el hilo lector que, al
/// recibir el `Hello` del peer, lo inserta en el mapa bajo su identidad; los
/// `Envelope` siguientes se empujan por `tx`.
fn registrar(mut stream: TcpStream, me: RailId, peers: &PeerMap, tx: &Sender<RailEnvelope>) {
    // Anunciar nuestra identidad primero.
    if escribir(&mut stream, &Frame::Hello(me)).is_err() {
        return;
    }
    let escritor = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let peers_lector = peers.clone();
    let tx_lector = tx.clone();
    thread::spawn(move || {
        let mut lector = stream;
        let mut peer_id: Option<RailId> = None;
        loop {
            match leer(&mut lector) {
                Ok(Frame::Hello(id)) => {
                    peer_id = Some(id);
                    if let Ok(w) = escritor.try_clone() {
                        peers_lector.lock().expect("mutex envenenado").insert(id, w);
                    }
                }
                Ok(Frame::Envelope(env)) => {
                    if tx_lector.send(env).is_err() {
                        break; // receptor cerrado: app terminando.
                    }
                }
                Err(_) => break, // EOF o frame corrupto.
            }
        }
        if let Some(id) = peer_id {
            peers_lector.lock().expect("mutex envenenado").remove(&id);
        }
    });
}

/// Lee un frame: longitud `u32` LE + payload postcard.
fn leer(lector: &mut impl Read) -> Result<Frame, RailError> {
    let mut cab = [0u8; 4];
    lector.read_exact(&mut cab).map_err(io)?;
    let n = u32::from_le_bytes(cab) as usize;
    if n == 0 || n > MAX_FRAME {
        return Err(RailError::Transport("frame inválido".into()));
    }
    let mut buf = vec![0u8; n];
    lector.read_exact(&mut buf).map_err(io)?;
    postcard::from_bytes(&buf).map_err(|e| RailError::Codec(e.to_string()))
}

/// Escribe un frame: longitud `u32` LE + payload postcard, con flush.
fn escribir(escritor: &mut impl Write, frame: &Frame) -> Result<(), RailError> {
    let bytes = postcard::to_allocvec(frame).map_err(|e| RailError::Codec(e.to_string()))?;
    if bytes.len() > MAX_FRAME {
        return Err(RailError::Transport("frame sobredimensionado".into()));
    }
    escritor.write_all(&(bytes.len() as u32).to_le_bytes()).map_err(io)?;
    escritor.write_all(&bytes).map_err(io)?;
    escritor.flush().map_err(io)?;
    Ok(())
}

fn io(e: std::io::Error) -> RailError {
    RailError::Transport(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::Keypair;
    use paloma_core::{Address, Flags, Message, MessageId, SignatureStatus};
    use std::time::{Duration, Instant};

    fn mensaje(subject: &str, body: &str) -> Message {
        Message {
            id: MessageId("<x@suyu>".into()),
            from: Address::named("Ana", "ana@suyu.net"),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: subject.into(),
            date: 0,
            in_reply_to: None,
            references: vec![],
            body_text: body.into(),
            body_html: None,
            flags: Flags::default(),
            signature: SignatureStatus::Unsigned,
            mailbox: "Borradores".into(),
            cuerpos: vec![],
        }
    }

    /// Espera hasta `cond` o se agota el tiempo (handshake asíncrono).
    fn esperar(mut cond: impl FnMut() -> bool) -> bool {
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(3) {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        cond()
    }

    #[test]
    fn rail_tcp_entrega_entre_dos_nodos() {
        let ana = Keypair::from_seed([1; 32]);
        let bob = Keypair::from_seed([2; 32]);

        let (rail_a, _rx_a) = TcpRail::escuchar("127.0.0.1:0", ana.public_key()).unwrap();
        let (rail_b, rx_b) = TcpRail::escuchar("127.0.0.1:0", bob.public_key()).unwrap();

        // Ana se conecta a Bob; ambos intercambian Hello.
        rail_a.conectar(&rail_b.direccion_local().to_string()).unwrap();

        // Esperar a que Ana conozca la identidad de Bob (handshake completo).
        assert!(esperar(|| rail_a.tiene_peer(&bob.public_key())), "handshake no completó");

        // Ana sella un mensaje para Bob y lo manda por el rail TCP.
        let env = paloma_rail::seal(&ana, bob.public_key(), &mensaje("minga", "vení el sábado")).unwrap();
        rail_a.send(bob.public_key(), &env).unwrap();

        // Bob recibe el sobre, lo abre y verifica.
        let recibido = rx_b.recv_timeout(Duration::from_secs(3)).expect("Bob no recibió");
        let msg = paloma_rail::open(&recibido, bob.public_key()).unwrap();
        assert_eq!(msg.subject, "minga");
        assert_eq!(msg.body_text, "vení el sábado");
        assert_eq!(msg.signature, SignatureStatus::Verified);
    }

    #[test]
    fn enviar_a_peer_desconocido_falla() {
        let ana = Keypair::from_seed([1; 32]);
        let (rail_a, _rx) = TcpRail::escuchar("127.0.0.1:0", ana.public_key()).unwrap();
        let env = paloma_rail::seal(&ana, [9; 32], &mensaje("x", "y")).unwrap();
        assert!(rail_a.send([9; 32], &env).is_err());
    }
}
