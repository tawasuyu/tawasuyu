// =============================================================================
//  uya-app::enlace — transporte P2P soberano sobre card-net (libp2p).
// -----------------------------------------------------------------------------
//  Envuelve `card_net::BrahmanNet` (el nodo libp2p de gioser, con relay/dcutr/
//  autonat) — el mismo transporte que usan ayni/minga/agora. Reemplaza al TCP
//  crudo anterior sin tocar `uya-core` ni la UI: el `Enlace` sigue siendo
//  sincrónico hacia afuera (eventos por `std::mpsc`, comandos por canal).
//
//  Un hilo dedicado corre un runtime tokio que:
//    · acepta streams entrantes del protocolo `/uya/transporte/1.0.0`,
//    · abre streams salientes al `conectar` a la multiaddr de un par,
//    · en cada conexión nueva manda el handshake `Hola`+`Estado`,
//    · lee `Paquete`s enmarcados → `EventoUya` (y el audio a la `MezclaRemota`),
//    · difunde lo que la app emite (`emitir`) a todos los pares.
//
//  Framing idéntico al de ayni-minga: `[u32 LE len][postcard(Paquete)]`.
//  Patrón calcado de `ayni-minga::EnlaceMinga`.
// =============================================================================

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel as std_channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use card_net::{
    BrahmanNet, Multiaddr, PeerId as LpPeerId, Protocol, Stream as LpStream, StreamProtocol,
};
use futures::StreamExt;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt, WriteHalf};
use tokio::sync::{mpsc as tmpsc, Mutex as TMutex};
use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt};

use uya_core::{id_desde_nombre, Paquete, ParticipanteId};

use crate::audio::MezclaRemota;
use crate::EventoUya;

/// El protocolo libp2p del transporte de uya. Coexiste multiplexado con los
/// demás (`/ayni/transporte/1.0.0`, `/minga/sync/1.0.0`...) sobre el nodo.
const PROTO: StreamProtocol = StreamProtocol::new("/uya/transporte/1.0.0");

/// Tope defensivo de un paquete serializado (8 MiB: cubre un cuadro RGBA).
const MAX_PAQUETE: usize = 8 * 1024 * 1024;

type CompatStream = Compat<LpStream>;
type Escritor = WriteHalf<CompatStream>;
type MapaEscritores = Arc<TMutex<HashMap<LpPeerId, Escritor>>>;

/// Comandos del API sync hacia el runtime tokio interno.
enum Cmd {
    Conectar(String),
    Difundir(Vec<u8>),
}

/// Identidad y estado de medios locales, compartidos con cada conexión para el
/// handshake `Hola`+`Estado`.
struct Yo {
    id: ParticipanteId,
    nombre: String,
    camara: Arc<AtomicBool>,
    microfono: Arc<AtomicBool>,
}

/// El handle de transporte de una sesión de uya. Sincrónico hacia afuera.
pub struct Enlace {
    yo: ParticipanteId,
    nombre: String,
    cmd_tx: tmpsc::UnboundedSender<Cmd>,
    eventos: Sender<EventoUya>,
    camara: Arc<AtomicBool>,
    microfono: Arc<AtomicBool>,
    direccion_local: String,
    /// Mezcla del audio entrante de todos los pares; la alimenta el lector y la
    /// drena el `AudioSink` de reproducción (ver `audio`).
    mezcla: Arc<Mutex<MezclaRemota>>,
}

impl Enlace {
    /// Levanta el nodo P2P y escucha en `bind` (una multiaddr, p. ej.
    /// `"/ip4/0.0.0.0/tcp/0"`). Devuelve el `Enlace` y el `Receiver` de eventos
    /// para la UI. Bloquea hasta que el nodo resolvió su dirección dialable.
    pub fn abrir(
        nombre: impl Into<String>,
        bind: &str,
    ) -> Result<(Self, Receiver<EventoUya>), String> {
        let nombre = nombre.into();
        let yo = id_desde_nombre(&nombre);

        let (cmd_tx, cmd_rx) = tmpsc::unbounded_channel::<Cmd>();
        let (ev_tx, ev_rx) = std_channel::<EventoUya>();
        let (listo_tx, listo_rx) = std_channel::<Result<String, String>>();
        let camara = Arc::new(AtomicBool::new(true));
        let microfono = Arc::new(AtomicBool::new(true));
        let mezcla = Arc::new(Mutex::new(MezclaRemota::default()));

        let yo_compartido = Arc::new(Yo {
            id: yo,
            nombre: nombre.clone(),
            camara: camara.clone(),
            microfono: microfono.clone(),
        });

        let bind = bind.to_string();
        {
            let ev_tx = ev_tx.clone();
            let mezcla = mezcla.clone();
            std::thread::Builder::new()
                .name("uya-net".into())
                .spawn(move || {
                    let rt = match tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(e) => {
                            let _ = listo_tx.send(Err(e.to_string()));
                            return;
                        }
                    };
                    rt.block_on(async move {
                        match arrancar(&bind).await {
                            Ok((node, dial_addr)) => {
                                let _ = listo_tx.send(Ok(dial_addr));
                                conducir(node, yo_compartido, cmd_rx, ev_tx, mezcla).await;
                            }
                            Err(e) => {
                                let _ = listo_tx.send(Err(e));
                            }
                        }
                    });
                })
                .map_err(|e| format!("uya: no pude lanzar el hilo de red: {e}"))?;
        }

        let direccion_local = match listo_rx.recv() {
            Ok(Ok(addr)) => addr,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err("uya: el hilo de red murió al arrancar".into()),
        };

        let enlace = Enlace {
            yo,
            nombre,
            cmd_tx,
            eventos: ev_tx,
            camara,
            microfono,
            direccion_local,
            mezcla,
        };
        Ok((enlace, ev_rx))
    }

    /// Mi identidad determinista (BLAKE3 del nombre).
    pub fn yo(&self) -> ParticipanteId {
        self.yo
    }

    /// Mi nombre.
    pub fn nombre(&self) -> &str {
        &self.nombre
    }

    /// La multiaddr dialable de este nodo (incluye `/p2p/<peerid>`). Es lo que
    /// el otro lado pasa a `conectar`.
    pub fn direccion_local(&self) -> &str {
        &self.direccion_local
    }

    /// Un emisor de eventos clonable, para que la captura empuje el
    /// auto-preview por el mismo canal que la red.
    pub fn eventos(&self) -> Sender<EventoUya> {
        self.eventos.clone()
    }

    /// La mezcla del audio entrante, para abrir la reproducción sobre ella
    /// (ver `audio::iniciar_reproduccion`).
    pub fn mezcla(&self) -> Arc<Mutex<MezclaRemota>> {
        self.mezcla.clone()
    }

    /// Conecta a un par dada su multiaddr COMPLETA (con `/p2p/<peerid>`), tal
    /// como la imprime `direccion_local` del otro lado.
    pub fn conectar(&self, addr: &str) {
        let _ = self.cmd_tx.send(Cmd::Conectar(addr.to_string()));
    }

    /// Difunde un paquete a todos los pares (serializa una vez).
    pub fn emitir(&self, paquete: &Paquete) {
        let _ = self.cmd_tx.send(Cmd::Difundir(paquete.codificar()));
    }

    /// ¿Está la cámara encendida? (lo lee el hilo de captura).
    pub fn camara_encendida(&self) -> bool {
        self.camara.load(Ordering::Relaxed)
    }

    /// ¿Está el micrófono encendido?
    pub fn microfono_encendido(&self) -> bool {
        self.microfono.load(Ordering::Relaxed)
    }

    /// Enciende/apaga la cámara y avisa a los pares.
    pub fn set_camara(&self, on: bool) {
        self.camara.store(on, Ordering::Relaxed);
        self.anunciar_estado();
    }

    /// Enciende/apaga el micrófono y avisa a los pares.
    pub fn set_microfono(&self, on: bool) {
        self.microfono.store(on, Ordering::Relaxed);
        self.anunciar_estado();
    }

    /// Cuelga: avisa a los pares que me voy.
    pub fn colgar(&self) {
        self.emitir(&Paquete::Adios);
    }

    fn anunciar_estado(&self) {
        self.emitir(&Paquete::Estado {
            camara: self.camara_encendida(),
            microfono: self.microfono_encendido(),
        });
    }
}

/// Crea el nodo, escucha, y compone la multiaddr dialable (con `/p2p/`).
async fn arrancar(bind: &str) -> Result<(BrahmanNet, String), String> {
    let node = BrahmanNet::new().map_err(|e| format!("uya: nodo libp2p: {e:?}"))?;
    let addr: Multiaddr = bind
        .parse()
        .map_err(|e| format!("uya: multiaddr inválida '{bind}': {e}"))?;
    let listen_addr = node.listen(addr).await;
    let dial = format!("{}/p2p/{}", listen_addr, node.peer_id);
    Ok((node, dial))
}

/// El bucle del runtime: acepta entrantes y atiende comandos de la app.
async fn conducir(
    node: BrahmanNet,
    yo: Arc<Yo>,
    mut cmd_rx: tmpsc::UnboundedReceiver<Cmd>,
    ev_tx: Sender<EventoUya>,
    mezcla: Arc<Mutex<MezclaRemota>>,
) {
    let escritores: MapaEscritores = Arc::new(TMutex::new(HashMap::new()));

    // Tarea aceptadora: streams entrantes del protocolo de uya.
    {
        let mut control = node.control.clone();
        let escritores = escritores.clone();
        let ev_tx = ev_tx.clone();
        let mezcla = mezcla.clone();
        let yo = yo.clone();
        tokio::spawn(async move {
            let entrantes = match control.accept(PROTO) {
                Ok(i) => i,
                Err(_) => return,
            };
            let mut entrantes = Box::pin(entrantes);
            while let Some((peer, stream)) = entrantes.next().await {
                registrar(
                    peer,
                    stream,
                    escritores.clone(),
                    ev_tx.clone(),
                    mezcla.clone(),
                    yo.clone(),
                )
                .await;
            }
        });
    }

    // Bucle de comandos de la app.
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            Cmd::Conectar(addr_str) => {
                let Ok(addr) = addr_str.parse::<Multiaddr>() else {
                    eprintln!("uya: multiaddr inválida '{addr_str}'");
                    continue;
                };
                let Some(peer) = peer_de(&addr) else {
                    eprintln!("uya: la multiaddr '{addr_str}' no lleva /p2p/<peerid>");
                    continue;
                };
                node.dial(addr);
                let mut control = node.control.clone();
                let escritores = escritores.clone();
                let ev_tx = ev_tx.clone();
                let mezcla = mezcla.clone();
                let yo = yo.clone();
                tokio::spawn(async move {
                    // Reintenta abrir el stream hasta que la conexión se establezca.
                    let limite = Instant::now() + Duration::from_secs(8);
                    loop {
                        match control.open_stream(peer, PROTO).await {
                            Ok(stream) => {
                                registrar(peer, stream, escritores, ev_tx, mezcla, yo).await;
                                break;
                            }
                            Err(_) if Instant::now() < limite => {
                                tokio::time::sleep(Duration::from_millis(150)).await;
                            }
                            Err(e) => {
                                eprintln!("uya: no pude abrir stream a {peer}: {e}");
                                break;
                            }
                        }
                    }
                });
            }
            Cmd::Difundir(bytes) => {
                let mut g = escritores.lock().await;
                let peers: Vec<LpPeerId> = g.keys().cloned().collect();
                let mut muertos = Vec::new();
                for p in peers {
                    if let Some(wr) = g.get_mut(&p) {
                        if escribir_frame(wr, &bytes).await.is_err() {
                            muertos.push(p);
                        }
                    }
                }
                for p in muertos {
                    g.remove(&p);
                }
            }
        }
    }
}

/// Registra un stream nuevo: manda el handshake `Hola`+`Estado`, guarda su mitad
/// de escritura y lanza la tarea lectora que traduce `Paquete`s a eventos.
async fn registrar(
    peer: LpPeerId,
    stream: LpStream,
    escritores: MapaEscritores,
    ev_tx: Sender<EventoUya>,
    mezcla: Arc<Mutex<MezclaRemota>>,
    yo: Arc<Yo>,
) {
    let compat = stream.compat();
    let (mut rd, mut wr) = tokio::io::split(compat);

    // Handshake: presentarse y declarar el estado de medios actual.
    let hola = Paquete::Hola {
        id: yo.id,
        nombre: yo.nombre.clone(),
    }
    .codificar();
    if escribir_frame(&mut wr, &hola).await.is_err() {
        return;
    }
    let estado = Paquete::Estado {
        camara: yo.camara.load(Ordering::Relaxed),
        microfono: yo.microfono.load(Ordering::Relaxed),
    }
    .codificar();
    if escribir_frame(&mut wr, &estado).await.is_err() {
        return;
    }

    escritores.lock().await.insert(peer, wr);

    let escritores_lector = escritores.clone();
    tokio::spawn(async move {
        let mut remoto: Option<ParticipanteId> = None;
        loop {
            let bytes = match leer_frame(&mut rd).await {
                Ok(b) => b,
                Err(_) => break,
            };
            let paquete = match Paquete::decodificar(&bytes) {
                Ok(p) => p,
                Err(_) => continue,
            };
            match paquete {
                Paquete::Hola { id, nombre } => {
                    remoto = Some(id);
                    let _ = ev_tx.send(EventoUya::Entra { id, nombre });
                }
                Paquete::Estado { camara, microfono } => {
                    if let Some(id) = remoto {
                        let _ = ev_tx.send(EventoUya::Estado {
                            id,
                            camara,
                            microfono,
                        });
                    }
                }
                Paquete::Cuadro {
                    ancho,
                    alto,
                    seq: _,
                    rgba,
                } => {
                    if let Some(id) = remoto {
                        let _ = ev_tx.send(EventoUya::Cuadro {
                            id,
                            ancho,
                            alto,
                            rgba: Arc::new(rgba),
                        });
                    }
                }
                Paquete::Audio {
                    sample_rate,
                    canales,
                    muestras,
                } => {
                    if let Some(id) = remoto {
                        mezcla.lock().empujar(id, sample_rate, canales as u16, &muestras);
                    }
                }
                Paquete::Adios => break,
            }
        }
        escritores_lector.lock().await.remove(&peer);
        if let Some(id) = remoto {
            mezcla.lock().quitar(&id);
            let _ = ev_tx.send(EventoUya::Sale { id });
        }
    });
}

/// Extrae el `PeerId` del componente `/p2p/...` de una multiaddr.
fn peer_de(addr: &Multiaddr) -> Option<LpPeerId> {
    addr.iter().find_map(|p| match p {
        Protocol::P2p(pid) => Some(pid),
        _ => None,
    })
}

/// Escribe un marco: largo u32 LE + payload.
async fn escribir_frame<W: AsyncWriteExt + Unpin>(wr: &mut W, payload: &[u8]) -> std::io::Result<()> {
    wr.write_all(&(payload.len() as u32).to_le_bytes()).await?;
    wr.write_all(payload).await?;
    wr.flush().await
}

/// Lee un marco completo.
async fn leer_frame<R: AsyncReadExt + Unpin>(rd: &mut R) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    rd.read_exact(&mut len).await?;
    let n = u32::from_le_bytes(len) as usize;
    if n == 0 || n > MAX_PAQUETE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "uya: marco fuera de rango",
        ));
    }
    let mut buf = vec![0u8; n];
    rd.read_exact(&mut buf).await?;
    Ok(buf)
}
