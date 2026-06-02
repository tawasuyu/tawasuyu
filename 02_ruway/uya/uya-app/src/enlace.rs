// =============================================================================
//  uya-app::enlace — transporte TCP punto-a-punto de la videollamada.
// -----------------------------------------------------------------------------
//  MVP feo a propósito (como el `EnlaceTcp` de ayni): un nodo escucha, los
//  demás se conectan; cada conexión es full-duplex y transporta `Paquete`s
//  enmarcados (largo u32 BE + postcard). Para una llamada de 2, uno escucha y
//  otro conecta — listo. Para N, cada par se conecta (malla manual): pasá
//  varias direcciones a `conectar`.
//
//  Toda la asincronía vive en un runtime tokio en un hilo aparte; hacia afuera
//  el `Enlace` es sincrónico y los eventos salen por un `std::mpsc::Receiver`,
//  igual que `ayni-app::Enlace`. La salida de cuadros se difunde a todas las
//  conexiones vivas con un `broadcast` (los cuadros viejos se descartan si una
//  conexión se atrasa — lo correcto para video).
//
//  Transporte destino: card-net (P2P soberano, relay/dcutr/autonat ya hechos).
//  Esta capa es el escalón intermedio para tener la llamada andando hoy.
// =============================================================================

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle as RtHandle;
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedSender;

use uya_core::{id_desde_nombre, Paquete, ParticipanteId};

use crate::EventoUya;

/// Tope defensivo del tamaño de un cuadro/paquete entrante (8 MiB). Evita que
/// un par malicioso o corrupto nos haga reservar memoria sin límite.
const MAX_PAQUETE: u32 = 8 * 1024 * 1024;

/// El handle de transporte de una sesión de uya. Sincrónico hacia afuera;
/// guarda dentro el `broadcast` de salida, el canal para marcar nuevas
/// conexiones, el estado de medios y el handle del runtime tokio.
pub struct Enlace {
    yo: ParticipanteId,
    nombre: String,
    /// Bytes ya serializados (postcard, sin enmarcar) de cada paquete a
    /// difundir. Cada conexión los enmarca y escribe.
    salida: broadcast::Sender<Arc<Vec<u8>>>,
    /// Eventos hacia la UI (clon del extremo `tx` que alimenta el `Receiver`).
    eventos: Sender<EventoUya>,
    /// Direcciones a las que conectarse (las consume el loop del runtime).
    dial: UnboundedSender<SocketAddr>,
    camara: Arc<AtomicBool>,
    microfono: Arc<AtomicBool>,
    direccion_local: Option<SocketAddr>,
    /// Se conserva para que el runtime no muera mientras el `Enlace` viva.
    _rt: RtHandle,
}

impl Enlace {
    /// Levanta el transporte. `bind` = dirección donde escuchar (None = sólo
    /// salientes). Devuelve el `Enlace` y el `Receiver` de eventos para la UI.
    pub fn abrir(
        nombre: impl Into<String>,
        bind: Option<SocketAddr>,
    ) -> io::Result<(Self, Receiver<EventoUya>)> {
        let nombre = nombre.into();
        let yo = id_desde_nombre(&nombre);

        let (ev_tx, ev_rx) = std::sync::mpsc::channel::<EventoUya>();
        let (salida_tx, _) = broadcast::channel::<Arc<Vec<u8>>>(64);
        let (dial_tx, mut dial_rx) = tokio::sync::mpsc::unbounded_channel::<SocketAddr>();
        let camara = Arc::new(AtomicBool::new(true));
        let microfono = Arc::new(AtomicBool::new(true));

        // Bind sincrónico para conocer la dirección real (útil con puerto 0).
        let std_listener = match bind {
            Some(a) => Some(std::net::TcpListener::bind(a)?),
            None => None,
        };
        let direccion_local = std_listener.as_ref().and_then(|l| l.local_addr().ok());
        if let Some(l) = &std_listener {
            l.set_nonblocking(true)?;
        }

        // Hilo dedicado con su propio runtime tokio.
        let (rt_tx, rt_rx) = std::sync::mpsc::channel::<RtHandle>();
        {
            let ev_tx = ev_tx.clone();
            let salida_tx = salida_tx.clone();
            let camara = camara.clone();
            let microfono = microfono.clone();
            let nombre_rt = nombre.clone();
            std::thread::Builder::new()
                .name("uya-net".into())
                .spawn(move || {
                    let rt = tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .build()
                        .expect("uya: runtime tokio");
                    let _ = rt_tx.send(rt.handle().clone());
                    rt.block_on(async move {
                        // Loop de aceptación de conexiones entrantes.
                        if let Some(l) = std_listener {
                            let listener =
                                TcpListener::from_std(l).expect("uya: TcpListener::from_std");
                            let ev = ev_tx.clone();
                            let sal = salida_tx.clone();
                            let cam = camara.clone();
                            let mic = microfono.clone();
                            let nom = nombre_rt.clone();
                            tokio::spawn(async move {
                                while let Ok((stream, _)) = listener.accept().await {
                                    conectar_par(
                                        stream,
                                        yo,
                                        nom.clone(),
                                        sal.clone(),
                                        ev.clone(),
                                        cam.clone(),
                                        mic.clone(),
                                    );
                                }
                            });
                        }
                        // Loop de conexiones salientes pedidas por `conectar`.
                        while let Some(addr) = dial_rx.recv().await {
                            let ev = ev_tx.clone();
                            let sal = salida_tx.clone();
                            let cam = camara.clone();
                            let mic = microfono.clone();
                            let nom = nombre_rt.clone();
                            tokio::spawn(async move {
                                match TcpStream::connect(addr).await {
                                    Ok(stream) => {
                                        conectar_par(stream, yo, nom, sal, ev, cam, mic)
                                    }
                                    Err(e) => eprintln!("uya: no pude conectar a {addr}: {e}"),
                                }
                            });
                        }
                    });
                })
                .expect("uya: spawn hilo de red");
        }

        let rt = rt_rx.recv().expect("uya: handle del runtime");
        let enlace = Enlace {
            yo,
            nombre,
            salida: salida_tx,
            eventos: ev_tx,
            dial: dial_tx,
            camara,
            microfono,
            direccion_local,
            _rt: rt,
        };
        Ok((enlace, ev_rx))
    }

    /// Mi identidad determinista.
    pub fn yo(&self) -> ParticipanteId {
        self.yo
    }

    /// Mi nombre.
    pub fn nombre(&self) -> &str {
        &self.nombre
    }

    /// Dirección local donde escucho (si fue con `bind`).
    pub fn direccion_local(&self) -> Option<SocketAddr> {
        self.direccion_local
    }

    /// Un emisor de eventos clonable, para que la captura empuje el
    /// auto-preview por el mismo canal que la red.
    pub fn eventos(&self) -> Sender<EventoUya> {
        self.eventos.clone()
    }

    /// Pide conectarse a un par. La conexión ocurre en el runtime; los errores
    /// se reportan por stderr (MVP).
    pub fn conectar(&self, addr: SocketAddr) {
        let _ = self.dial.send(addr);
    }

    /// Difunde un paquete a todas las conexiones vivas (serializa una vez).
    pub fn emitir(&self, paquete: &Paquete) {
        let _ = self.salida.send(Arc::new(paquete.codificar()));
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

/// Arma una conexión ya establecida: un task escritor (difunde la salida) y el
/// loop lector (traduce paquetes a `EventoUya`). Comparte el código entre
/// entrantes y salientes — el protocolo es simétrico.
fn conectar_par(
    stream: TcpStream,
    yo: ParticipanteId,
    nombre: String,
    salida: broadcast::Sender<Arc<Vec<u8>>>,
    eventos: Sender<EventoUya>,
    camara: Arc<AtomicBool>,
    microfono: Arc<AtomicBool>,
) {
    let _ = stream.set_nodelay(true);
    let (lectura, escritura) = stream.into_split();

    // Escritor: primero Hola + Estado (handshake por conexión), luego difunde.
    let mut rx = salida.subscribe();
    let escritor = tokio::spawn(async move {
        let mut wr = escritura;
        let hola = Paquete::Hola { id: yo, nombre }.codificar();
        if escribir_marco(&mut wr, &hola).await.is_err() {
            return;
        }
        let estado = Paquete::Estado {
            camara: camara.load(Ordering::Relaxed),
            microfono: microfono.load(Ordering::Relaxed),
        }
        .codificar();
        if escribir_marco(&mut wr, &estado).await.is_err() {
            return;
        }
        loop {
            match rx.recv().await {
                Ok(bytes) => {
                    if escribir_marco(&mut wr, &bytes).await.is_err() {
                        break;
                    }
                }
                // Nos atrasamos: saltamos los cuadros perdidos (video al día).
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Lector: traduce el cable a eventos para la UI.
    tokio::spawn(async move {
        let mut rd = lectura;
        let mut remoto: Option<ParticipanteId> = None;
        loop {
            let bytes = match leer_marco(&mut rd).await {
                Ok(Some(b)) => b,
                _ => break,
            };
            let paquete = match Paquete::decodificar(&bytes) {
                Ok(p) => p,
                Err(_) => continue,
            };
            match paquete {
                Paquete::Hola { id, nombre } => {
                    remoto = Some(id);
                    let _ = eventos.send(EventoUya::Entra { id, nombre });
                }
                Paquete::Estado { camara, microfono } => {
                    if let Some(id) = remoto {
                        let _ = eventos.send(EventoUya::Estado {
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
                        let _ = eventos.send(EventoUya::Cuadro {
                            id,
                            ancho,
                            alto,
                            rgba: Arc::new(rgba),
                        });
                    }
                }
                Paquete::Adios => break,
            }
        }
        escritor.abort();
        if let Some(id) = remoto {
            let _ = eventos.send(EventoUya::Sale { id });
        }
    });
}

/// Escribe un marco: largo u32 big-endian + payload.
async fn escribir_marco(wr: &mut OwnedWriteHalf, payload: &[u8]) -> io::Result<()> {
    wr.write_all(&(payload.len() as u32).to_be_bytes()).await?;
    wr.write_all(payload).await?;
    wr.flush().await
}

/// Lee un marco. `Ok(None)` = fin de stream limpio.
async fn leer_marco(rd: &mut OwnedReadHalf) -> io::Result<Option<Vec<u8>>> {
    let mut largo = [0u8; 4];
    match rd.read_exact(&mut largo).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_be_bytes(largo);
    if n == 0 || n > MAX_PAQUETE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "uya: marco fuera de rango",
        ));
    }
    let mut buf = vec![0u8; n as usize];
    rd.read_exact(&mut buf).await?;
    Ok(Some(buf))
}
