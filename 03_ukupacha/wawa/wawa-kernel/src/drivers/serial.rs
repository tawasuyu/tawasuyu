// =============================================================================
//  renaser :: kernel/src/drivers/serial.rs — Fase 38 :: el canal del firmador
// -----------------------------------------------------------------------------
//  Puerto serial COM1 (`0x3F8`) en modo POLLING, sin IRQ. El kernel lo usa para
//  emitir solicitudes de firma al operador externo (`wawactl`) y leer la firma
//  Ed25519 que viene de vuelta — sin filtrar nada de su estado interno al canal.
//
//  La politica de la Fase 25 (el kernel solo VERIFICA, jamas FIRMA) se respeta
//  estrictamente: la clave privada vive del lado del host, en `wawactl` o un
//  HSM futuro. Este driver es el cordon umbilical limpio entre el Ring 0 de
//  Wawa y el operador externo — todo lo que cruza es el hash a firmar (en
//  ASCII hexadecimal, prefijado por una etiqueta de control inconfundible) y
//  los 64 bytes crudos de la firma de retorno.
//
//  ZERO-ALLOC: el formateo del prefijo + hex del hash vive en un buffer en
//  pila estatica de tamaño fijo; el ring de input es un array global de
//  256 B. Ningun camino toca al `linked_list_allocator`.
//
//  ZERO-IRQ: usamos polling del LSR (Line Status Register) en cada lectura y
//  escritura. Es simple, no requiere registrar handlers ni mezcla con la
//  matriz de interrupciones del PIC. En cargas reales la velocidad de COM1
//  (115200 baud) basta para tramas de 64 bytes en milisegundos.
// =============================================================================

use spin::Mutex;
use x86_64::instructions::port::Port;

// --- Puertos del UART 16550 estandar en COM1. -----------------------------
const COM1_BASE: u16 = 0x3F8;
const PORT_DATA: u16 = COM1_BASE;
const PORT_INT_EN: u16 = COM1_BASE + 1;
const PORT_FIFO_CTL: u16 = COM1_BASE + 2;
const PORT_LINE_CTL: u16 = COM1_BASE + 3;
const PORT_MODEM_CTL: u16 = COM1_BASE + 4;
const PORT_LINE_STATUS: u16 = COM1_BASE + 5;

// Bits del LSR utiles en polling.
const LSR_DATA_READY: u8 = 1 << 0;
const LSR_THR_EMPTY: u8 = 1 << 5;

/// Capacidad del ring de input. 256 bytes cubre los 64 bytes de firma con
/// holgura amplia y limita el coste del drenado por tic.
const RX_RING_CAP: usize = 256;

struct RingRx {
    buf: [u8; RX_RING_CAP],
    /// Indice del proximo byte a LEER (consumido por la syscall).
    head: usize,
    /// Indice del proximo byte a ESCRIBIR (producido por el drenado de UART).
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

    /// Inserta `b` si hay espacio. Si el ring esta lleno, descarta — preferimos
    /// perder bytes viejos que bloquear el reactor en una rafaga adversaria.
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

/// Ring lock-friendly de bytes RX. Se accede desde el camino de syscall y desde
/// `drenar_input` — un `spin::Mutex` basta porque ambos caminos viven en el
/// reactor cooperativo (no en contexto IRQ).
static RX: Mutex<RingRx> = Mutex::new(RingRx::new());

/// Inicializa COM1 a 115200 baud, 8N1, polling. Llamada una sola vez al boot.
pub fn init() {
    unsafe {
        // 1. Apagar interrupciones del UART — usamos polling.
        Port::<u8>::new(PORT_INT_EN).write(0x00);
        // 2. Habilitar DLAB para programar el divisor de baudios.
        Port::<u8>::new(PORT_LINE_CTL).write(0x80);
        // Divisor = 1 -> 115200 baud (DLL=1, DLM=0).
        Port::<u8>::new(PORT_DATA).write(0x01);
        Port::<u8>::new(PORT_INT_EN).write(0x00);
        // 3. 8 bits, sin paridad, 1 bit de stop; DLAB = 0.
        Port::<u8>::new(PORT_LINE_CTL).write(0x03);
        // 4. FIFO habilitado, umbral 14 bytes; limpiar TX/RX FIFOs.
        Port::<u8>::new(PORT_FIFO_CTL).write(0xC7);
        // 5. Modem control: DTR + RTS + OUT2 (necesario para algunas
        //    configuraciones de QEMU; OUT2 habilita IRQs si las prendieras).
        Port::<u8>::new(PORT_MODEM_CTL).write(0x0B);
    }
}

#[inline]
fn lsr() -> u8 {
    unsafe { Port::<u8>::new(PORT_LINE_STATUS).read() }
}

/// Espera (spin) hasta que el transmit holding register este vacio y emite
/// `b`. Bloquea al kernel; el coste por byte a 115200 baud son ~87us.
pub fn poner(b: u8) {
    while lsr() & LSR_THR_EMPTY == 0 {}
    unsafe { Port::<u8>::new(PORT_DATA).write(b) }
}

/// Emite una secuencia de bytes por COM1, en orden estricto.
pub fn escribir(bytes: &[u8]) {
    for &b in bytes {
        poner(b);
    }
}

/// Drena el RX UART al ring interno. Cada llamada lee como mucho `RX_RING_CAP`
/// bytes — un techo duro para no quedarse atascado si una flood inunda el
/// puerto. Llamala antes de leer del ring para asegurar que tienes los bytes
/// mas recientes que llegaron del host.
pub fn drenar_input() {
    let mut ring = RX.lock();
    for _ in 0..RX_RING_CAP {
        if lsr() & LSR_DATA_READY == 0 {
            break;
        }
        let b = unsafe { Port::<u8>::new(PORT_DATA).read() };
        ring.push(b);
    }
}

/// Lee del ring interno hasta llenar `out` o agotar lo disponible. Devuelve
/// el numero de bytes copiados (0..=out.len()). NO bloquea — el llamante
/// decide que hacer con una lectura corta (reintentar, devolver `Saturado`).
pub fn leer_disponible(out: &mut [u8]) -> usize {
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

/// FASE 39 :: vacia el ring de RX Y purga el FIFO del UART. Llamala cuando
/// arranca una nueva solicitud (emision de prefijo) — asi descartamos
/// cualquier byte huerfano que quedo de una solicitud anterior abortada o
/// de basura en el canal antes del demonio. Tras esta llamada, el primer
/// byte que entre por COM1 sera el primero que vea el llamante.
pub fn vaciar_input() {
    // Drenar el UART al ring (por si hay bytes pendientes en el FIFO).
    drenar_input();
    // Resetear el ring entero — head = tail = 0 descarta todo el contenido
    // sin recorrerlo byte a byte.
    let mut ring = RX.lock();
    ring.head = 0;
    ring.tail = 0;
}
