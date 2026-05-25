// =============================================================================
//  renaser :: kernel/src/akasha — Fase 20 :: Akasha Over Ether
// -----------------------------------------------------------------------------
//  El servicio del kernel que habla AoE: el respondedor de Akasha Over Ether.
//  Tres oficios:
//
//   1. Drena la cola RX del dispositivo de red y DEMULTIPLEXA cada frame:
//      - Si su EtherType es `0x88B5` y el payload deserializa como un
//        `MensajeAkasha`, lo procesa en el kernel.
//      - Cualquier otro frame se encola hacia el userspace: las apps lo
//        recibiran via `sys_net_recibir` como hasta ahora.
//   2. Atiende los mensajes AoE:
//      - `SolicitarObjeto(id)`     → si tenemos `id` en el grafo,
//                                    respondemos con `ProveedorObjeto`.
//      - `ProveedorObjeto(id, p)`  → si la integridad cuadra, lo absorbemos
//                                    al grafo local.
//      - `AnunciarRaiz(id)`        → se contabiliza y, si no tenemos el
//                                    nodo, se le solicita al emisor.
//   3. Difunde periodicamente nuestra raiz del grafo (`AnunciarRaiz` con el
//      hash del manifiesto) — el faro que delata nuestra presencia.
//
//  Con esto, el grafo de objetos —direccionado por contenido, ya BLAKE3— deja
//  de ser una propiedad local del disco y empieza a ser una propiedad
//  distribuida del cable. Tres frames bastan, sin TCP, sin IP, sin DNS.
// =============================================================================

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::fmt::Write;
use core::sync::atomic::{AtomicU64, Ordering};

use spin::{Mutex, Once};

use crate::async_system::reloj;

use akasha::{
    analizar_frame, componer_frame, ErrorAkasha, Mac, MensajeAkasha, MAC_BROADCAST,
};
use formato::Hash;

use crate::almacen;
use crate::baliza;
use crate::drivers::red;

// =============================================================================
//  Cola del userspace — frames NO-AoE en espera de `sys_net_recibir`
// =============================================================================

/// Cuantos frames como mucho retenemos en la cola del userspace. Cota dura:
/// si el userspace se duerme, los frames mas antiguos se pierden antes que
/// la cola se desborde.
const PROFUNDIDAD_COLA_USUARIO: usize = 64;

/// La cola FIFO de frames que NO son AoE y aguardan a `sys_net_recibir`. Cada
/// frame se copia tal cual lo entrega el driver; el userspace ve la cabecera
/// Ethernet completa, como antes de la Fase 20.
static COLA_USUARIO: Mutex<VecDeque<Vec<u8>>> = Mutex::new(VecDeque::new());

// =============================================================================
//  Contadores — la voz que la barra y el diagnostico leen
// =============================================================================

/// Frames AoE procesados en RX, por variante.
static RX_SOLICITUDES: AtomicU64 = AtomicU64::new(0);
static RX_PROVEEDORES: AtomicU64 = AtomicU64::new(0);
static RX_ANUNCIOS: AtomicU64 = AtomicU64::new(0);

/// Frames AoE emitidos en TX, por variante.
static TX_SOLICITUDES: AtomicU64 = AtomicU64::new(0);
static TX_PROVEEDORES: AtomicU64 = AtomicU64::new(0);
static TX_ANUNCIOS: AtomicU64 = AtomicU64::new(0);

/// Frames descartados en RX por ser basura (payload AoE invalido).
static RX_DESCARTADOS: AtomicU64 = AtomicU64::new(0);

/// Frames no-AoE encolados hacia el userspace.
static USUARIO_ENCOLADOS: AtomicU64 = AtomicU64::new(0);
/// Frames no-AoE descartados por desbordamiento de la cola del userspace.
static USUARIO_DESBORDADOS: AtomicU64 = AtomicU64::new(0);

/// La MAC con la que firmamos los frames AoE. La cachea `montar`.
static NUESTRA_MAC: Once<Mac> = Once::new();

// =============================================================================
//  Montaje
// =============================================================================

/// Anota la MAC del dispositivo de red. La llama el orquestador cuando el
/// driver entrega su `Mac` tras `red::montar`.
pub fn montar(mac: Mac) {
    NUESTRA_MAC.call_once(|| mac);
}

// =============================================================================
//  Demultiplexor — el oficio numero 1 de la Fase 20
// =============================================================================

/// Drena la cola RX del dispositivo y demultiplexa cada frame:
/// - Frames AoE (EtherType `0x88B5` con payload valido) → `procesar`.
/// - Cualquier otro frame → cola del userspace (lo recogera `sys_net_recibir`).
///
/// Llamada en CADA fotograma desde la tarea cooperativa de red. El cerrojo
/// del driver virtio-net se libera entre frame y frame; el del userspace solo
/// se toma para empujar, sin solapar con el del driver.
pub fn drenar_y_demultiplexar() {
    let mac = match NUESTRA_MAC.get().copied() {
        Some(m) => m,
        None => return,
    };
    red::drenar_rx(|frame| {
        // Intentar analizar como AoE. Si el EtherType cuadra Y el payload
        // deserializa, procesamos en el kernel; el frame NO viaja al
        // userspace —el protocolo es asunto del nucleo—.
        match analizar_frame(frame) {
            Ok((origen, mensaje)) => procesar(mensaje, origen, mac),
            // EtherType ajeno o frame truncado: a la cola del userspace.
            Err(ErrorAkasha::EtherTypeAjeno) | Err(ErrorAkasha::FrameDemasiadoCorto) => {
                encolar_para_usuario(frame)
            }
            // EtherType nuestro pero payload no-postcard: el ejemplo canonico
            // es el saludo en texto plano de `pregon`. NO es Akasha legitimo,
            // pero tampoco basura del cable — es un protocolo userspace que
            // comparte EtherType. Lo contamos y lo dejamos pasar al userspace
            // para que el app destinatario (si lo hay) lo recoja.
            Err(ErrorAkasha::PayloadInvalido) | Err(ErrorAkasha::PayloadDemasiadoLargo) => {
                RX_DESCARTADOS.fetch_add(1, Ordering::Relaxed);
                encolar_para_usuario(frame);
            }
        }
    });
}

/// Encola un frame entero hacia el userspace. Si la cola esta llena, descarta
/// el mas antiguo —preferimos perder el pasado a quedar sordos del futuro—.
fn encolar_para_usuario(frame: &[u8]) {
    let mut cola = COLA_USUARIO.lock();
    if cola.len() >= PROFUNDIDAD_COLA_USUARIO {
        cola.pop_front();
        USUARIO_DESBORDADOS.fetch_add(1, Ordering::Relaxed);
    }
    cola.push_back(frame.to_vec());
    USUARIO_ENCOLADOS.fetch_add(1, Ordering::Relaxed);
}

// =============================================================================
//  Interfaz que ve `sys_net_recibir`
// =============================================================================

/// Saca UN frame de la cola del userspace hacia `buf`. Devuelve los bytes
/// copiados (acotados por `buf.len()`), o `0` si no hay frame pendiente. El
/// reemplazo natural de `red::recibir_en` para el lado WASM —ahora el
/// userspace ya no compite con el kernel por la cola RX del dispositivo—.
pub fn pop_usuario(buf: &mut [u8]) -> usize {
    let frame = match COLA_USUARIO.lock().pop_front() {
        Some(f) => f,
        None => return 0,
    };
    let n = frame.len().min(buf.len());
    buf[..n].copy_from_slice(&frame[..n]);
    n
}

// =============================================================================
//  Atencion de mensajes — el oficio numero 2
// =============================================================================

/// Procesa UN mensaje AoE entrante. Contabiliza, traza y —si toca— responde.
fn procesar(mensaje: MensajeAkasha, origen: Mac, nuestra: Mac) {
    match mensaje {
        MensajeAkasha::SolicitarObjeto(id) => {
            RX_SOLICITUDES.fetch_add(1, Ordering::Relaxed);
            atender_solicitud(id, origen, nuestra);
        }
        MensajeAkasha::ProveedorObjeto(id, payload) => {
            RX_PROVEEDORES.fetch_add(1, Ordering::Relaxed);
            absorber_proveedor(id, &payload, origen);
        }
        MensajeAkasha::AnunciarRaiz(id) => {
            RX_ANUNCIOS.fetch_add(1, Ordering::Relaxed);
            atender_anuncio(id, origen, nuestra);
        }
    }
}

/// Si tenemos el objeto que nos piden, le respondemos al solicitante (unicast)
/// con un `ProveedorObjeto` que carga la forma serializada del nodo. La
/// integridad se preserva por construccion: `payload` es exactamente la
/// secuencia que rehashea al `id` pedido. Si no lo tenemos, no decimos nada
/// —AoE no tiene «not found»; un par puede preguntarle a otro—.
fn atender_solicitud(id: Hash, origen: Mac, nuestra: Mac) {
    let objeto = match almacen::recuperar(&id) {
        Ok(Some(o)) => o,
        Ok(None) => {
            let _ = writeln!(
                baliza::Serie,
                "akasha :: solicitud rechazada (objeto ausente) :: {}",
                FormatoHash(&id)
            );
            return;
        }
        Err(motivo) => {
            let _ = writeln!(baliza::Serie, "akasha :: solicitud fallida :: {motivo}");
            return;
        }
    };
    let payload = match objeto.serializar() {
        Ok(p) => p,
        Err(_) => return,
    };
    // Defensa en profundidad: el rehash DEBE coincidir. postcard es canonico
    // pero verificamos antes de poner algo en el cable que dice ser `id`.
    if formato::hash(&payload) != id {
        let _ = writeln!(
            baliza::Serie,
            "akasha :: rehash no coincide al servir :: descartado"
        );
        return;
    }
    let mensaje = MensajeAkasha::ProveedorObjeto(id, payload);
    if enviar(&mensaje, nuestra, origen).is_ok() {
        TX_PROVEEDORES.fetch_add(1, Ordering::Relaxed);
        let _ = writeln!(
            baliza::Serie,
            "akasha :: PROVEEDOR enviado :: {} -> {}",
            FormatoHash(&id),
            FormatoMac(&origen)
        );
    }
}

/// Acepta un objeto venido del cable. Verifica que su forma serializada
/// rehashea al `id` que el remitente afirma; si cuadra, lo deposita en el
/// grafo local; si no, lo descarta con una traza. La unica entrada de
/// integridad esta aqui: el grafo local no admite mentiras.
fn absorber_proveedor(id: Hash, payload: &[u8], origen: Mac) {
    if formato::hash(payload) != id {
        let _ = writeln!(
            baliza::Serie,
            "akasha :: proveedor rechazado (rehash no coincide) :: src={}",
            FormatoMac(&origen)
        );
        return;
    }
    let objeto = match formato::Objeto::deserializar(payload) {
        Ok(o) => o,
        Err(motivo) => {
            let _ = writeln!(
                baliza::Serie,
                "akasha :: proveedor no deserializable :: {motivo}"
            );
            return;
        }
    };
    match almacen::almacenar(objeto.datos, objeto.hijos) {
        Ok(hash) => {
            let _ = writeln!(
                baliza::Serie,
                "akasha :: PROVEEDOR absorbido :: {} <- {}",
                FormatoHash(&hash),
                FormatoMac(&origen)
            );
        }
        Err(motivo) => {
            let _ = writeln!(
                baliza::Serie,
                "akasha :: no se pudo absorber proveedor :: {motivo}"
            );
        }
    }
}

/// Un par nos anuncio su raiz. Si la conocemos, no hay nada que hacer; si no,
/// le pedimos el nodo: el `AnunciarRaiz` es el faro que basta para iniciar la
/// replicacion.
fn atender_anuncio(id: Hash, origen: Mac, nuestra: Mac) {
    // ¿Lo tenemos ya? Entonces nada que pedir.
    if let Ok(Some(_)) = almacen::recuperar(&id) {
        let _ = writeln!(
            baliza::Serie,
            "akasha :: anuncio reconocido (ya lo tenemos) :: {} de {}",
            FormatoHash(&id),
            FormatoMac(&origen)
        );
        return;
    }
    // No lo tenemos — pedirlo al emisor en unicast.
    let mensaje = MensajeAkasha::SolicitarObjeto(id);
    if enviar(&mensaje, nuestra, origen).is_ok() {
        TX_SOLICITUDES.fetch_add(1, Ordering::Relaxed);
        let _ = writeln!(
            baliza::Serie,
            "akasha :: SOLICITUD enviada :: {} -> {}",
            FormatoHash(&id),
            FormatoMac(&origen)
        );
    }
}

// =============================================================================
//  Difusion periodica de nuestra raiz — el oficio numero 3
// =============================================================================

/// Intervalo entre faros AoE consecutivos, en milisegundos. 5 s es un
/// compromiso conservador: lo bastante frecuente para descubrir vecinos
/// nuevos sin saturar la red de capa-2 con anuncios.
const INTERVALO_FARO_MS: u64 = 5_000;

/// Marca del reloj monotono (`reloj::milisegundos`) en la que se difundira el
/// proximo faro. La inicializamos a `0` — la primera difusion ocurre cuanto
/// antes pase `tic_compositor` con el manifiesto montado—.
static PROXIMO_FARO_MS: AtomicU64 = AtomicU64::new(0);

/// Punto de entrada que el tic del compositor llama una vez por fotograma.
/// Junta los dos oficios AoE que viven en linea con el latido del escritorio:
///   - drenar la cola RX del dispositivo y demultiplexar Akasha vs userspace,
///   - difundir nuestra raiz cada `INTERVALO_FARO_MS` segun reloj monotono.
///
/// El compositor late de forma fiable (avanza el reloj de la barra cada
/// segundo, prueba indirecta de que su tarea se atiende sin atascos); por
/// eso elegimos su tic como portador. La cadencia del faro se mide contra
/// el reloj monotono y NO contra los awaits, asi el ritmo del faro es
/// independiente de cuanto trabajo del reactor consume cada vuelta.
pub fn tic_compositor() {
    drenar_y_demultiplexar();
    let ahora = reloj::milisegundos();
    let proximo = PROXIMO_FARO_MS.load(Ordering::Relaxed);
    if ahora >= proximo {
        difundir_raiz();
        PROXIMO_FARO_MS.store(ahora + INTERVALO_FARO_MS, Ordering::Relaxed);
    }
}

/// Anuncia, por broadcast, el hash del manifiesto actual. Es el faro de
/// renaser: quien escuche en la red de capa-2 sabra de nuestra existencia y
/// del nodo raiz del grafo. Si aun no hay manifiesto anclado, no se difunde
/// nada — el silencio es preferible a un faro vacio.
pub fn difundir_raiz() {
    let nuestra = match NUESTRA_MAC.get().copied() {
        Some(m) => m,
        None => return,
    };
    let id = match almacen::manifiesto() {
        Some(h) => h,
        None => return,
    };
    let mensaje = MensajeAkasha::AnunciarRaiz(id);
    if enviar(&mensaje, nuestra, MAC_BROADCAST).is_ok() {
        TX_ANUNCIOS.fetch_add(1, Ordering::Relaxed);
        let _ = writeln!(
            baliza::Serie,
            "akasha :: ANUNCIO emitido :: raiz={}",
            FormatoHash(&id)
        );
    }
}

// =============================================================================
//  Helpers internos
// =============================================================================

/// Compone un frame AoE y lo entrega al driver. Punto de salida unico de TX.
fn enviar(mensaje: &MensajeAkasha, src: Mac, dst: Mac) -> Result<(), ()> {
    let frame = componer_frame(src, dst, mensaje).map_err(|_| ())?;
    red::enviar(&frame).map_err(|motivo| {
        let _ = writeln!(baliza::Serie, "akasha :: envio fallido :: {motivo}");
    })
}

// =============================================================================
//  Lectores de estado — los expone la barra de tareas (futuro indicador) y el
//  diagnostico de COM1
// =============================================================================

/// Resumen de actividad AoE, en una sola lectura coherente. Los enteros se
/// leen `Relaxed`: el resumen es informativo, no transaccional.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct ResumenAkasha {
    pub rx_solicitudes: u64,
    pub rx_proveedores: u64,
    pub rx_anuncios: u64,
    pub tx_solicitudes: u64,
    pub tx_proveedores: u64,
    pub tx_anuncios: u64,
    pub rx_descartados: u64,
    pub usuario_encolados: u64,
    pub usuario_desbordados: u64,
}

#[allow(dead_code)]
pub fn resumen() -> ResumenAkasha {
    ResumenAkasha {
        rx_solicitudes: RX_SOLICITUDES.load(Ordering::Relaxed),
        rx_proveedores: RX_PROVEEDORES.load(Ordering::Relaxed),
        rx_anuncios: RX_ANUNCIOS.load(Ordering::Relaxed),
        tx_solicitudes: TX_SOLICITUDES.load(Ordering::Relaxed),
        tx_proveedores: TX_PROVEEDORES.load(Ordering::Relaxed),
        tx_anuncios: TX_ANUNCIOS.load(Ordering::Relaxed),
        rx_descartados: RX_DESCARTADOS.load(Ordering::Relaxed),
        usuario_encolados: USUARIO_ENCOLADOS.load(Ordering::Relaxed),
        usuario_desbordados: USUARIO_DESBORDADOS.load(Ordering::Relaxed),
    }
}

// =============================================================================
//  Formateadores cortos para la traza de COM1 — sin allocations innecesarias
// =============================================================================

struct FormatoHash<'h>(&'h Hash);
impl core::fmt::Display for FormatoHash<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Primeros 8 bytes en hex — suficiente para distinguir en una traza.
        for b in &self.0[..8] {
            write!(f, "{b:02x}")?;
        }
        write!(f, "..")
    }
}

struct FormatoMac<'m>(&'m Mac);
impl core::fmt::Display for FormatoMac<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ":")?;
            }
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}
