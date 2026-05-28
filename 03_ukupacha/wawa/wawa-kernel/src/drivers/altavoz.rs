// =============================================================================
//  renaser :: kernel/src/drivers/altavoz.rs — Fase 12 :: la bocina del PC
// -----------------------------------------------------------------------------
//  La bocina del PC es el instrumento mas humilde del hardware: un solo bit que,
//  conmutado a la frecuencia justa, hace vibrar una membrana. No hay PCM, ni
//  DMA, ni mezclador — solo el canal 2 del PIT generando una onda cuadrada y
//  una compuerta en el puerto 0x61 que la deja pasar al altavoz, o no.
//
//  El canal 0 del PIT es el latido del kernel (ver `pic`); el canal 2 es de la
//  bocina y de nadie mas — programarlo aqui no perturba el temporizador—. Esta
//  es la unica via del kernel hacia el sonido; la capacidad `sys_tono` la
//  ofrece al userspace, gobernada por el foco del compositor.
// =============================================================================

use core::sync::atomic::{AtomicU64, Ordering};

use alloc::collections::VecDeque;
use spin::Mutex;
use x86_64::instructions::port::Port;

/// Frecuencia del cristal del PIT, en Hz — el divisor se calcula contra ella.
const PIT_BASE_HZ: u32 = 1_193_182;

/// Puerto de comando del PIT.
const PIT_COMANDO: u16 = 0x43;
/// Puerto de datos del canal 2 del PIT — el de la bocina.
const PIT_CANAL2: u16 = 0x42;
/// Puerto de control de la bocina (bits 0 y 1: compuerta del PIT y dato).
const CONTROL_BOCINA: u16 = 0x61;

/// ¿Hay un virtio-sound montado que deba recibir el audio en vez de la bocina
/// del PIT? Cuando lo hay, `tono`/`agendar`/`atender` enrutan hacia `sonido`
/// (Fase 62) y la bocina queda en reposo; cuando no, la bocina es la voz.
fn usar_virtio() -> bool {
    crate::drivers::sonido::disponible()
}

/// Pone la bocina a sonar a `frecuencia_hz`. Un `0` —o una frecuencia que un
/// divisor de 16 bits no pueda representar (por debajo de ~19 Hz)— la SILENCIA.
/// Es la unica via del kernel hacia el sonido.
///
/// FASE 62 :: si hay virtio-sound, el tono se reproduce como PCM real (un tono
/// SOSTENIDO de app, via `sys_tono`); la bocina del PIT no se toca.
pub fn tono(frecuencia_hz: u32) {
    if usar_virtio() {
        crate::drivers::sonido::fijar_tono_app(frecuencia_hz);
        return;
    }
    if frecuencia_hz == 0 || PIT_BASE_HZ / frecuencia_hz > 0xFFFF {
        silenciar();
        return;
    }
    // El divisor cabe en 16 bits; un `.max(1)` lo protege de una frecuencia
    // disparatadamente alta que lo dejara en cero.
    let divisor = (PIT_BASE_HZ / frecuencia_hz).max(1) as u16;

    // SEGURIDAD: 0x43 y 0x42 son los puertos del PIT en la arquitectura PC;
    // 0xB6 selecciona el canal 2, acceso lobyte+hibyte, modo 3 (onda cuadrada).
    // El canal 2 es exclusivo de la bocina: no perturba el latido del kernel.
    unsafe {
        let mut comando = Port::<u8>::new(PIT_COMANDO);
        let mut canal2 = Port::<u8>::new(PIT_CANAL2);
        comando.write(0xB6u8);
        canal2.write((divisor & 0xFF) as u8);
        canal2.write((divisor >> 8) as u8);
    }
    abrir_compuerta();
}

/// Abre la compuerta del puerto 0x61: deja pasar la onda del canal 2 al altavoz.
fn abrir_compuerta() {
    // SEGURIDAD: 0x61 es el puerto de control de la bocina; sus bits 0 y 1
    // —compuerta del PIT y dato del altavoz— se tocan con leer-modificar-
    // escribir para no perturbar los demas bits del chipset.
    unsafe {
        let mut control = Port::<u8>::new(CONTROL_BOCINA);
        let estado = control.read();
        control.write(estado | 0b11);
    }
}

/// Silencia la bocina: cierra la compuerta del puerto 0x61. La onda del canal 2
/// sigue generandose, pero ya no alcanza la membrana.
fn silenciar() {
    // SEGURIDAD: ver `abrir_compuerta`. Limpiar los bits 0 y 1 corta el sonido.
    unsafe {
        let mut control = Port::<u8>::new(CONTROL_BOCINA);
        let estado = control.read();
        control.write(estado & !0b11);
    }
}

// =============================================================================
//  SECUENCIAS DEL KERNEL — la voz propia del sistema (Fase 15)
// -----------------------------------------------------------------------------
//  La bocina es de la ventana enfocada (Fase 12), pero el kernel tambien
//  necesita hablar: un acorde al arrancar, un repique al lanzar una app, un
//  bajo al desalojarla. Una cola de notas pendientes —`(frecuencia, ms)`— y
//  un reloj de fin —`FIN_NOTA`— que la tarea del compositor consulta cada
//  fotograma: si la nota actual ya termino, pasa a la siguiente. Mientras el
//  kernel suena, las llamadas de los apps a `sys_tono` se ignoran — el
//  kernel manda en su propia voz.
// =============================================================================

/// La cola de notas pendientes — `(frecuencia_hz, duracion_ms)`. Solo la
/// tocan tareas cooperativas: agendar (desde los hitos del kernel) y atender
/// (desde la tarea del compositor). Ninguna IRQ se la disputa.
static SECUENCIA: Mutex<VecDeque<(u32, u32)>> = Mutex::new(VecDeque::new());

/// Milisegundo (lectura del reloj monotono) en que la nota actual acaba. Lo
/// consulta `kernel_sonando` para gatear a las apps.
static FIN_NOTA: AtomicU64 = AtomicU64::new(0);

/// Agenda una secuencia de notas: cada `(frecuencia_hz, duracion_ms)` se hara
/// sonar en orden. Un `frecuencia_hz=0` es una pausa silenciosa. Si ya habia
/// una secuencia sonando, las nuevas notas se encolan al final.
pub fn agendar(secuencia: &[(u32, u32)]) {
    // FASE 62 :: con virtio-sound, la voz del kernel suena como PCM real;
    // delegamos la secuencia a `sonido`, que la mezcla con prioridad.
    if usar_virtio() {
        crate::drivers::sonido::agendar(secuencia);
        return;
    }
    let mut cola = SECUENCIA.lock();
    for &(frec, dur) in secuencia {
        cola.push_back((frec, dur));
    }
}

/// ¿Esta el kernel sonando una nota suya? Mientras dure, las llamadas de los
/// apps a `sys_tono` quedan silenciadas — el kernel no se interrumpe a si
/// mismo.
pub fn kernel_sonando() -> bool {
    crate::async_system::reloj::milisegundos() < FIN_NOTA.load(Ordering::Relaxed)
}

/// Atiende el reloj de la secuencia: si la nota actual ya termino, saca la
/// siguiente de la cola y la hace sonar; si la cola esta vacia, calla la
/// bocina. La invoca la tarea del compositor cada fotograma.
pub fn atender() {
    // FASE 62 :: con virtio-sound, la reproduccion la conduce la tarea de
    // sonido (`sonido::bombear`), no esta cadencia de bocina. No-op aqui.
    if usar_virtio() {
        return;
    }
    let ahora = crate::async_system::reloj::milisegundos();
    if ahora < FIN_NOTA.load(Ordering::Relaxed) {
        return; // la nota actual sigue sonando
    }
    let siguiente = SECUENCIA.lock().pop_front();
    match siguiente {
        Some((frec, dur)) => {
            tono(frec);
            FIN_NOTA.store(ahora + dur as u64, Ordering::Relaxed);
        }
        None => {
            // Sin notas que sonar: silenciar. Las apps recuperaran la bocina
            // en cuanto su proxima llamada a `sys_tono` vea `kernel_sonando`
            // ya en `false`.
            tono(0);
        }
    }
}

// =============================================================================
//  CATALOGO DE VOCES — los hitos del sistema y su sonido
// =============================================================================

/// Acorde de bienvenida: Do — Mi — Sol del Do mayor. Suena una vez, al
/// completarse el arranque del kernel.
pub const VOZ_BIENVENIDA: [(u32, u32); 3] = [(523, 130), (659, 130), (784, 240)];

/// Llamada al lanzar una app NUEVA en vivo: dos notas ascendentes.
pub const VOZ_LANZAR: [(u32, u32); 2] = [(700, 70), (1050, 90)];

/// Llamada al cerrar una app LIMPIAMENTE (`Alt+Q`): dos notas descendentes.
pub const VOZ_CERRAR: [(u32, u32); 2] = [(900, 70), (520, 100)];

/// Llamada al DESALOJAR una app por falla: un bajo de aviso.
pub const VOZ_DESALOJO: [(u32, u32); 1] = [(180, 260)];
