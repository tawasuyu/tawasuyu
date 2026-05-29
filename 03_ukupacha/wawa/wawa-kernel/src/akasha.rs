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
use core::fmt::Write;
use core::sync::atomic::{AtomicU64, Ordering};

use spin::{Mutex, Once};

use crate::async_system::reloj;

use akasha::{
    analizar_frame, componer_frame, ErrorAkasha, Mac, MensajeAkasha, MAC_BROADCAST,
};
use format::Hash;

use crate::almacen;
use crate::baliza;
use crate::drivers::red;

// =============================================================================
//  Cola del userspace — frames NO-AoE en espera de `sys_net_recibir`
// -----------------------------------------------------------------------------
//  ZERO-ALLOC (Fase 55) :: la cola es un anillo de slots MTU pre-alocados en
//  `.bss`, no un `VecDeque<Vec<u8>>` que aloca por cada frame entrante. Dos
//  pistas de indices `u8` orquestan el anillo:
//    - `fifo`: indices de slots ocupados, en orden de llegada (FIFO).
//    - `libres`: indices de slots disponibles, pila LIFO (free-list).
//  `push` copia el frame al slot que asoma la pila libres y encola su indice;
//  `pop` desencola un indice, copia su slot al buffer del userspace y devuelve
//  el indice a la pila libres. Cuando el anillo esta lleno, el slot del frame
//  mas antiguo se libera —preferimos perder pasado a quedar sordos del
//  futuro—. Cero `to_vec()`, cero `push_back` que alocan, cero `pop_front`
//  que liberan.
// =============================================================================

/// Cuantos frames como mucho retenemos en la cola del userspace. Cota dura:
/// si el userspace se duerme, los frames mas antiguos se pierden antes que
/// la cola se desborde. Debe caber en `u8` (los indices viajan como `u8`).
const PROFUNDIDAD_COLA_USUARIO: usize = 64;
const _: () = assert!(PROFUNDIDAD_COLA_USUARIO <= u8::MAX as usize + 1);

/// Capacidad de cada slot del anillo. MTU clasico de Ethernet (1500 datos +
/// 14 cabecera + holgura): cualquier frame del cable comun cabe entero. El
/// driver `red::drenar_rx` ya recorta frames mayores antes de llegar aqui.
const SLOT_CAPACIDAD: usize = 2048;

/// Un slot del anillo: capacidad fija MTU + bytes utiles. La capacidad se
/// reserva al fundar el kernel; los frames variables se acomodan adentro.
#[derive(Clone, Copy)]
struct SlotCola {
    bytes: [u8; SLOT_CAPACIDAD],
    len: u16,
}

impl SlotCola {
    const fn nuevo() -> Self {
        Self { bytes: [0u8; SLOT_CAPACIDAD], len: 0 }
    }
}

/// Anillo de slots MTU + dos pistas de indices. Ocupacion: 64 * 2050 B
/// = ~128 KiB en `.bss` del kernel, mas un epsilon de contadores. Cero
/// alocacion dinamica en el path RX.
struct AnilloCola {
    slots: [SlotCola; PROFUNDIDAD_COLA_USUARIO],
    /// Indices de slots ocupados, FIFO circular sobre `[u8; N]`.
    fifo: [u8; PROFUNDIDAD_COLA_USUARIO],
    /// Indice del slot mas antiguo dentro de `fifo`.
    fifo_ini: u8,
    /// Cuantos slots estan ocupados ahora. `fifo_ini + fifo_n mod N` apunta
    /// al primer hueco.
    fifo_n: u8,
    /// Indices de slots disponibles, pila LIFO.
    libres: [u8; PROFUNDIDAD_COLA_USUARIO],
    /// Cuantos indices libres hay en la pila. Invariante: `fifo_n + libres_n
    /// == PROFUNDIDAD_COLA_USUARIO` siempre — cada indice habita una sola
    /// pista.
    libres_n: u8,
}

impl AnilloCola {
    /// `const fn` para vivir como `static`: arranca con todos los slots
    /// libres (`libres = [0, 1, ..., N-1]`), `fifo_n = 0`.
    const fn nuevo() -> Self {
        let mut libres = [0u8; PROFUNDIDAD_COLA_USUARIO];
        let mut i = 0;
        while i < PROFUNDIDAD_COLA_USUARIO {
            libres[i] = i as u8;
            i += 1;
        }
        Self {
            slots: [SlotCola::nuevo(); PROFUNDIDAD_COLA_USUARIO],
            fifo: [0u8; PROFUNDIDAD_COLA_USUARIO],
            fifo_ini: 0,
            fifo_n: 0,
            libres,
            libres_n: PROFUNDIDAD_COLA_USUARIO as u8,
        }
    }

    /// Encola un frame copiandolo a un slot libre. Si el anillo esta lleno,
    /// libera el slot mas antiguo y reutiliza su indice — descarte FIFO
    /// natural, sin alocacion intermedia. El contador
    /// `USUARIO_DESBORDADOS` lo lleva el llamante (encolar_para_usuario).
    fn empujar(&mut self, frame: &[u8]) -> bool {
        let lleno = self.fifo_n as usize >= PROFUNDIDAD_COLA_USUARIO;
        if lleno {
            // Liberar el slot mas antiguo. Su indice vuelve a la pila libres.
            let idx = self.fifo[self.fifo_ini as usize];
            self.fifo_ini = ((self.fifo_ini as usize + 1) % PROFUNDIDAD_COLA_USUARIO) as u8;
            self.fifo_n -= 1;
            self.libres[self.libres_n as usize] = idx;
            self.libres_n += 1;
        }
        // Tomar un slot libre de la pila. Invariante: siempre hay al menos
        // uno tras la poda anterior.
        self.libres_n -= 1;
        let idx = self.libres[self.libres_n as usize];
        let slot = &mut self.slots[idx as usize];
        let n = frame.len().min(SLOT_CAPACIDAD);
        slot.bytes[..n].copy_from_slice(&frame[..n]);
        slot.len = n as u16;
        // Encolar el indice al final del FIFO circular.
        let fin = (self.fifo_ini as usize + self.fifo_n as usize) % PROFUNDIDAD_COLA_USUARIO;
        self.fifo[fin] = idx;
        self.fifo_n += 1;
        lleno
    }

    /// Saca el frame mas antiguo a `buf`, devuelve los bytes copiados. El
    /// slot del frame retorna a la pila libres para el proximo `empujar`.
    fn sacar(&mut self, buf: &mut [u8]) -> usize {
        if self.fifo_n == 0 {
            return 0;
        }
        let idx = self.fifo[self.fifo_ini as usize];
        self.fifo_ini = ((self.fifo_ini as usize + 1) % PROFUNDIDAD_COLA_USUARIO) as u8;
        self.fifo_n -= 1;
        let slot = &self.slots[idx as usize];
        let n = (slot.len as usize).min(buf.len());
        buf[..n].copy_from_slice(&slot.bytes[..n]);
        // Devolver el slot a la pila libres para reciclar.
        self.libres[self.libres_n as usize] = idx;
        self.libres_n += 1;
        n
    }
}

/// La cola FIFO de frames que NO son AoE y aguardan a `sys_net_recibir`. Vive
/// integramente en `.bss` — el anillo se construye en `const fn` y entra al
/// kernel como dato estatico, no como alocacion en el heap.
static COLA_USUARIO: Mutex<AnilloCola> = Mutex::new(AnilloCola::nuevo());

// =============================================================================
//  Contadores — la voz que la barra y el diagnostico leen
// =============================================================================

/// Frames AoE procesados en RX, por variante.
static RX_SOLICITUDES: AtomicU64 = AtomicU64::new(0);
static RX_PROVEEDORES: AtomicU64 = AtomicU64::new(0);
static RX_ANUNCIOS: AtomicU64 = AtomicU64::new(0);
/// Anuncios de canal recibidos. Independiente de `RX_ANUNCIOS` (que cuenta
/// `AnunciarRaiz`): un anuncio de canal lleva una recomendacion FIRMADA por
/// un autor agora, y la politica de "actualizar" la decide el userspace.
static RX_ANUNCIOS_CANAL: AtomicU64 = AtomicU64::new(0);

/// El ULTIMO anuncio de canal recibido, con su firma intacta. La capa-2 no
/// garantiza orden ni unicidad; guardamos el mas reciente en una sola ranura
/// y dejamos que el userspace (la app `mudanza`) lo lea por
/// `sys_canal_anuncio`, se lo muestre al operador, y —si este acepta— pida
/// re-anclar por `sys_canal_aceptar`. El kernel NO verifica la firma al
/// recibir (no conoce aun el nombre del canal hasta que el objeto `Canal`
/// llega via `ProveedorObjeto`); la verificacion soberana ocurre integra en
/// `sys_canal_aceptar`. Esta ranura es solo el buzon de "hay una propuesta".
static ULTIMO_ANUNCIO: Mutex<Option<AnuncioCanal>> = Mutex::new(None);

/// Los campos firmados de un `MensajeAkasha::AnunciarCanal`, retenidos para
/// que el userspace los recoja. Espeja el layout de 168 B que `sys_canal_anuncio`
/// expone y que `agora-cli wawa publicar` escribe en `anuncio.bin`.
#[derive(Clone, Copy)]
pub struct AnuncioCanal {
    /// Hash del objeto `format::Canal` (lleva el nombre + el historial firmado).
    pub canal: Hash,
    /// Hash del manifiesto que el autor recomienda anclar.
    pub raiz: Hash,
    /// Clave publica Ed25519 del autor del anuncio.
    pub autor: [u8; 32],
    /// Segundos UNIX de la recomendacion; parte del mensaje firmado.
    pub timestamp: u64,
    /// Firma Ed25519 sobre `format::mensaje_a_firmar(nombre, timestamp, raiz)`.
    pub firma: [u8; 64],
}

/// Devuelve una copia del ultimo anuncio de canal recibido, o `None` si aun
/// no llego ninguno. La toma `sys_canal_anuncio` para volcarla al userspace.
pub fn ultimo_anuncio() -> Option<AnuncioCanal> {
    *ULTIMO_ANUNCIO.lock()
}

/// Frames AoE emitidos en TX, por variante.
static TX_SOLICITUDES: AtomicU64 = AtomicU64::new(0);
static TX_PROVEEDORES: AtomicU64 = AtomicU64::new(0);
static TX_ANUNCIOS: AtomicU64 = AtomicU64::new(0);

/// Frames descartados en RX por ser basura (payload AoE invalido).
static RX_DESCARTADOS: AtomicU64 = AtomicU64::new(0);

/// Solicitudes que el dedup descarto: un mismo par pidio el mismo objeto
/// dentro de `VENTANA_DEDUP_MS` — la primera respuesta ya esta en vuelo
/// (o el grafo no la tiene), repetir es ruido.
static RX_SOLICITUDES_DEDUP: AtomicU64 = AtomicU64::new(0);

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
/// Cero alocacion: la copia entra a un slot del anillo MTU pre-alocado.
fn encolar_para_usuario(frame: &[u8]) {
    let desbordo = COLA_USUARIO.lock().empujar(frame);
    if desbordo {
        USUARIO_DESBORDADOS.fetch_add(1, Ordering::Relaxed);
    }
    USUARIO_ENCOLADOS.fetch_add(1, Ordering::Relaxed);
}

// =============================================================================
//  Interfaz que ve `sys_net_recibir`
// =============================================================================

/// Saca UN frame de la cola del userspace hacia `buf`. Devuelve los bytes
/// copiados (acotados por `buf.len()`), o `0` si no hay frame pendiente. El
/// reemplazo natural de `red::recibir_en` para el lado WASM —ahora el
/// userspace ya no compite con el kernel por la cola RX del dispositivo—.
/// Cero alocacion: el slot del anillo retorna a la pila libres.
pub fn pop_usuario(buf: &mut [u8]) -> usize {
    COLA_USUARIO.lock().sacar(buf)
}

// =============================================================================
//  Dedup de solicitudes recientes — pareja con la retransmision del cliente
// =============================================================================
//
//  El cliente AoE de `wawa-explorer-aoe` reenvia una `SolicitarObjeto` hasta
//  `INTENTOS_SOLICITAR` veces dentro de un mismo `timeout` para tolerar
//  broadcast perdido. Si NO dedupamos del lado del kernel, cada uno de esos
//  reenvios dispara una `ProveedorObjeto` unicast — ruido innecesario en la
//  red de capa 2 y trabajo redundante en el almacen.
//
//  Solucion minima: una ventana corta `VENTANA_DEDUP_MS` durante la cual el
//  primer par que repite la misma `(MAC, hash)` queda silenciado. Una vez
//  servida la primera, las repeticiones se descuentan a contador y se
//  ignoran.

/// Cuanto tiempo (ms) consideramos "la misma rafaga" del mismo par pidiendo
/// el mismo objeto. Acotado por arriba al timeout total tipico del cliente
/// (3 s) para que la rafaga completa de retransmisiones quepa adentro.
const VENTANA_DEDUP_MS: u64 = 3_000;

/// Cuantas (MAC, hash, ms) recordamos a la vez. Mas grande = mas memoria de
/// rafagas paralelas; mas chico = un par ruidoso desplaza a otro. 64 es el
/// mismo orden que `PROFUNDIDAD_COLA_USUARIO`.
const PROFUNDIDAD_DEDUP: usize = 64;

/// Entrada del cache de dedup. Determinista, sin Hash inline para no traer
/// hashing aleatorio al kernel.
#[derive(Clone, Copy)]
struct RegistroSolicitud {
    origen: Mac,
    id: Hash,
    cuando_ms: u64,
}

/// Cache FIFO de solicitudes recientes. Lookup lineal; con PROFUNDIDAD <= 64
/// es trivial y no justifica una estructura mas compleja.
static RECIENTES_SOLICITUDES: Mutex<VecDeque<RegistroSolicitud>> =
    Mutex::new(VecDeque::new());

/// Devuelve `true` si esta solicitud es un duplicado dentro de la ventana —
/// el caller debe descartarla. Si no lo es, la registra y devuelve `false`.
///
/// La pasada de purga de entradas viejas (mas alla de la ventana) ocurre en
/// el mismo lock, asi que no acumulamos basura mas alla del horizonte util.
fn es_duplicado_y_registrar(origen: Mac, id: Hash, ahora_ms: u64) -> bool {
    let mut cache = RECIENTES_SOLICITUDES.lock();
    // Purgar entradas vencidas (las que se acumulan al frente por ser viejas).
    while let Some(reg) = cache.front() {
        if ahora_ms.saturating_sub(reg.cuando_ms) > VENTANA_DEDUP_MS {
            cache.pop_front();
        } else {
            break;
        }
    }
    // Buscar duplicado dentro de la ventana.
    for reg in cache.iter() {
        if reg.origen == origen
            && reg.id == id
            && ahora_ms.saturating_sub(reg.cuando_ms) <= VENTANA_DEDUP_MS
        {
            return true;
        }
    }
    // No es duplicado — registrar. Si la cola esta llena, descartar la mas
    // antigua para hacer lugar (otro guardarrail termodinamico clasico).
    if cache.len() >= PROFUNDIDAD_DEDUP {
        cache.pop_front();
    }
    cache.push_back(RegistroSolicitud { origen, id, cuando_ms: ahora_ms });
    false
}

// =============================================================================
//  Atencion de mensajes — el oficio numero 2
// =============================================================================

/// Procesa UN mensaje AoE entrante. Contabiliza, traza y —si toca— responde.
fn procesar(mensaje: MensajeAkasha, origen: Mac, nuestra: Mac) {
    match mensaje {
        MensajeAkasha::SolicitarObjeto(id) => {
            RX_SOLICITUDES.fetch_add(1, Ordering::Relaxed);
            // Dedup: la rafaga de retransmision del cliente (3 broadcasts
            // dentro del mismo timeout) NO debe dispararnos 3 respuestas.
            let ahora = reloj::milisegundos();
            if es_duplicado_y_registrar(origen, id, ahora) {
                RX_SOLICITUDES_DEDUP.fetch_add(1, Ordering::Relaxed);
                let _ = writeln!(
                    baliza::Serie,
                    "akasha :: solicitud dedupada (rafaga reciente) :: {} de {}",
                    FormatoHash(&id),
                    FormatoMac(&origen)
                );
                return;
            }
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
        MensajeAkasha::AnunciarCanal { canal, raiz, autor, timestamp, firma } => {
            // El kernel NO verifica la firma ni decide si actualizar al RECIBIR
            // —no conoce el nombre del canal hasta que el objeto `Canal` llega—.
            // Aqui ingesta el DAG (pide `Canal` y la raiz si faltan) Y retiene
            // el anuncio en la ranura `ULTIMO_ANUNCIO` para que la app `mudanza`
            // lo lea por `sys_canal_anuncio`. La verificacion soberana (anillo +
            // firma canonica) y la re-ancla ocurren integras en `sys_canal_aceptar`
            // cuando el operador acepta. Aqui solo anotamos "hay una propuesta".
            RX_ANUNCIOS_CANAL.fetch_add(1, Ordering::Relaxed);
            *ULTIMO_ANUNCIO.lock() = Some(AnuncioCanal {
                canal,
                raiz,
                autor,
                timestamp,
                firma,
            });
            atender_anuncio_canal(canal, raiz, autor, timestamp, origen, nuestra);
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
    if format::hash(&payload) != id {
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
    if format::hash(payload) != id {
        let _ = writeln!(
            baliza::Serie,
            "akasha :: proveedor rechazado (rehash no coincide) :: src={}",
            FormatoMac(&origen)
        );
        return;
    }
    let objeto = match format::Objeto::deserializar(payload) {
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

/// Atiende un `AnunciarCanal`: pide al emisor el objeto `Canal` y la raiz de
/// manifiesto si el almacen local no los tiene. No verifica la firma —el
/// kernel no carga criptografia de identidad— ni reancla nada; solo ingesta
/// los nodos del DAG para que la app `mudanza` los pueda leer, verificar y
/// decidir. Los parametros `autor` y `timestamp` van a la traza para
/// diagnostico; la firma se descarto en el match.
fn atender_anuncio_canal(
    canal: Hash,
    raiz: Hash,
    autor: [u8; 32],
    timestamp: u64,
    origen: Mac,
    nuestra: Mac,
) {
    let _ = writeln!(
        baliza::Serie,
        "akasha :: anuncio de canal :: canal={} raiz={} autor={} ts={} de {}",
        FormatoHash(&canal),
        FormatoHash(&raiz),
        FormatoHash(&autor),
        timestamp,
        FormatoMac(&origen)
    );
    // Pedir `canal` si nos falta.
    if matches!(almacen::recuperar(&canal), Ok(None)) {
        let mensaje = MensajeAkasha::SolicitarObjeto(canal);
        if enviar(&mensaje, nuestra, origen).is_ok() {
            TX_SOLICITUDES.fetch_add(1, Ordering::Relaxed);
        }
    }
    // Pedir la raiz de manifiesto si nos falta.
    if matches!(almacen::recuperar(&raiz), Ok(None)) {
        let mensaje = MensajeAkasha::SolicitarObjeto(raiz);
        if enviar(&mensaje, nuestra, origen).is_ok() {
            TX_SOLICITUDES.fetch_add(1, Ordering::Relaxed);
        }
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

/// Difunde, por broadcast, una solicitud de objeto por su hash. Es la
/// version "tira del cable" del fetch — la userspace la activa cuando
/// `almacen::recuperar(&hash)` devuelve `None` y quiere intentar
/// completarse desde la red. El kernel ya tiene el camino de respuesta
/// montado: cuando un par conteste con `ProveedorObjeto`, el
/// demultiplexer lo absorbe via `absorber_proveedor`, y el siguiente
/// `recuperar` lo encontrara en el almacen local.
///
/// Devuelve `Ok(())` si el frame se entrego al driver; `Err(())` si no
/// hay MAC montada (red ausente) o el envio fallo.
pub fn difundir_solicitud(id: Hash) -> Result<(), ()> {
    let nuestra = NUESTRA_MAC.get().copied().ok_or(())?;
    let mensaje = MensajeAkasha::SolicitarObjeto(id);
    enviar(&mensaje, nuestra, MAC_BROADCAST)?;
    TX_SOLICITUDES.fetch_add(1, Ordering::Relaxed);
    let _ = writeln!(
        baliza::Serie,
        "akasha :: SOLICITUD (userspace) :: {}",
        FormatoHash(&id)
    );
    Ok(())
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
    pub rx_anuncios_canal: u64,
    pub tx_solicitudes: u64,
    pub tx_proveedores: u64,
    pub tx_anuncios: u64,
    pub rx_descartados: u64,
    pub rx_solicitudes_dedup: u64,
    pub usuario_encolados: u64,
    pub usuario_desbordados: u64,
}

#[allow(dead_code)]
pub fn resumen() -> ResumenAkasha {
    ResumenAkasha {
        rx_solicitudes: RX_SOLICITUDES.load(Ordering::Relaxed),
        rx_proveedores: RX_PROVEEDORES.load(Ordering::Relaxed),
        rx_anuncios: RX_ANUNCIOS.load(Ordering::Relaxed),
        rx_anuncios_canal: RX_ANUNCIOS_CANAL.load(Ordering::Relaxed),
        tx_solicitudes: TX_SOLICITUDES.load(Ordering::Relaxed),
        tx_proveedores: TX_PROVEEDORES.load(Ordering::Relaxed),
        tx_anuncios: TX_ANUNCIOS.load(Ordering::Relaxed),
        rx_descartados: RX_DESCARTADOS.load(Ordering::Relaxed),
        rx_solicitudes_dedup: RX_SOLICITUDES_DEDUP.load(Ordering::Relaxed),
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
