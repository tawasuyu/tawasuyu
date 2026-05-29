// =============================================================================
//  renaser :: kernel/src/drivers/raton.rs — Fase 13 :: el raton PS/2
// -----------------------------------------------------------------------------
//  El raton cuelga del controlador 8042 —el mismo que sirve el teclado—, por su
//  puerto AUXILIAR, y anuncia cada movimiento por la IRQ12. Su lenguaje es un
//  paquete de tres bytes: banderas (botones + signos), delta X y delta Y.
//
//  Como el teclado en la Fase 8c, el raton respeta el GUARDARRAIL de
//  interrupciones: el manejador de IRQ12 solo toca atomicos —la posicion del
//  puntero— y una cola lock-free de eventos. Jamas un cerrojo cooperativo. El
//  compositor drena esa cola desde el reactor, a su ritmo.
// =============================================================================

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use crossbeam_queue::ArrayQueue;
use spin::Once;
use x86_64::instructions::port::Port;

use crate::pic;

/// Puerto de datos del controlador 8042 (compartido con el teclado).
const DATOS: u16 = 0x60;
/// Puerto de estado / comando del 8042.
const ESTADO: u16 = 0x64;

/// Capacidad de la cola de eventos del raton — holgada: nadie agita tanto.
const CAPACIDAD: usize = 128;

/// Un evento del raton, tal como lo entrega un paquete completo: la posicion
/// ya absoluta del puntero y el estado crudo de los botones (bit 0 izquierdo,
/// bit 1 derecho, bit 2 central). Lo produce la IRQ12; lo consume el compositor.
#[derive(Clone, Copy)]
pub struct EventoRaton {
    pub x: u16,
    pub y: u16,
    pub botones: u8,
}

/// Posicion del puntero, en pixeles. La escribe la IRQ12 a cada paquete; la lee
/// la consola para estampar el puntero, y el compositor para situar los clics.
static RATON_X: AtomicUsize = AtomicUsize::new(0);
static RATON_Y: AtomicUsize = AtomicUsize::new(0);

/// Limites de la pantalla — el puntero jamas se sale de ellos.
static ANCHO: AtomicUsize = AtomicUsize::new(0);
static ALTO: AtomicUsize = AtomicUsize::new(0);

/// ¿Esta el raton inicializado y vivo? Hasta entonces no hay puntero que pintar.
static ACTIVO: AtomicBool = AtomicBool::new(false);

/// Estado del ensamblado del paquete de 3 bytes. Lo toca SOLO la IRQ12, que es
/// no-reentrante en un solo nucleo: una secuencia de atomicos basta, sin cerrojo.
static FASE: AtomicUsize = AtomicUsize::new(0);
static BYTE0: AtomicU8 = AtomicU8::new(0);
static BYTE1: AtomicU8 = AtomicU8::new(0);
/// Estado de los botones en el paquete anterior — para detectar transiciones.
static BOTONES_ANTES: AtomicU8 = AtomicU8::new(0);

/// La cola de eventos: la IRQ12 deposita (lock-free, segura en interrupcion),
/// el compositor drena desde el reactor cooperativo.
static EVENTOS: Once<ArrayQueue<EventoRaton>> = Once::new();

// =============================================================================
//  Dialogo con el 8042 — esperas acotadas, sin colgar jamas el arranque
// =============================================================================

/// Espera, con un tope de intentos, a que el 8042 admita una escritura (su
/// bufer de entrada vacio). Devuelve `false` si se agota la paciencia.
fn esperar_envio() -> bool {
    for _ in 0..100_000 {
        // SEGURIDAD: 0x64 es el puerto de estado del 8042, fijo en el PC.
        let estado = unsafe { Port::<u8>::new(ESTADO).read() };
        if estado & 0b10 == 0 {
            return true;
        }
    }
    false
}

/// Espera, con un tope de intentos, a que el 8042 tenga un byte que entregar.
fn esperar_recepcion() -> bool {
    for _ in 0..100_000 {
        // SEGURIDAD: ver `esperar_envio`.
        let estado = unsafe { Port::<u8>::new(ESTADO).read() };
        if estado & 0b01 != 0 {
            return true;
        }
    }
    false
}

/// Escribe un byte en el puerto de comando del 8042.
fn comando_8042(byte: u8) {
    esperar_envio();
    // SEGURIDAD: 0x64 es el puerto de comando del 8042 en la arquitectura PC.
    unsafe { Port::<u8>::new(ESTADO).write(byte) };
}

/// Escribe un byte en el puerto de datos del 8042.
fn escribir_datos(byte: u8) {
    esperar_envio();
    // SEGURIDAD: 0x60 es el puerto de datos del 8042 en la arquitectura PC.
    unsafe { Port::<u8>::new(DATOS).write(byte) };
}

/// Lee un byte del puerto de datos del 8042.
fn leer_datos() -> u8 {
    esperar_recepcion();
    // SEGURIDAD: ver `escribir_datos`.
    unsafe { Port::<u8>::new(DATOS).read() }
}

/// Envia un comando AL raton (no al 8042): el prefijo 0xD4 le dice al 8042 que
/// el proximo byte de datos va al dispositivo auxiliar. Consume el ACK (0xFA).
fn comando_raton(byte: u8) {
    comando_8042(0xD4);
    escribir_datos(byte);
    let _ack = leer_datos();
}

/// Vacia el bufer de salida del 8042 — descarta bytes rezagados que pudieran
/// desincronizar el ensamblado del primer paquete.
fn vaciar() {
    for _ in 0..16 {
        // SEGURIDAD: ver `esperar_envio`.
        let estado = unsafe { Port::<u8>::new(ESTADO).read() };
        if estado & 0b01 == 0 {
            return;
        }
        let _ = unsafe { Port::<u8>::new(DATOS).read() };
    }
}

// =============================================================================
//  Arranque
// =============================================================================

/// Funda el raton: despierta el dispositivo auxiliar del 8042, programa su IRQ,
/// le ordena reportar movimiento y deja el puntero en el centro de la pantalla.
/// Requiere el heap activo; debe invocarse una vez, antes de habilitar las
/// interrupciones.
pub fn init(ancho: usize, alto: usize) {
    ANCHO.store(ancho, Ordering::Relaxed);
    ALTO.store(alto, Ordering::Relaxed);
    RATON_X.store(ancho / 2, Ordering::Relaxed);
    RATON_Y.store(alto / 2, Ordering::Relaxed);
    EVENTOS.call_once(|| ArrayQueue::new(CAPACIDAD));

    vaciar();

    // Despertar el dispositivo auxiliar (el raton) en el 8042.
    comando_8042(0xA8);

    // Leer el byte de configuracion, encender la IRQ del auxiliar (bit 1) y
    // asegurar que su reloj corre (bit 5 a cero), y reescribirlo.
    comando_8042(0x20);
    let mut config = leer_datos();
    config |= 0b0000_0010;
    config &= !0b0010_0000;
    comando_8042(0x60);
    escribir_datos(config);

    // Al raton: valores por defecto.
    comando_raton(0xF6);

    // Subir la TASA DE MUESTREO a 200 paquetes/s: `0xF3` (set sample rate)
    // seguido del dato `200`. El default del PS/2 son 100/s —10 ms entre
    // paquetes—; a 200/s son 5 ms, la mitad de latencia del puntero. Es la
    // diferencia entre un cursor a saltos y uno fluido EN METAL. En un firmware
    // que emula el i8042 sobre USB legacy el valor puede ignorarse o acotarse —
    // pedirlo no hace daño y, donde se honra, se nota.
    comando_raton(0xF3);
    comando_raton(200);

    // Y, por fin, reportar movimiento.
    comando_raton(0xF4);

    vaciar();
    ACTIVO.store(true, Ordering::Relaxed);
    // Levantar la mascara de la IRQ12 — el raton vive en el PIC esclavo.
    pic::desenmascarar(12);
}

// =============================================================================
//  El paquete de 3 bytes — punto de entrada desde la IRQ12
// =============================================================================

/// Punto de entrada DESDE el manejador de IRQ12. Ensambla el paquete de tres
/// bytes y, al completarlo, actualiza la posicion del puntero y encola un
/// evento. Deliberadamente breve y libre de panicos: corre en contexto de IRQ.
pub fn recibir_byte(byte: u8) {
    match FASE.load(Ordering::Relaxed) {
        0 => {
            // El primer byte SIEMPRE trae el bit 3 a 1. Si no, el flujo esta
            // desincronizado: se descarta el byte y se sigue esperando uno bueno.
            if byte & 0b0000_1000 == 0 {
                return;
            }
            BYTE0.store(byte, Ordering::Relaxed);
            FASE.store(1, Ordering::Relaxed);
        }
        1 => {
            BYTE1.store(byte, Ordering::Relaxed);
            FASE.store(2, Ordering::Relaxed);
        }
        _ => {
            FASE.store(0, Ordering::Relaxed);
            procesar(BYTE0.load(Ordering::Relaxed), BYTE1.load(Ordering::Relaxed), byte);
        }
    }
}

/// Procesa un paquete completo: traduce los deltas a una posicion absoluta del
/// puntero, acotada a la pantalla, y encola el evento si hay algo que el
/// compositor deba atender —un boton pulsado o un arrastre en curso—.
fn procesar(banderas: u8, dx_crudo: u8, dy_crudo: u8) {
    // Un paquete con desbordamiento de delta trae un salto disparatado: se
    // ignora su movimiento por completo.
    if banderas & 0b1100_0000 != 0 {
        return;
    }
    let dx = dx_crudo as i8 as i32;
    // El raton PS/2 da Y positivo hacia ARRIBA; la pantalla, hacia ABAJO.
    let dy = dy_crudo as i8 as i32;

    let ancho = ANCHO.load(Ordering::Relaxed) as i32;
    let alto = ALTO.load(Ordering::Relaxed) as i32;
    let x = (RATON_X.load(Ordering::Relaxed) as i32 + dx).clamp(0, (ancho - 1).max(0)) as usize;
    let y = (RATON_Y.load(Ordering::Relaxed) as i32 - dy).clamp(0, (alto - 1).max(0)) as usize;

    // El delta ya esta integrado a una posicion absoluta; comprometerla al
    // sumidero comun del puntero (botones en los 3 bits bajos de las banderas).
    comprometer(x, y, banderas & 0b0000_0111);
}

/// Compromete una posicion ABSOLUTA del puntero y su estado de botones al
/// estado compartido: acota a la pantalla, publica la posicion y encola un
/// evento SOLO si importa —los botones cambiaron, o alguno sigue pulsado (un
/// arrastre)—. El movimiento ocioso no satura la cola; el puntero, aun asi, ya
/// se movio (los atomicos de posicion).
///
/// Lo comparten dos origenes: el paquete PS/2 (`procesar`, que integra deltas)
/// y el driver de tableta virtio-input (`actualizar_desde_tableta`, que ya da
/// posicion absoluta). El compositor que drena los eventos no distingue cual
/// de los dos los produjo. Breve y libre de panicos: PS/2 lo llama en IRQ12.
fn comprometer(x: usize, y: usize, botones: u8) {
    let ancho = ANCHO.load(Ordering::Relaxed);
    let alto = ALTO.load(Ordering::Relaxed);
    let x = x.min(ancho.saturating_sub(1));
    let y = y.min(alto.saturating_sub(1));
    RATON_X.store(x, Ordering::Relaxed);
    RATON_Y.store(y, Ordering::Relaxed);

    let antes = BOTONES_ANTES.swap(botones, Ordering::Relaxed);
    if botones != antes || botones != 0 {
        if let Some(cola) = EVENTOS.get() {
            // Si la cola se desborda, el evento se pierde en silencio: mas vale
            // perder un gesto que arriesgar un panico dentro de una IRQ.
            let _ = cola.push(EventoRaton {
                x: x as u16,
                y: y as u16,
                botones,
            });
        }
    }

    // REDIBUJAR EL PUNTERO AQUI MISMO, sin esperar al proximo tic del compositor
    // (PIT 100 Hz = 10 ms). Llamado desde `procesar` corre en contexto de IRQ12,
    // al ritmo del sample rate del PS/2 (200 Hz = 5 ms): el cursor sigue al raton
    // en vez de saltar entre tics. `refrescar_puntero` es seguro en IRQ —usa
    // atomicos y `try_lock` sobre la consola, saliendo en silencio si esta
    // tomada; el proximo paquete reintenta—. Tambien lo llama la tableta
    // (`actualizar_desde_tableta`) desde el reactor, igual de seguro. Es la cura
    // del cursor a saltos en metal que los docstrings prometian sin cablear.
    crate::compositor::refrescar_puntero();
}

/// FASE 61 :: punto de entrada del driver de tableta virtio-input. Publica una
/// posicion ABSOLUTA del puntero —ya escalada a pixeles por el driver— y su
/// estado de botones (bit 0 izquierdo, 1 derecho, 2 central), reusando el
/// sumidero comun. Si el raton PS/2 nunca reporto, este es el unico origen del
/// puntero; si ambos viven, en la practica QEMU enruta el cursor del host a la
/// tableta (absoluta) y el PS/2 queda ocioso — no se pelean por la posicion.
pub fn actualizar_desde_tableta(x: usize, y: usize, botones: u8) {
    comprometer(x, y, botones);
}

// =============================================================================
//  Consulta — para la consola (puntero) y el compositor (eventos)
// =============================================================================

/// La posicion actual del puntero, o `None` si el raton aun no esta vivo. La
/// consulta la consola para estampar el puntero en cada volcado de pantalla.
pub fn posicion() -> Option<(usize, usize)> {
    if !ACTIVO.load(Ordering::Relaxed) {
        return None;
    }
    Some((
        RATON_X.load(Ordering::Relaxed),
        RATON_Y.load(Ordering::Relaxed),
    ))
}

/// Extrae el siguiente evento del raton, o `None` si no hay ninguno pendiente.
/// La drena el compositor, evento a evento, desde el reactor cooperativo.
pub fn siguiente_evento() -> Option<EventoRaton> {
    EVENTOS.get().and_then(ArrayQueue::pop)
}
