// =============================================================================
//  ayni :: ayni-minga — EnlaceMinga, transporte P2P sobre libp2p
// -----------------------------------------------------------------------------
//  Envuelve `card_net::BrahmanNet` (el nodo libp2p de tawasuyu) tras el trait
//  `Transporte` de `ayni-sync`. Un hilo dedicado corre un runtime tokio que:
//    * escucha streams entrantes del protocolo `/ayni/transporte/1.0.0`,
//    * abre streams salientes al `conectar` a un peer,
//    * lee frames postcard de cada stream → `EventoRed::Sobre`,
//    * escribe frames cuando la app llama `difundir`/`enviar`.
//  La app habla SYNC (manda comandos por canal, drena `EventoRed` por mpsc),
//  igual que con `EnlaceTcp`: el mismo bucle de red de la app sirve para ambos.
//
//  Framing idéntico al de `EnlaceTcp`: `[u32 LE len][postcard(Sobre)]`.
// =============================================================================

use std::collections::HashMap;
use std::sync::mpsc::{channel as std_channel, Receiver, Sender as StdSender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use card_net::{BrahmanNet, Multiaddr, PeerId as LpPeerId, Protocol, Stream as LpStream, StreamProtocol};
use futures::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt, WriteHalf};
use tokio::sync::{mpsc as tmpsc, Mutex as TMutex};
use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt};

use ayni_sync::{ErrorSync, EventoRed, PeerId, Sobre, Transporte};

/// El protocolo libp2p del transporte de Ayni. Vecino conceptual del
/// `/minga/sync/1.0.0`; coexisten multiplexados sobre el mismo nodo.
const PROTO: StreamProtocol = StreamProtocol::new("/ayni/transporte/1.0.0");

/// Techo de un sobre serializado: 16 MiB (igual que `EnlaceTcp`).
const MAX_SOBRE: usize = 16 * 1024 * 1024;

type CompatStream = Compat<LpStream>;
type Escritor = WriteHalf<CompatStream>;
type MapaEscritores = Arc<TMutex<HashMap<LpPeerId, Escritor>>>;

/// Comandos del API sync hacia el runtime tokio interno.
enum Cmd {
    Conectar(String),
    Difundir(Vec<u8>),
    Enviar(String, Vec<u8>),
}

/// Falla de construcción/uso de `EnlaceMinga`.
#[derive(Debug, thiserror::Error)]
pub enum ErrorMinga {
    #[error("ayni-minga :: fallo al arrancar el nodo libp2p: {0}")]
    Arranque(String),
    #[error("ayni-minga :: el runtime de red está cerrado")]
    Cerrado,
}

/// Transporte P2P sobre libp2p. Construir con [`EnlaceMinga::escuchar`]; usar
/// vía el trait [`Transporte`] (igual que `EnlaceTcp`).
pub struct EnlaceMinga {
    cmd_tx: tmpsc::UnboundedSender<Cmd>,
    local: String,
}

impl EnlaceMinga {
    /// Arranca el nodo, escucha en `bind` (una multiaddr, p. ej.
    /// `"/ip4/0.0.0.0/tcp/0"`), y devuelve el enlace + el receptor de eventos.
    /// Bloquea hasta que el nodo resolvió su dirección de escucha.
    pub fn escuchar(bind: &str) -> Result<(EnlaceMinga, Receiver<EventoRed>), ErrorMinga> {
        let (cmd_tx, cmd_rx) = tmpsc::unbounded_channel();
        let (ev_tx, ev_rx) = std_channel::<EventoRed>();
        let (listo_tx, listo_rx) = std_channel::<Result<String, String>>();
        let bind = bind.to_string();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
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
                        conducir(node, cmd_rx, ev_tx).await;
                    }
                    Err(e) => {
                        let _ = listo_tx.send(Err(e));
                    }
                }
            });
        });

        match listo_rx.recv() {
            Ok(Ok(local)) => Ok((EnlaceMinga { cmd_tx, local }, ev_rx)),
            Ok(Err(e)) => Err(ErrorMinga::Arranque(e)),
            Err(_) => Err(ErrorMinga::Arranque("el hilo de runtime murió".into())),
        }
    }

    /// Conecta a un peer dado su multiaddr COMPLETA (con `/p2p/<peerid>`), tal
    /// como la devuelve [`direccion_local`](Self::direccion_local) del otro lado.
    pub fn conectar(&self, addr: &str) -> Result<(), ErrorMinga> {
        self.cmd_tx
            .send(Cmd::Conectar(addr.to_string()))
            .map_err(|_| ErrorMinga::Cerrado)
    }

    /// La multiaddr dialable de este nodo (incluye `/p2p/<peerid>`).
    pub fn direccion_local(&self) -> &str {
        &self.local
    }
}

impl Transporte for EnlaceMinga {
    fn difundir(&self, sobre: &Sobre) -> Result<(), ErrorSync> {
        let bytes = postcard::to_allocvec(sobre).map_err(|_| ErrorSync::FrameInvalido)?;
        self.cmd_tx
            .send(Cmd::Difundir(bytes))
            .map_err(|_| ErrorSync::PeerDesconocido)?;
        Ok(())
    }

    fn enviar(&self, peer: &PeerId, sobre: &Sobre) -> Result<(), ErrorSync> {
        let bytes = postcard::to_allocvec(sobre).map_err(|_| ErrorSync::FrameInvalido)?;
        self.cmd_tx
            .send(Cmd::Enviar(peer.0.clone(), bytes))
            .map_err(|_| ErrorSync::PeerDesconocido)?;
        Ok(())
    }
}

/// Crea el nodo, escucha, y compone la multiaddr dialable.
async fn arrancar(bind: &str) -> Result<(BrahmanNet, String), String> {
    let node = BrahmanNet::new().map_err(|e| format!("{e:?}"))?;
    let addr: Multiaddr = bind.parse().map_err(|e| format!("multiaddr inválida: {e}"))?;
    let listen_addr = node.listen(addr).await;
    let dial = format!("{}/p2p/{}", listen_addr, node.peer_id);
    Ok((node, dial))
}

/// El bucle del runtime: acepta entrantes y atiende comandos de la app.
async fn conducir(
    node: BrahmanNet,
    mut cmd_rx: tmpsc::UnboundedReceiver<Cmd>,
    ev_tx: StdSender<EventoRed>,
) {
    let escritores: MapaEscritores = Arc::new(TMutex::new(HashMap::new()));

    // tarea aceptadora: streams entrantes del protocolo Ayni.
    {
        let mut control = node.control.clone();
        let escritores = escritores.clone();
        let ev_tx = ev_tx.clone();
        tokio::spawn(async move {
            let entrantes = match control.accept(PROTO) {
                Ok(i) => i,
                Err(_) => return,
            };
            let mut entrantes = Box::pin(entrantes);
            while let Some((peer, stream)) = entrantes.next().await {
                registrar(peer, stream, escritores.clone(), ev_tx.clone()).await;
            }
        });
    }

    // bucle de comandos.
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            Cmd::Conectar(addr_str) => {
                let Ok(addr) = addr_str.parse::<Multiaddr>() else { continue };
                let Some(peer) = peer_de(&addr) else { continue };
                node.dial(addr);
                let mut control = node.control.clone();
                let escritores = escritores.clone();
                let ev_tx = ev_tx.clone();
                tokio::spawn(async move {
                    // reintenta abrir el stream hasta que la conexión se establezca.
                    let limite = Instant::now() + Duration::from_secs(8);
                    loop {
                        match control.open_stream(peer, PROTO).await {
                            Ok(stream) => {
                                registrar(peer, stream, escritores, ev_tx).await;
                                break;
                            }
                            Err(_) if Instant::now() < limite => {
                                tokio::time::sleep(Duration::from_millis(150)).await;
                            }
                            Err(_) => break,
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
                    let _ = ev_tx.send(EventoRed::Desconectado(PeerId(p.to_string())));
                }
            }
            Cmd::Enviar(peer_str, bytes) => {
                let Ok(peer) = peer_str.parse::<LpPeerId>() else { continue };
                let mut g = escritores.lock().await;
                if let Some(wr) = g.get_mut(&peer) {
                    let _ = escribir_frame(wr, &bytes).await;
                }
            }
        }
    }
}

/// Registra un stream: guarda su mitad de escritura, emite `Conectado`, y lanza
/// la tarea lectora que decodifica `Sobre`s y los empuja como `EventoRed::Sobre`.
async fn registrar(
    peer: LpPeerId,
    stream: LpStream,
    escritores: MapaEscritores,
    ev_tx: StdSender<EventoRed>,
) {
    let compat = stream.compat();
    let (mut rd, wr) = tokio::io::split(compat);
    escritores.lock().await.insert(peer, wr);
    let _ = ev_tx.send(EventoRed::Conectado(PeerId(peer.to_string())));

    let escritores_lector = escritores.clone();
    tokio::spawn(async move {
        loop {
            match leer_frame(&mut rd).await {
                Ok(bytes) => match postcard::from_bytes::<Sobre>(&bytes) {
                    Ok(sobre) => {
                        if ev_tx
                            .send(EventoRed::Sobre(PeerId(peer.to_string()), sobre))
                            .is_err()
                        {
                            break; // app cerró el receptor.
                        }
                    }
                    Err(_) => break, // frame corrupto.
                },
                Err(_) => break, // EOF o error.
            }
        }
        escritores_lector.lock().await.remove(&peer);
        let _ = ev_tx.send(EventoRed::Desconectado(PeerId(peer.to_string())));
    });
}

/// Extrae el `PeerId` del componente `/p2p/...` de una multiaddr.
fn peer_de(addr: &Multiaddr) -> Option<LpPeerId> {
    addr.iter().find_map(|p| match p {
        Protocol::P2p(pid) => Some(pid),
        _ => None,
    })
}

async fn leer_frame<R: AsyncReadExt + Unpin>(rd: &mut R) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    rd.read_exact(&mut len).await?;
    let n = u32::from_le_bytes(len) as usize;
    if n == 0 || n > MAX_SOBRE {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame inválido"));
    }
    let mut buf = vec![0u8; n];
    rd.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn escribir_frame<W: AsyncWriteExt + Unpin>(wr: &mut W, bytes: &[u8]) -> std::io::Result<()> {
    wr.write_all(&(bytes.len() as u32).to_le_bytes()).await?;
    wr.write_all(bytes).await?;
    wr.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{Carga, Conversacion};
    use ayni_crypto::{verificar_firma, Identidad};
    use ayni_sync::Fusionador;
    use std::sync::Mutex;

    type Estado = Arc<Mutex<(Conversacion, Fusionador)>>;

    /// El "pump" de red de un peer (igual que en la app y en los tests de TCP):
    /// al conectar anuncia cabezas; ante cada sobre, corre la anti-entropía.
    fn lanzar_pump(enlace: Arc<EnlaceMinga>, rx: Receiver<EventoRed>, estado: Estado) {
        std::thread::spawn(move || {
            for ev in rx {
                match ev {
                    EventoRed::Conectado(peer) => {
                        let cabezas = estado.lock().unwrap().0.cabezas();
                        let _ = enlace.enviar(&peer, &Sobre::Cabezas(cabezas));
                    }
                    EventoRed::Desconectado(_) => {}
                    EventoRed::Sobre(peer, sobre) => {
                        let respuestas = {
                            let mut g = estado.lock().unwrap();
                            let (conv, fus) = &mut *g;
                            fus.procesar(conv, sobre, verificar_firma).1
                        };
                        for r in respuestas {
                            let _ = enlace.enviar(&peer, &r);
                        }
                    }
                }
            }
        });
    }

    fn esperar_len(estado: &Estado, objetivo: usize, segundos: u64) -> bool {
        let limite = Instant::now() + Duration::from_secs(segundos);
        loop {
            if estado.lock().unwrap().0.len() >= objetivo {
                return true;
            }
            if Instant::now() > limite {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn dos_nodos_minga_convergen_por_libp2p() {
        let alicia = Identidad::desde_semilla([1u8; 32], "Alicia");

        let est_a: Estado = Arc::new(Mutex::new((Conversacion::nueva(), Fusionador::nuevo())));
        let est_b: Estado = Arc::new(Mutex::new((Conversacion::nueva(), Fusionador::nuevo())));

        let (enlace_a, rx_a) = EnlaceMinga::escuchar("/ip4/127.0.0.1/tcp/0").unwrap();
        let (enlace_b, rx_b) = EnlaceMinga::escuchar("/ip4/127.0.0.1/tcp/0").unwrap();
        let enlace_a = Arc::new(enlace_a);
        let enlace_b = Arc::new(enlace_b);

        // Alicia escribe dos mensajes ANTES de conectar.
        {
            let mut g = est_a.lock().unwrap();
            for i in 0..2 {
                let n = g.0.redactar(alicia.agora_id(), Carga::Texto(format!("hola {i}")), i, |id| {
                    alicia.firmar(id)
                });
                g.0.agregar(n).unwrap();
            }
        }

        lanzar_pump(enlace_a.clone(), rx_a, est_a.clone());
        lanzar_pump(enlace_b.clone(), rx_b, est_b.clone());

        // Beto se conecta a la multiaddr libp2p de Alicia; la anti-entropía corre.
        enlace_b.conectar(enlace_a.direccion_local()).unwrap();

        assert!(
            esperar_len(&est_b, 2, 20),
            "Beto debe reconciliar los 2 mensajes de Alicia por libp2p"
        );
        assert!(est_b.lock().unwrap().0.verificar_firmas(verificar_firma).is_ok());
    }
}
