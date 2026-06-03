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

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel as std_channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use card_net::{
    BrahmanNet, Multiaddr, PeerId as LpPeerId, Protocol, Stream as LpStream, StreamProtocol,
};
use futures::StreamExt;
use opus_wave::types::{Channels, SampleRate};
use opus_wave::OpusDecoder;
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt, WriteHalf};
use tokio::sync::{mpsc as tmpsc, Mutex as TMutex};
use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt};

use uya_core::{id_desde_nombre, FormatoCuadro, Paquete, ParticipanteId};

use crate::audio::{DetectorVoz, MezclaRemota};
use crate::identidad::{verificar_presentacion, Identidad};
use crate::EventoUya;

/// El protocolo libp2p del transporte de uya. Coexiste multiplexado con los
/// demás (`/ayni/transporte/1.0.0`, `/minga/sync/1.0.0`...) sobre el nodo.
const PROTO: StreamProtocol = StreamProtocol::new("/uya/transporte/1.0.0");

/// Tope defensivo de un paquete serializado (8 MiB: cubre un cuadro RGBA).
const MAX_PAQUETE: usize = 8 * 1024 * 1024;

/// Tope de muestras que un frame Opus puede expandir (120 ms @ 48 kHz mono).
const MAX_OPUS_FRAME: usize = 5_760;

type CompatStream = Compat<LpStream>;
type Escritor = WriteHalf<CompatStream>;
type MapaEscritores = Arc<TMutex<HashMap<LpPeerId, Escritor>>>;

/// Estado compartido de la malla N-a-N, pasado a cada conexión. Permite que un
/// nodo, al enterarse por gossip de un par nuevo, lo disque por su cuenta —
/// hasta converger en malla completa con sólo unirse a un anfitrión.
#[derive(Clone)]
struct Malla {
    /// Mi propio `PeerId`, para el desempate de quién disca (el menor) y para no
    /// discarme a mí mismo.
    mi_peer: LpPeerId,
    /// Escritores por par conectado (también el blanco de las difusiones).
    escritores: MapaEscritores,
    /// Direcciones dialables conocidas (incluida la propia). Sólo crece.
    conocidas: Arc<TMutex<HashSet<String>>>,
    /// Pares con un dial en vuelo, para no abrir dos conexiones al mismo.
    pendientes: Arc<TMutex<HashSet<LpPeerId>>>,
    /// Para que el lector pida discar un par recién descubierto.
    cmd_tx: tmpsc::UnboundedSender<Cmd>,
}

/// Comandos del API sync hacia el runtime tokio interno.
enum Cmd {
    Conectar(String),
    Difundir(Vec<u8>),
    /// Unirse a una sala por nombre vía DHT: anunciarse como provider de la
    /// clave de la sala y descubrir periódicamente a los demás providers.
    UnirSala {
        sala: String,
        bootstrap: Vec<String>,
    },
}

/// Identidad y estado de medios locales, compartidos con cada conexión para el
/// handshake `Hola`+`Estado`. La clave pública y la firma de presentación se
/// precalculan una vez (son constantes de la sesión).
struct Yo {
    id: ParticipanteId,
    nombre: String,
    clave: [u8; 32],
    firma: Vec<u8>,
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
    /// Si está en alto, el hilo de captura sustituye la cámara por la pantalla.
    pantalla: Arc<AtomicBool>,
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
        // Identidad soberana persistida: la semilla secreta engendra el par
        // Ed25519 (firma del Hola) y, abajo, el keypair libp2p del transporte.
        let identidad = Identidad::cargar();
        let yo = identidad.id();
        let semilla = identidad.semilla();
        let firma = identidad.firmar_presentacion(&nombre);

        let (cmd_tx, cmd_rx) = tmpsc::unbounded_channel::<Cmd>();
        let (ev_tx, ev_rx) = std_channel::<EventoUya>();
        let (listo_tx, listo_rx) = std_channel::<Result<String, String>>();
        let camara = Arc::new(AtomicBool::new(true));
        let microfono = Arc::new(AtomicBool::new(true));
        let pantalla = Arc::new(AtomicBool::new(false));
        let mezcla = Arc::new(Mutex::new(MezclaRemota::default()));

        let yo_compartido = Arc::new(Yo {
            id: yo,
            nombre: nombre.clone(),
            clave: identidad.clave(),
            firma,
            camara: camara.clone(),
            microfono: microfono.clone(),
        });

        let bind = bind.to_string();
        {
            let ev_tx = ev_tx.clone();
            let mezcla = mezcla.clone();
            let cmd_tx = cmd_tx.clone();
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
                        match arrancar(&bind, semilla).await {
                            Ok((node, dial_addr)) => {
                                let _ = listo_tx.send(Ok(dial_addr.clone()));
                                conducir(node, dial_addr, cmd_tx, yo_compartido, cmd_rx, ev_tx, mezcla)
                                    .await;
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
            pantalla,
            direccion_local,
            mezcla,
        };
        Ok((enlace, ev_rx))
    }

    /// Mi identidad (huella BLAKE3 de mi clave pública, ver `identidad`).
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

    /// Silencia (o reactiva) a un par **localmente**: deja de oírse de este
    /// lado, sin avisar a nadie ni cortar la red.
    pub fn silenciar_par(&self, id: ParticipanteId, on: bool) {
        self.mezcla.lock().silenciar(id, on);
    }

    /// ¿Tengo silenciado a este par localmente?
    pub fn par_silenciado(&self, id: &ParticipanteId) -> bool {
        self.mezcla.lock().esta_silenciado(id)
    }

    /// Conecta a un par dada su multiaddr COMPLETA (con `/p2p/<peerid>`), tal
    /// como la imprime `direccion_local` del otro lado.
    pub fn conectar(&self, addr: &str) {
        let _ = self.cmd_tx.send(Cmd::Conectar(addr.to_string()));
    }

    /// Se une a una sala por nombre: se anuncia en el DHT como provider de la
    /// clave de la sala y descubre a los demás providers (que entran a la malla
    /// automáticamente). `bootstrap` son multiaddrs de nodos a los que dialear
    /// para sembrar el DHT (un rendezvous conocido). Aditivo: convive con
    /// `conectar` manual.
    pub fn unir_sala(&self, sala: impl Into<String>, bootstrap: Vec<String>) {
        let _ = self.cmd_tx.send(Cmd::UnirSala {
            sala: sala.into(),
            bootstrap,
        });
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

    /// ¿Estoy compartiendo pantalla? (lo lee el hilo de captura para elegir la
    /// fuente de video).
    pub fn compartiendo_pantalla(&self) -> bool {
        self.pantalla.load(Ordering::Relaxed)
    }

    /// Empieza/termina de compartir la pantalla. El hilo de captura sustituye
    /// la cámara por el display (o vuelve a ella). Requiere compilar con la
    /// feature `pantalla`; sin ella, no tiene efecto.
    pub fn set_compartir_pantalla(&self, on: bool) {
        self.pantalla.store(on, Ordering::Relaxed);
    }

    /// Cuelga: avisa a los pares que me voy.
    pub fn colgar(&self) {
        self.emitir(&Paquete::Adios);
    }

    /// Difunde un mensaje de texto a la sala (la charla). El eco local lo pinta
    /// la UI por su cuenta —no vuelve por la red—, igual que en `ayni`.
    pub fn enviar_mensaje(&self, texto: impl Into<String>) {
        self.emitir(&Paquete::Mensaje {
            texto: texto.into(),
        });
    }

    fn anunciar_estado(&self) {
        self.emitir(&Paquete::Estado {
            camara: self.camara_encendida(),
            microfono: self.microfono_encendido(),
        });
    }
}

/// Crea el nodo, escucha, y compone la multiaddr dialable (con `/p2p/`).
async fn arrancar(bind: &str, semilla: [u8; 32]) -> Result<(BrahmanNet, String), String> {
    // Identidad de transporte determinista: el keypair ed25519 deriva de la
    // misma semilla BLAKE3(nombre) que el `ParticipanteId` de app. Así el PeerId
    // —y por ende la multiaddr dialable— es estable entre arranques, y la
    // identidad libp2p comparte raíz con la de la app.
    let keypair = card_net::Keypair::ed25519_from_bytes(semilla)
        .map_err(|e| format!("uya: keypair ed25519: {e}"))?;
    let node =
        BrahmanNet::with_keypair(keypair).map_err(|e| format!("uya: nodo libp2p: {e:?}"))?;
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
    mi_dir: String,
    cmd_tx: tmpsc::UnboundedSender<Cmd>,
    yo: Arc<Yo>,
    mut cmd_rx: tmpsc::UnboundedReceiver<Cmd>,
    ev_tx: Sender<EventoUya>,
    mezcla: Arc<Mutex<MezclaRemota>>,
) {
    // El nodo se comparte con la tarea de descubrimiento DHT (Arc): start/find
    // toman `&self` y corren en el mismo runtime.
    let node = Arc::new(node);
    // Mi propia dirección dialable arranca el set de conocidas: es lo que
    // gossipeo para que los demás puedan discarme.
    let mut conocidas = HashSet::new();
    conocidas.insert(mi_dir);
    let malla = Malla {
        mi_peer: node.peer_id,
        escritores: Arc::new(TMutex::new(HashMap::new())),
        conocidas: Arc::new(TMutex::new(conocidas)),
        pendientes: Arc::new(TMutex::new(HashSet::new())),
        cmd_tx,
    };

    // Tarea aceptadora: streams entrantes del protocolo de uya.
    {
        let mut control = node.control.clone();
        let malla = malla.clone();
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
                registrar(peer, stream, malla.clone(), ev_tx.clone(), mezcla.clone(), yo.clone())
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
                // Nunca me disco a mí mismo ni a un par ya conectado / en vuelo.
                if peer == malla.mi_peer || malla.escritores.lock().await.contains_key(&peer) {
                    continue;
                }
                if !malla.pendientes.lock().await.insert(peer) {
                    continue;
                }
                node.dial(addr);
                let mut control = node.control.clone();
                let malla = malla.clone();
                let ev_tx = ev_tx.clone();
                let mezcla = mezcla.clone();
                let yo = yo.clone();
                tokio::spawn(async move {
                    // Reintenta abrir el stream hasta que la conexión se establezca.
                    let limite = Instant::now() + Duration::from_secs(8);
                    loop {
                        match control.open_stream(peer, PROTO).await {
                            Ok(stream) => {
                                registrar(peer, stream, malla.clone(), ev_tx, mezcla, yo).await;
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
                    malla.pendientes.lock().await.remove(&peer);
                });
            }
            Cmd::Difundir(bytes) => {
                difundir_a_todos(&malla.escritores, &bytes).await;
            }
            Cmd::UnirSala { sala, bootstrap } => {
                // Clave de la sala = BLAKE3("uya/sala/<nombre>"), reusando la
                // misma derivación determinista de los ParticipanteId.
                let clave = id_desde_nombre(&format!("uya/sala/{sala}"));
                // Sembrar el DHT dialando los bootstraps (rendezvous conocidos).
                for b in &bootstrap {
                    if let Ok(a) = b.parse::<Multiaddr>() {
                        node.dial(a);
                    }
                }
                node.start_providing(&clave);

                // Loop de descubrimiento: cada pocos segundos busca a los demás
                // providers de la sala y los mete en la malla (deduplicada).
                let node_d = node.clone();
                let cmd_d = malla.cmd_tx.clone();
                let mi = malla.mi_peer;
                let depurar = std::env::var("UYA_DEBUG").is_ok();
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        let provs = node_d.find_providers(&clave).await;
                        if depurar {
                            eprintln!("uya: find_providers(sala) → {} provider(s)", provs.len());
                        }
                        for p in provs {
                            if p == mi {
                                continue;
                            }
                            // Discar por PeerId desnudo: el swarm resuelve la
                            // dirección desde Kad (poblado por mDNS/identify). Más
                            // robusto que resolverla a mano con find_closest_peers.
                            let _ = cmd_d.send(Cmd::Conectar(format!("/p2p/{p}")));
                        }
                    }
                });
            }
        }
    }
}

/// Escribe `bytes` (un paquete ya enmarcable) a todas las conexiones vivas,
/// soltando las que fallan.
async fn difundir_a_todos(escritores: &MapaEscritores, bytes: &[u8]) {
    let mut g = escritores.lock().await;
    let peers: Vec<LpPeerId> = g.keys().cloned().collect();
    let mut muertos = Vec::new();
    for p in peers {
        if let Some(wr) = g.get_mut(&p) {
            if escribir_frame(wr, bytes).await.is_err() {
                muertos.push(p);
            }
        }
    }
    for p in muertos {
        g.remove(&p);
    }
}

/// Registra un stream nuevo: manda el handshake `Hola`+`Estado`, guarda su mitad
/// de escritura y lanza la tarea lectora que traduce `Paquete`s a eventos.
async fn registrar(
    peer: LpPeerId,
    stream: LpStream,
    malla: Malla,
    ev_tx: Sender<EventoUya>,
    mezcla: Arc<Mutex<MezclaRemota>>,
    yo: Arc<Yo>,
) {
    let compat = stream.compat();
    let (mut rd, mut wr) = tokio::io::split(compat);

    // Handshake: presentarse (con prueba de identidad firmada) y declarar el
    // estado de medios actual.
    let hola = Paquete::Hola {
        id: yo.id,
        nombre: yo.nombre.clone(),
        clave: yo.clave,
        firma: yo.firma.clone(),
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
    // Gossip de malla: comparto todas las direcciones dialables que conozco
    // (incluida la mía) para que este par alcance al resto.
    let snapshot: Vec<String> = malla.conocidas.lock().await.iter().cloned().collect();
    if escribir_frame(&mut wr, &Paquete::Pares { direcciones: snapshot }.codificar())
        .await
        .is_err()
    {
        return;
    }

    malla.escritores.lock().await.insert(peer, wr);

    let malla = malla.clone();
    tokio::spawn(async move {
        let mut remoto: Option<ParticipanteId> = None;
        // Nombre del par (aprendido del `Hola`), para etiquetar sus mensajes.
        let mut remoto_nombre: Option<String> = None;
        // Decoder Opus con estado, propio de esta conexión (lazy).
        let mut dec: Option<OpusDecoder> = None;
        // Detector de voz del par, para resaltar a quien habla en la UI.
        let mut vad = DetectorVoz::nuevo();
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
                Paquete::Hola {
                    id,
                    nombre,
                    clave,
                    firma,
                } => {
                    remoto = Some(id);
                    remoto_nombre = Some(nombre.clone());
                    let verificado = verificar_presentacion(&id, &nombre, &clave, &firma);
                    let _ = ev_tx.send(EventoUya::Entra {
                        id,
                        nombre,
                        verificado,
                    });
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
                    formato,
                    datos,
                } => {
                    if let Some(id) = remoto {
                        // Decodificar a RGBA según el formato del cable.
                        let cuadro = match formato {
                            FormatoCuadro::Rgba => Some((ancho, alto, datos)),
                            FormatoCuadro::Jpeg => crate::video::decodar_jpeg(&datos),
                        };
                        if let Some((w, h, rgba)) = cuadro {
                            let _ = ev_tx.send(EventoUya::Cuadro {
                                id,
                                ancho: w,
                                alto: h,
                                rgba: Arc::new(rgba),
                            });
                        }
                    }
                }
                Paquete::Audio { opus } => {
                    if let Some(id) = remoto {
                        if dec.is_none() {
                            dec = OpusDecoder::new(SampleRate::Hz48000, Channels::Mono).ok();
                        }
                        if let Some(d) = dec.as_mut() {
                            let mut pcm = vec![0f32; MAX_OPUS_FRAME];
                            if let Ok(frames) =
                                d.decode_float(Some(&opus), &mut pcm, MAX_OPUS_FRAME as i32, false)
                            {
                                // `decode_float` devuelve frames por canal (mono → muestras).
                                pcm.truncate(frames.max(0) as usize);
                                // Detección de voz del par: avisar a la UI en
                                // los flancos para resaltar a quien habla.
                                if let Some(hablando) = vad.procesar(&pcm) {
                                    let _ = ev_tx.send(EventoUya::Voz { id, hablando });
                                }
                                mezcla.lock().empujar(id, 48_000, 1, &pcm);
                            }
                        }
                    }
                }
                Paquete::Mensaje { texto } => {
                    if let Some(id) = remoto {
                        let nombre = remoto_nombre
                            .clone()
                            .unwrap_or_else(|| uya_core::hex_corto(&id));
                        let _ = ev_tx.send(EventoUya::Mensaje { id, nombre, texto });
                    }
                }
                Paquete::Pares { direcciones } => {
                    // Aprender las direcciones nuevas (conocidas sólo crece).
                    let mut nuevas = Vec::new();
                    {
                        let mut con = malla.conocidas.lock().await;
                        for d in direcciones {
                            if con.insert(d.clone()) {
                                nuevas.push(d);
                            }
                        }
                    }
                    if !nuevas.is_empty() {
                        // Discar cada par nuevo, pero con desempate por PeerId:
                        // sólo el de PeerId menor inicia, para no abrir dos
                        // conexiones cruzadas al mismo par.
                        for d in &nuevas {
                            if let Ok(a) = d.parse::<Multiaddr>() {
                                if let Some(p) = peer_de(&a) {
                                    if p != malla.mi_peer && malla.mi_peer < p {
                                        let _ = malla.cmd_tx.send(Cmd::Conectar(d.clone()));
                                    }
                                }
                            }
                        }
                        // Re-difundir la lista actualizada para propagar la malla.
                        let snap: Vec<String> =
                            malla.conocidas.lock().await.iter().cloned().collect();
                        let pares = Paquete::Pares { direcciones: snap }.codificar();
                        difundir_a_todos(&malla.escritores, &pares).await;
                    }
                }
                Paquete::Adios => break,
            }
        }
        malla.escritores.lock().await.remove(&peer);
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
