// =============================================================================
//  renaser :: kernel/src/drivers/consola_virtio.rs — Fase 49 :: virtio-console
// -----------------------------------------------------------------------------
//  La Fase 38 abrio el canal del firmador externo sobre el UART de COM1
//  (`0x3F8`) en polling — un teletipo de los anyos 70 envuelto en un FIFO
//  de 16 bytes. La Fase 49 corona el HAL de comunicaciones del kernel
//  bajo el estandar paravirtualizado de VirtIO: este driver expone la
//  MISMA API que `serial.rs` (escribir/drenar_input/leer_disponible/
//  vaciar_input) pero soportada por `virtio-drivers::device::console`
//  y enrutada por el bus PCI moderno —el mismo transporte que ya
//  gobierna a `virtio-blk-pci` (Fase 6) y `virtio-net-pci` (Fase 18).
//
//  ZERO POLLING CIEGO :: la lectura usa `VirtIOConsole::recv(pop=true)`
//  no bloqueante; el drenado a nuestro ring estatico ocurre en el
//  reactor cooperativo, no en una IRQ. La transmision usa `send_bytes`
//  sobre el transmit virtqueue — el host lo absorbe a la velocidad
//  del bus PCI, ordenes de magnitud mas rapido que 115200 baud.
//
//  ZERO-ALLOC EN EL CAMINO CALIENTE :: el ring de RX vive en un static
//  global tras un `spin::Mutex`. La constructora de `VirtIOConsole`
//  pide UN `Box<[u8; PAGE_SIZE]>` para su buffer interno via
//  `linked_list_allocator` — esa alocacion ocurre UNA SOLA VEZ en
//  `montar` y queda viva hasta el apagado. Ningun camino caliente
//  (escribir/leer) toca al asignador.
//
//  FALLBACK A SERIAL :: si el firmware no expone un virtio-console
//  (target sin `-device virtconsole`), `montar` devuelve `Err`. La
//  syscall `sys_cuaderno_solicitar_firma_host` detecta el estado
//  `desmontado` y cae limpiamente al camino UART de la Fase 38 — la
//  boot story se preserva sin interrumpir la cadena de firma.
// =============================================================================

use spin::{Mutex, Once};
use virtio_drivers::device::console::VirtIOConsole;
use virtio_drivers::transport::pci::PciTransport;
use virtio_drivers::transport::pci::bus::{Command, DeviceFunction, PciRoot};

use super::disco::KernelHal;
use super::pci::CamPuertos;

/// Vendor ID de VirtIO; Device IDs de consola (legacy 0x1003 + modern 0x1043).
const VENDOR_VIRTIO: u16 = 0x1AF4;
const VIRTIO_CONSOLE_IDS: [u16; 2] = [0x1003, 0x1043];

/// Capacidad del ring de input. Espeja a `serial::RX_RING_CAP` para que
/// el caller pueda intercambiar drivers sin re-tunear umbrales. 256 B
/// cubre los 65 bytes de respuesta de firma con holgura amplia y limita
/// el coste del drenado por tic.
const RX_RING_CAP: usize = 256;

struct RingRx {
    buf: [u8; RX_RING_CAP],
    /// Indice del proximo byte a LEER (consumido por la syscall).
    head: usize,
    /// Indice del proximo byte a ESCRIBIR (producido por `drenar_input`).
    tail: usize,
}

impl RingRx {
    const fn new() -> Self {
        RingRx {
            buf: [0; RX_RING_CAP],
            head: 0,
            tail: 0,
        }
    }

    /// Inserta `b` si hay espacio. Si el ring esta lleno, descarta — el
    /// mismo principio que en `serial.rs`: preferimos perder bytes
    /// viejos que bloquear el reactor en una rafaga adversaria.
    fn push(&mut self, b: u8) {
        let next_tail = (self.tail + 1) % RX_RING_CAP;
        if next_tail != self.head {
            self.buf[self.tail] = b;
            self.tail = next_tail;
        }
    }

    /// Extrae el proximo byte disponible. `None` si el ring esta vacio.
    fn pop(&mut self) -> Option<u8> {
        if self.head == self.tail {
            return None;
        }
        let b = self.buf[self.head];
        self.head = (self.head + 1) % RX_RING_CAP;
        Some(b)
    }
}

/// Ring lock-friendly de bytes RX. Mismo patron que `serial.rs`: ambos
/// caminos (drenar + leer) viven en el reactor cooperativo, asi que un
/// `spin::Mutex` basta — sin contienda con contexto IRQ.
static RX: Mutex<RingRx> = Mutex::new(RingRx::new());

/// El dispositivo virtio-console montado. Encierra punteros crudos a las
/// virtqueues y al MMIO; toda interaccion va serializada tras el `Mutex`
/// global. Una sola instancia se construye en `montar`.
struct Consola(VirtIOConsole<KernelHal, PciTransport>);

// SEGURIDAD: `Consola` envuelve `VirtIOConsole` cuyas virtqueues son
// internamente seguras de compartir bajo el `Mutex`. renaser es de un
// solo nucleo; los accesos cooperativos viven todos tras el cerrojo.
unsafe impl Send for Consola {}

/// La consola global. Se monta UNA SOLA VEZ en `montar`. `Once` da la
/// inicializacion idempotente: llamadas repetidas son no-op.
static CONSOLA: Once<Mutex<Consola>> = Once::new();

// =============================================================================
//  Montaje
// =============================================================================

/// Enumera el bus PCI, localiza el primer virtio-console (vendor 0x1AF4,
/// device 0x1003 o 0x1043) y monta su transporte moderno. Llamada UNA
/// sola vez al boot. Devuelve `Ok(())` si el dispositivo existe y se
/// inicializo correctamente; `Err` cuando QEMU no expone un
/// virtconsole — la syscall de firma cae entonces al UART de la Fase 38.
pub fn montar() -> Result<(), &'static str> {
    let mut raiz = PciRoot::new(CamPuertos);

    // 1. Localizar el primer virtio-console en el bus.
    let mut hallado: Option<DeviceFunction> = None;
    'busqueda: for bus in 0..=255u8 {
        for (device_function, info) in raiz.enumerate_bus(bus) {
            if info.vendor_id == VENDOR_VIRTIO && VIRTIO_CONSOLE_IDS.contains(&info.device_id) {
                hallado = Some(device_function);
                break 'busqueda;
            }
        }
    }
    let device_function = hallado.ok_or("virtio-console no hallado en el bus PCI")?;

    // 2. Habilitar E/S, MMIO y BUS-MASTER en la configuracion PCI.
    raiz.set_command(
        device_function,
        Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
    );

    // 3. Montar el transporte PCI moderno y construir el dispositivo.
    let transporte = PciTransport::new::<KernelHal, _>(&mut raiz, device_function)
        .map_err(|_| "no se pudo montar el transporte PCI de virtio-console")?;
    let consola = VirtIOConsole::<KernelHal, _>::new(transporte)
        .map_err(|_| "no se pudo inicializar el dispositivo virtio-console")?;

    CONSOLA.call_once(|| Mutex::new(Consola(consola)));
    Ok(())
}

/// `true` si el driver se monto correctamente. La syscall lo usa para
/// decidir si emite por VirtIO o cae al UART legacy.
#[inline]
pub fn montada() -> bool {
    CONSOLA.get().is_some()
}

// =============================================================================
//  E/S — API espejo de `drivers::serial`
// =============================================================================

/// Emite una secuencia de bytes por el transmit virtqueue. No bloquea
/// el kernel — el host absorbe a la velocidad del bus PCI. Si el
/// driver no esta montado, es no-op (la syscall ya filtro antes via
/// `montada()`; este chequeo defensivo evita un panico si alguien
/// llama sin filtrar).
pub fn escribir(bytes: &[u8]) {
    let Some(consola) = CONSOLA.get() else {
        return;
    };
    let mut c = consola.lock();
    let _ = c.0.send_bytes(bytes);
}

/// Drena el RX del dispositivo al ring interno. Cada llamada lee como
/// mucho `RX_RING_CAP` bytes — techo duro para no quedarse atascado si
/// una flood inunda la cola. Llamala antes de `leer_disponible` para
/// asegurar que tienes los bytes mas recientes que el host emitio.
pub fn drenar_input() {
    let Some(consola) = CONSOLA.get() else {
        return;
    };
    let mut c = consola.lock();
    let mut ring = RX.lock();
    for _ in 0..RX_RING_CAP {
        match c.0.recv(true) {
            Ok(Some(b)) => ring.push(b),
            Ok(None) => break,
            Err(_) => break,
        }
    }
}

/// Lee del ring interno hasta llenar `out` o agotar lo disponible.
/// Devuelve el numero de bytes copiados (0..=out.len()). NO bloquea —
/// el llamante decide que hacer con una lectura corta (reintentar,
/// devolver `Saturado`). Espejo exacto de `serial::leer_disponible`.
pub fn leer_disponible(out: &mut [u8]) -> usize {
    if CONSOLA.get().is_none() {
        return 0;
    }
    let mut ring = RX.lock();
    let mut n = 0;
    while n < out.len() {
        match ring.pop() {
            Some(b) => {
                out[n] = b;
                n += 1;
            }
            None => break,
        }
    }
    n
}

/// FASE 39/49 :: vacia el ring de RX. Llamala cuando arranca una nueva
/// solicitud (emision de prefijo) — descarta cualquier byte huerfano
/// que quedo de una solicitud anterior abortada. Espejo de
/// `serial::vaciar_input`.
pub fn vaciar_input() {
    // Drenar la cola del dispositivo al ring (por si hay bytes en
    // vuelo) y luego resetear el ring entero.
    drenar_input();
    let mut ring = RX.lock();
    ring.head = 0;
    ring.tail = 0;
}
