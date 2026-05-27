// =============================================================================
//  renaser :: kernel/src/drivers/disco.rs — Fase 6.2 :: el disco asincrono
// -----------------------------------------------------------------------------
//  La Fase 6.1 hizo hablar al disco; pero lo hacia por SONDEO: el procesador se
//  quedaba en un bucle de espera activa vigilando el «used ring» de virtio,
//  incapaz de atender nada mas. La Fase 6.2 lo libera. La E/S de bloques pasa a
//  ser REACTIVA, guiada por la interrupcion fisica del dispositivo:
//
//    * `EsperaDisco` — un `Future` nativo: enviada la peticion, cede la CPU; la
//      IRQ del disco lo despertara cuando el bloque este listo.
//    * `atender_irq` — el punto al que salta el manejador de la IRQ del disco:
//      reconoce la interrupcion en el dispositivo y despierta a quien aguardaba.
//    * `bloquear_en` — el puente para los contextos SINCRONOS (el arranque, las
//      capacidades WASM): lleva un `Future` de disco hasta su final durmiendo la
//      CPU con `hlt` —jamas en espera activa una vez el sistema esta en marcha—.
//
//  Subsisten de la Fase 6.1 el asignador de marcos por mapa de bits (con
//  liberacion real) y `KernelHal`, el puente DMA hacia `virtio-drivers`.
// =============================================================================

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use core::task::{Context, Poll, Waker};

use spin::{Mutex, Once};
use virtio_drivers::device::blk::{BlkReq, BlkResp, VirtIOBlk, SECTOR_SIZE};
use virtio_drivers::transport::pci::bus::{Command, DeviceFunction, PciRoot};
use virtio_drivers::transport::pci::PciTransport;
use virtio_drivers::{BufferDirection, Hal, PhysAddr};
use x86_64::instructions::interrupts;

use super::pci::CamPuertos;

/// Tamaño de una pagina / marco fisico, en bytes.
const PAGINA: u64 = 4096;

/// Techo de marcos que gestiona el asignador de DMA: 4096 marcos => una arena
/// de 16 MiB. El DMA del disco —colas virtio y buferes rebote— necesita una
/// fraccion minima de eso; el techo solo acota el tamaño del mapa de bits.
const MAX_MARCOS: usize = 4096;

/// Vendor ID de VirtIO; Device IDs de un disco de bloques (transicional/moderno).
const VENDOR_VIRTIO: u16 = 0x1AF4;
const VIRTIO_BLK_IDS: [u16; 2] = [0x1001, 0x1042];

/// El tamaño de un sector, reexportado para el resto del kernel.
pub const TAM_SECTOR: usize = SECTOR_SIZE;

// =============================================================================
//  EL OFFSET DE MEMORIA FISICA Y EL ASIGNADOR DE MARCOS
// =============================================================================

/// Desplazamiento al que el cargador mapeo toda la memoria fisica: una
/// direccion fisica `p` es accesible por el kernel en la virtual `p + offset`.
static OFFSET_FISICO: AtomicU64 = AtomicU64::new(0);

/// Asignador de marcos por MAPA DE BITS: gestiona una arena de RAM fisica
/// contigua y LIBERA. Cada bit representa un marco de 4 KiB; `1` es ocupado,
/// `0` libre. Un almacen de objetos vivo asigna y devuelve marcos sin descanso.
struct AsignadorMarcos {
    /// Direccion fisica del primer marco gestionado, alineada a pagina.
    base: u64,
    /// Numero de marcos que abarca la arena.
    total: usize,
    /// Mapa de ocupacion: un bit por marco.
    mapa: Vec<u64>,
}

impl AsignadorMarcos {
    /// ¿Esta libre el marco `i`?
    fn libre(&self, i: usize) -> bool {
        (self.mapa[i / 64] >> (i % 64)) & 1 == 0
    }

    /// Marca el marco `i` como ocupado.
    fn ocupar(&mut self, i: usize) {
        self.mapa[i / 64] |= 1 << (i % 64);
    }

    /// Marca el marco `i` como libre.
    fn soltar(&mut self, i: usize) {
        self.mapa[i / 64] &= !(1u64 << (i % 64));
    }

    /// Reserva `paginas` marcos CONTIGUOS y devuelve la direccion fisica del
    /// primero. `None` si no hay un hueco contiguo bastante grande.
    fn asignar(&mut self, paginas: usize) -> Option<u64> {
        if paginas == 0 || paginas > self.total {
            return None;
        }
        let mut i = 0;
        while i + paginas <= self.total {
            // Buscar `paginas` marcos libres consecutivos a partir de `i`.
            match (i..i + paginas).find(|&k| !self.libre(k)) {
                // Un marco ocupado rompe la racha: reanudar tras el.
                Some(ocupado) => i = ocupado + 1,
                // Racha completa: ocuparla y entregar su direccion fisica.
                None => {
                    for k in i..i + paginas {
                        self.ocupar(k);
                    }
                    return Some(self.base + (i as u64) * PAGINA);
                }
            }
        }
        None
    }

    /// Devuelve a la arena `paginas` marcos que arrancan en la direccion
    /// fisica `fisica`. Direcciones ajenas a la arena se ignoran sin daño.
    fn liberar(&mut self, fisica: u64, paginas: usize) {
        if fisica < self.base {
            return;
        }
        let inicio = ((fisica - self.base) / PAGINA) as usize;
        for k in inicio..(inicio + paginas).min(self.total) {
            self.soltar(k);
        }
    }
}

/// El asignador global de marcos para DMA. Se funde en `init`.
static ASIGNADOR: Once<Mutex<AsignadorMarcos>> = Once::new();

/// Funda el subsistema de disco: registra el offset de memoria fisica y forja
/// el asignador de marcos sobre la region de RAM libre que el cargador reporto.
/// Una sola vez, antes de montar el disco.
pub fn init(offset_fisico: u64, region_inicio: u64, region_fin: u64) {
    use core::fmt::Write;
    OFFSET_FISICO.store(offset_fisico, Ordering::Relaxed);
    // Saltar SIEMPRE la primera pagina fisica: algunos cargadores la dejan sin
    // mapear como proteccion contra punteros NULL — un marco DMA ahi seria una
    // bomba en cuanto el driver lo desreferenciase via el mapeo alto.
    let base = alinear_arriba(region_inicio.max(PAGINA), PAGINA);
    let disponibles = region_fin.saturating_sub(base) / PAGINA;
    let total = (disponibles as usize).min(MAX_MARCOS);
    let _ = writeln!(
        crate::baliza::Serie,
        "disco :: init offset={:#x} region=[{:#x}, {:#x}) base={:#x} marcos={}",
        offset_fisico,
        region_inicio,
        region_fin,
        base,
        total,
    );
    ASIGNADOR.call_once(|| {
        Mutex::new(AsignadorMarcos {
            base,
            total,
            mapa: vec![0u64; total.div_ceil(64)],
        })
    });
}

/// Redondea `valor` hacia arriba al multiplo de `alineacion` (potencia de dos).
fn alinear_arriba(valor: u64, alineacion: u64) -> u64 {
    (valor + alineacion - 1) & !(alineacion - 1)
}

/// Reserva `paginas` marcos fisicos contiguos de 4 KiB y devuelve su direccion
/// fisica. Agotar la arena es un fallo del kernel, no recuperable aqui: el
/// rasgo `Hal` no admite que `dma_alloc` falle. En ese caso cedemos al sello
/// de la baliza —pantalla carmesí + traza serial sin recorte— en lugar del
/// `panic!` por defecto: el operador ve YA en pantalla qué pasó y se evita
/// que el panic handler corra sobre un estado de I/O comprometido.
///
/// Casos de aborto:
///   1. `ASIGNADOR` no fundado: `init` no se llamó antes del primer
///      `dma_alloc`. Bug de orden de arranque en el kernel — sube prioridad
///      del `init` del subsistema disco.
///   2. Arena exhausta: con `MAX_MARCOS=4096` y la cota per-app
///      `MAX_PAGINAS_DMA_PER_APP=4` × `MAX_VENTANAS=32`=128 simultaneas en
///      vuelo, agotarla solo ocurre si un subsistema interno fuga marcos
///      o si se sube alguna constante sin acompañar a `MAX_MARCOS`.
fn asignar_marcos(paginas: usize) -> u64 {
    let asignador = match ASIGNADOR.get() {
        Some(a) => a,
        None => crate::baliza::aborto_fatal_carmesi(
            b"DMA ARENA NO FUNDADA",
            "DMA :: asignador no inicializado al primer dma_alloc — bug de orden de init del kernel",
        ),
    };
    match asignador.lock().asignar(paginas) {
        Some(fisica) => fisica,
        None => crate::baliza::aborto_fatal_carmesi(
            b"DMA ARENA AGOTADA",
            "DMA :: arena de marcos fisicos exhausta — fuga interna o constantes desbalanceadas",
        ),
    }
}

/// Devuelve `paginas` marcos fisicos a la arena. El reverso de `asignar_marcos`.
fn liberar_marcos(fisica: u64, paginas: usize) {
    if let Some(asignador) = ASIGNADOR.get() {
        asignador.lock().liberar(fisica, paginas);
    }
}

/// Asigna UN marco para servir de tabla de paginas. Sin pánico: si la arena
/// esta exhausta, devuelve `None` y deja al mapeador decidir como reaccionar
/// — el kernel no puede caerse por no poder añadir una tabla intermedia, ya
/// se delatara en cuanto el dispositivo lea su propio MMIO no mapeado.
pub fn asignar_marco_para_tabla() -> Option<u64> {
    ASIGNADOR.get()?.lock().asignar(1)
}

/// Traduce una direccion fisica a la virtual que el kernel puede desreferenciar.
fn a_virtual(fisica: u64) -> *mut u8 {
    (fisica + OFFSET_FISICO.load(Ordering::Relaxed)) as *mut u8
}

// =============================================================================
//  KernelHal — EL PUENTE ENTRE `virtio-drivers` Y LA MEMORIA DE renaser
// =============================================================================

/// La implementacion del rasgo `Hal` de `virtio-drivers`. Sin estado propio:
/// se apoya en el asignador de marcos y el offset fisico, ambos globales.
pub struct KernelHal;

// SEGURIDAD: cada metodo respeta su contrato — `dma_alloc` entrega marcos
// fisicos exclusivos, contiguos, alineados a pagina y a cero; `dma_dealloc` y
// `unshare` los devuelven a la arena; las traducciones de direccion son validas
// porque el cargador mapeo toda la memoria fisica.
unsafe impl Hal for KernelHal {
    fn dma_alloc(paginas: usize, _direccion: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let fisica = asignar_marcos(paginas);
        let virtual_ = a_virtual(fisica);
        // SEGURIDAD: `asignar_marcos` entrego `paginas` marcos exclusivos y
        // contiguos; `virtual_` es su imagen valida y escribible en el mapeo
        // de memoria fisica. El rasgo exige que las paginas vayan a cero.
        unsafe {
            core::ptr::write_bytes(virtual_, 0, paginas * PAGINA as usize);
        }
        (fisica, NonNull::new(virtual_).expect("DMA :: marco fisico nulo"))
    }

    unsafe fn dma_dealloc(fisica: PhysAddr, _virtual_: NonNull<u8>, paginas: usize) -> i32 {
        // Con un asignador real, la liberacion ya NO es un gesto vacio: los
        // marcos vuelven a la arena y quedan disponibles para el proximo DMA.
        liberar_marcos(fisica, paginas);
        0
    }

    unsafe fn mmio_phys_to_virt(fisica: PhysAddr, tam: usize) -> NonNull<u8> {
        // OVMF aloja los BAR prefetchables 64-bit de virtio en la «ventana PCI
        // de 64 bits» —decenas o cientos de GiB de phys—, que el cargador NO
        // mapea. Antes de devolver el puntero virtual, abrimos en la tabla L4
        // las paginas que cubren la region pedida; si ya estaban, no pasa nada.
        crate::memory::mmio::mapear(fisica as u64, tam);
        NonNull::new(a_virtual(fisica)).expect("MMIO :: direccion fisica nula")
    }

    unsafe fn share(bufer: NonNull<[u8]>, direccion: BufferDirection) -> PhysAddr {
        let longitud = bufer.len();
        let paginas = longitud.div_ceil(PAGINA as usize);
        let fisica = asignar_marcos(paginas);
        // Si el bufer viaja HACIA el dispositivo, copiarlo al area DMA rebote.
        if !matches!(direccion, BufferDirection::DeviceToDriver) {
            // SEGURIDAD: el rasgo garantiza que `bufer` apunta a `longitud`
            // bytes validos; el area DMA recien reservada los abarca de sobra.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    bufer.as_ptr() as *const u8,
                    a_virtual(fisica),
                    longitud,
                );
            }
        }
        fisica
    }

    unsafe fn unshare(fisica: PhysAddr, bufer: NonNull<[u8]>, direccion: BufferDirection) {
        let longitud = bufer.len();
        // Si el bufer viene DESDE el dispositivo, copiar el area DMA de vuelta.
        if !matches!(direccion, BufferDirection::DriverToDevice) {
            // SEGURIDAD: `fisica` lo entrego `share` para este mismo `bufer`;
            // ambas regiones abarcan los `longitud` bytes que se copian.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    a_virtual(fisica),
                    bufer.as_ptr() as *mut u8,
                    longitud,
                );
            }
        }
        // Devolver a la arena los marcos del area rebote.
        liberar_marcos(fisica, longitud.div_ceil(PAGINA as usize));
    }
}

// =============================================================================
//  EL DISCO PERSISTENTE
// =============================================================================

/// El disco virtio-blk, ya montado. Envuelve al `VirtIOBlk` para poder ligarlo
/// a un `static`.
struct Disco(VirtIOBlk<KernelHal, PciTransport>);

// SEGURIDAD: `Disco` encierra punteros crudos a las colas virtio y al MMIO del
// dispositivo. renaser es un kernel de un solo nucleo y todo acceso al disco se
// serializa tras el `Mutex` global `DISCO`; jamas se comparte entre hilos
// reales. Todo acceso normal toma el cerrojo con las interrupciones acalladas,
// de modo que la IRQ del disco jamas lo encuentra ocupado.
unsafe impl Send for Disco {}

/// El disco global de renaser. Se monta una sola vez, en `montar`.
static DISCO: Once<Mutex<Disco>> = Once::new();

/// La linea de IRQ del disco, descubierta al montarlo. Vale 0 si el firmware no
/// enruto una linea util: en ese caso la E/S recae en el sondeo, con la red de
/// seguridad del temporizador.
static IRQ_DISCO: AtomicU8 = AtomicU8::new(0);

/// Cuenta de interrupciones del disco atendidas desde el arranque. Es el
/// testigo vivo de que la E/S asincrona late de verdad.
static PULSOS_DISCO: AtomicU64 = AtomicU64::new(0);

/// El waker de la (unica) espera de disco en curso. Las operaciones de disco se
/// serializan, de modo que una sola ranura basta. La IRQ del disco lo invoca.
static WAKER_DISCO: Mutex<Option<Waker>> = Mutex::new(None);

/// Enumera el bus PCI, localiza el disco virtio-blk, lo monta y lo deja tras el
/// `Mutex` global. Descubre ademas su linea de IRQ, registra el manejador y
/// abre la linea en el PIC: desde aqui el disco puede interrumpir. Devuelve la
/// capacidad del disco en sectores. Toda falla se devuelve como `Err`.
pub fn montar() -> Result<u64, &'static str> {
    let mut raiz = PciRoot::new(CamPuertos);

    // 1. Localizar el primer disco virtio-blk recorriendo el bus.
    let mut hallado: Option<DeviceFunction> = None;
    'busqueda: for bus in 0..=255u8 {
        for (device_function, info) in raiz.enumerate_bus(bus) {
            if info.vendor_id == VENDOR_VIRTIO && VIRTIO_BLK_IDS.contains(&info.device_id) {
                hallado = Some(device_function);
                break 'busqueda;
            }
        }
    }
    let device_function = hallado.ok_or("virtio-blk no hallado en el bus PCI")?;

    // 2. Habilitar E/S, espacio de memoria y BUS-MASTER. Sin bus-master el
    //    dispositivo no puede iniciar transferencias DMA por su cuenta.
    raiz.set_command(
        device_function,
        Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
    );

    // 3. Montar el transporte PCI moderno de virtio y el dispositivo de bloques.
    let transporte = PciTransport::new::<KernelHal, _>(&mut raiz, device_function)
        .map_err(|_| "no se pudo montar el transporte PCI de virtio")?;
    let mut disco = VirtIOBlk::<KernelHal, _>::new(transporte)
        .map_err(|_| "no se pudo inicializar el dispositivo virtio-blk")?;
    let capacidad = disco.capacity();

    // 4. Pedir al dispositivo que EMITA una interrupcion al completar cada
    //    peticion — el corazon de la E/S asincrona de esta fase.
    disco.enable_interrupts();
    DISCO.call_once(|| Mutex::new(Disco(disco)));

    // 5. Descubrir la linea de IRQ que el firmware asigno al dispositivo y
    //    enrutarla: registrar el manejador en la IDT y abrir la linea en el
    //    PIC. Las IRQ 0 y 1 son del temporizador y el teclado; un valor fuera
    //    de 2..15 (p. ej. 0xFF, «sin conexion») significa que no hay linea
    //    util — la E/S seguira funcionando, pero por sondeo.
    let irq = super::pci::linea_irq(device_function);
    if (2..=15).contains(&irq) {
        crate::interrupts::registrar_irq_disco(irq);
        crate::pic::desenmascarar(irq);
        IRQ_DISCO.store(irq, Ordering::SeqCst);
    }

    Ok(capacidad)
}

/// La linea de IRQ del disco, si el firmware enruto una util.
pub fn irq() -> Option<u8> {
    match IRQ_DISCO.load(Ordering::SeqCst) {
        0 => None,
        n => Some(n),
    }
}

/// Numero de interrupciones del disco atendidas desde el arranque.
pub fn pulsos_disco() -> u64 {
    PULSOS_DISCO.load(Ordering::Relaxed)
}

/// Punto de entrada DESDE el manejador de la IRQ del disco. Reconoce la
/// interrupcion en el dispositivo —leer su registro ISR baja la linea INTx— y
/// despierta a la tarea que aguardaba el bloque. Breve y libre de panicos:
/// corre en contexto de interrupcion.
pub fn atender_irq() {
    // Estamos en contexto de interrupcion (IF=0). Todo acceso normal a `DISCO`
    // toma su cerrojo con las interrupciones acalladas, de modo que aqui jamas
    // esta ocupado: tomarlo no puede interbloquear.
    if let Some(disco) = DISCO.get() {
        let _ = disco.lock().0.ack_interrupt();
    }
    PULSOS_DISCO.fetch_add(1, Ordering::Relaxed);
    if let Some(waker) = WAKER_DISCO.lock().take() {
        waker.wake();
    }
}

/// Inscribe el waker de la espera de disco en curso. Una sola ranura: las
/// operaciones de disco estan serializadas. El cerrojo lo disputa el manejador
/// de IRQ — tomarlo con las interrupciones acalladas hace imposible el bloqueo.
fn registrar_waker(waker: Waker) {
    interrupts::without_interrupts(|| *WAKER_DISCO.lock() = Some(waker));
}

// =============================================================================
//  EsperaDisco — UNA OPERACION DE BLOQUE COMO `Future`
// =============================================================================

/// Una transferencia de bloques en vuelo, expresada como `Future`. Posee sus
/// propios buferes DMA —el encabezado de peticion, la respuesta y los datos—,
/// que `virtio-drivers` mantiene prestados hasta que la operacion concluye.
///
/// Su `poll` envia la peticion la primera vez y, despues, comprueba el «used
/// ring»: si el dispositivo aun no ha terminado, inscribe el waker y cede; la
/// IRQ del disco lo reanudara. Una operacion a la vez — basta para renaser.
pub struct EsperaDisco {
    /// Encabezado de la peticion virtio. En el heap: direccion estable mientras
    /// el dispositivo lo tiene prestado, sin importar si el `Future` se mueve.
    req: Box<BlkReq>,
    /// Respuesta de estado del dispositivo. En el heap, por la misma razon.
    resp: Box<BlkResp>,
    /// Los datos: destino de una lectura, origen de una escritura.
    buf: Vec<u8>,
    /// Primer sector de la transferencia.
    sector: u64,
    /// `true` si la operacion escribe; `false` si lee.
    es_escritura: bool,
    /// Token que `virtio-drivers` devolvio al enviar la peticion. `None` hasta
    /// que el primer `poll` la envia.
    token: Option<u16>,
}

impl EsperaDisco {
    /// Hace avanzar la operacion: la envia si aun no lo estaba y comprueba si el
    /// dispositivo la completo. `None` => sigue en vuelo; `Some` => terminada.
    fn avanzar(&mut self) -> Option<Result<Vec<u8>, &'static str>> {
        // Todo el dialogo con el dispositivo, con las interrupciones acalladas:
        // asi la IRQ del disco jamas encuentra ocupado el cerrojo de `DISCO`.
        interrupts::without_interrupts(|| {
            let disco = match DISCO.get() {
                Some(disco) => disco,
                None => return Some(Err("disco no montado")),
            };
            let mut guardia = disco.lock();
            let blk = &mut guardia.0;

            // 1. Enviar la peticion la primera vez que se sondea esta espera.
            if self.token.is_none() {
                // SEGURIDAD: `req`, `buf` y `resp` viven en esta `EsperaDisco`,
                // que no se libera ni se accede por otra via hasta que el
                // `complete_*` de mas abajo cierre la operacion.
                let envio = unsafe {
                    if self.es_escritura {
                        blk.write_blocks_nb(
                            self.sector as usize,
                            &mut self.req,
                            self.buf.as_slice(),
                            &mut self.resp,
                        )
                    } else {
                        blk.read_blocks_nb(
                            self.sector as usize,
                            &mut self.req,
                            self.buf.as_mut_slice(),
                            &mut self.resp,
                        )
                    }
                };
                match envio {
                    Ok(token) => self.token = Some(token),
                    Err(_) => return Some(Err("no se pudo enviar la peticion al disco")),
                }
            }
            let token = self.token.unwrap();

            // 2. ¿Ha colocado el dispositivo este token en el «used ring»?
            if blk.peek_used() != Some(token) {
                return None; // aun en vuelo
            }

            // 3. Completado: recoger el resultado y desligar los buferes DMA.
            // SEGURIDAD: se pasan los MISMOS buferes que recibio el `*_nb`.
            let fin = unsafe {
                if self.es_escritura {
                    blk.complete_write_blocks(token, &self.req, self.buf.as_slice(), &mut self.resp)
                } else {
                    blk.complete_read_blocks(
                        token,
                        &self.req,
                        self.buf.as_mut_slice(),
                        &mut self.resp,
                    )
                }
            };
            Some(match fin {
                Ok(()) => Ok(core::mem::take(&mut self.buf)),
                Err(_) => Err("la operacion de disco no se completo"),
            })
        })
    }
}

impl Future for EsperaDisco {
    /// Al terminar, una lectura entrega sus datos; una escritura, un vector
    /// vacio. El error es siempre un mensaje estable.
    type Output = Result<Vec<u8>, &'static str>;

    fn poll(self: Pin<&mut Self>, contexto: &mut Context<'_>) -> Poll<Self::Output> {
        // `EsperaDisco` es `Unpin` —solo `Box`, `Vec` y escalares—: el `Pin`
        // no impone nada y el acceso mutable es directo.
        let this = self.get_mut();
        // Inscribir el waker ANTES de comprobar: si la IRQ se cuela entre la
        // comprobacion y la inscripcion, el waker ya esta puesto y el despertar
        // no se pierde — el mismo blindaje que usa `EsperaFrame` (ver `reloj`).
        registrar_waker(contexto.waker().clone());
        match this.avanzar() {
            Some(resultado) => Poll::Ready(resultado),
            None => Poll::Pending,
        }
    }
}

/// Prepara la LECTURA asincrona de `n_sectores` sectores desde `sector`. El
/// `Future` que devuelve no toca el disco hasta que se le sondea.
pub fn leer_bloques(sector: u64, n_sectores: usize) -> EsperaDisco {
    EsperaDisco {
        req: Box::new(BlkReq::default()),
        resp: Box::new(BlkResp::default()),
        buf: vec![0u8; n_sectores * TAM_SECTOR],
        sector,
        es_escritura: false,
        token: None,
    }
}

/// Prepara la ESCRITURA asincrona de `datos` a partir de `sector`. La longitud
/// de `datos` debe ser multiplo de un sector.
pub fn escribir_bloques(sector: u64, datos: Vec<u8>) -> EsperaDisco {
    EsperaDisco {
        req: Box::new(BlkReq::default()),
        resp: Box::new(BlkResp::default()),
        buf: datos,
        sector,
        es_escritura: true,
        token: None,
    }
}

// =============================================================================
//  bloquear_en — EL PUENTE PARA LOS CONTEXTOS SINCRONOS
// =============================================================================

/// Lleva un `Future` de disco hasta su final desde un contexto SINCRONO — el
/// arranque, o una capacidad WASM, que no pueden `.await`-ear—. Mientras el
/// disco trabaja:
///
///   * si las interrupciones estan activas, duerme la CPU con `hlt`; la
///     despertara la IRQ del disco —o el temporizador, como red de seguridad—;
///   * si no —en el arranque, antes de habilitarlas—, sondea en bucle.
///
/// Asi, una vez el sistema esta en marcha, la espera de disco JAMAS malgasta
/// ciclos en espera activa.
fn bloquear_en<F: Future>(futuro: F) -> F::Output {
    let mut futuro = core::pin::pin!(futuro);
    let waker = Waker::noop();
    let mut contexto = Context::from_waker(waker);
    loop {
        match futuro.as_mut().poll(&mut contexto) {
            Poll::Ready(salida) => return salida,
            Poll::Pending => {
                if interrupts::are_enabled() {
                    x86_64::instructions::hlt();
                } else {
                    core::hint::spin_loop();
                }
            }
        }
    }
}

/// Lee `buf.len() / 512` sectores consecutivos a partir de `sector`. Sincrono:
/// construido sobre la maquinaria asincrona via `bloquear_en`. El bufer debe
/// medir un multiplo entero de un sector.
pub fn leer_sectores(sector: u64, buf: &mut [u8]) -> Result<(), &'static str> {
    let datos = bloquear_en(leer_bloques(sector, buf.len() / TAM_SECTOR))?;
    buf.copy_from_slice(&datos);
    Ok(())
}

/// Escribe `buf.len() / 512` sectores consecutivos a partir de `sector`.
/// Sincrono, sobre la misma maquinaria asincrona.
pub fn escribir_sectores(sector: u64, buf: &[u8]) -> Result<(), &'static str> {
    bloquear_en(escribir_bloques(sector, buf.to_vec())).map(|_| ())
}
