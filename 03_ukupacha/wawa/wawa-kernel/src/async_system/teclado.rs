// =============================================================================
//  renaser :: async_system/teclado.rs — el canal de scancodes del teclado
// -----------------------------------------------------------------------------
//  El manejador de IRQ1 es un mero PRODUCTOR: deposita cada scancode en colas
//  lock-free, seguras frente a interrupciones. Los consumidores —las apps WASM,
//  via la capacidad `sys_get_scancode`— las drenan sin bloquear.
//
//  FASE 5 :: con varias apps concurrentes, una sola cola compartida no sirve:
//  la primera en sondear le robaria la pulsacion a las demas. Por eso cada
//  aplicacion abre su PROPIO canal y la IRQ1 DIFUNDE cada scancode a todos —
//  cada app recibe su copia integra del flujo de entrada.
// =============================================================================

use alloc::sync::Arc;
use alloc::vec::Vec;

use crossbeam_queue::ArrayQueue;
use spin::{Mutex, Once};
use x86_64::instructions::interrupts;

/// Capacidad de la cola de scancodes de cada app. Holgada: nadie teclea tanto.
const CAPACIDAD_COLA: usize = 256;

/// Un canal de teclado: la cola lock-free de scancodes de UNA aplicacion.
pub type CanalTeclado = Arc<ArrayQueue<u8>>;

/// Censo de canales — uno por aplicacion del userspace. El manejador de IRQ1
/// difunde cada scancode a TODOS: asi cada app recibe su propia copia del
/// evento, sin que una le arrebate la pulsacion a otra.
static CANALES: Once<Mutex<Vec<CanalTeclado>>> = Once::new();

/// Funda el censo de canales del teclado. Requiere el heap ya activo; debe
/// invocarse una sola vez, antes de habilitar las interrupciones.
pub fn init() {
    CANALES.call_once(|| Mutex::new(Vec::new()));
}

/// Crea un canal de teclado nuevo, AUN sin inscribir en la difusion. Cada
/// aplicacion reclama el suyo al empezar a cargarse.
pub fn crear_canal() -> CanalTeclado {
    Arc::new(ArrayQueue::new(CAPACIDAD_COLA))
}

/// Inscribe un canal en el censo de difusion. Desde este instante, la IRQ1
/// empuja cada scancode tambien a este canal. Se invoca al final de la carga
/// de una app: una carga fallida no debe dejar canales huerfanos.
pub fn registrar_canal(canal: &CanalTeclado) {
    if let Some(censo) = CANALES.get() {
        // El cerrojo lo disputa el manejador de IRQ1: tomarlo con las
        // interrupciones acalladas hace imposible el interbloqueo.
        interrupts::without_interrupts(|| censo.lock().push(canal.clone()));
    }
}

/// Da de baja un canal del censo de difusion. Lo invoca el `Drop` de una
/// aplicacion desalojada: la IRQ1 deja, de inmediato, de empujarle scancodes.
pub fn cerrar_canal(canal: &CanalTeclado) {
    if let Some(censo) = CANALES.get() {
        interrupts::without_interrupts(|| {
            censo.lock().retain(|inscrito| !Arc::ptr_eq(inscrito, canal));
        });
    }
}

/// Punto de entrada DESDE el manejador de IRQ1. DIFUNDE el scancode a cuantos
/// canales haya abiertos. Deliberadamente breve y libre de panicos: corre en
/// contexto de interrupcion.
pub fn recibir_scancode(scancode: u8) {
    if let Some(censo) = CANALES.get() {
        for canal in censo.lock().iter() {
            // Si un canal desborda, se descarta el scancode en silencio: mas
            // vale perder una tecla que colapsar dentro de una interrupcion.
            let _ = canal.push(scancode);
        }
    }
}
