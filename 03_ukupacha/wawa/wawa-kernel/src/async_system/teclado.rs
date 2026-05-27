// =============================================================================
//  renaser :: async_system/teclado.rs — el canal de scancodes del teclado
// -----------------------------------------------------------------------------
//  El manejador de IRQ1 es el PRODUCTOR: deposita cada scancode en colas
//  lock-free, seguras frente a interrupciones. Los consumidores —las apps WASM,
//  via la capacidad `sys_get_scancode`— las drenan sin bloquear.
//
//  FASE 5 :: cada app abre su PROPIO canal; la primera en sondear no le roba la
//  pulsacion a las demas.
//
//  FASE 8c :: el teclado deja de DIFUNDIR a ciegas. Ahora discrimina:
//
//    * La tecla Alt es el MODIFICADOR del sistema. Con Alt pulsada, los make
//      codes son MANDOS del compositor (ciclar el teselado, mover el foco,
//      promover, reordenar y hacer flotar ventanas, cerrar y lanzar apps): se
//      consumen aqui, jamas llegan a una app.
//    * Una tecla ordinaria se entrega SOLO a la app ENFOCADA — la que el
//      compositor senala. El censo de canales se indexa por el `indice_app`,
//      de modo que el foco —un atomico— elija el canal exacto.
//
//  Todo esto corre en contexto de IRQ y NO bloquea ningun cerrojo cooperativo:
//  el modificador es un atomico, los mandos van a una cola lock-free.
// =============================================================================

use core::sync::atomic::{AtomicBool, Ordering};

use alloc::sync::Arc;
use alloc::vec::Vec;

use crossbeam_queue::ArrayQueue;
use spin::{Mutex, Once};
use x86_64::instructions::interrupts;

use crate::compositor::{self, Mando};

/// Capacidad de la cola de scancodes de cada app. Holgada: nadie teclea tanto.
const CAPACIDAD_COLA: usize = 256;

// --- Scancodes del set 1 que el teclado interpreta como mandos del sistema. ---
/// Alt izquierda — make (pulsada) y break (soltada).
const ALT_MAKE: u8 = 0x38;
const ALT_BREAK: u8 = 0xB8;
/// Barra espaciadora — `Alt + Espacio` cicla el modo de teselado.
const ESPACIO: u8 = 0x39;
/// Tecla J — `Alt + J` mueve el foco a la ventana siguiente.
const TECLA_J: u8 = 0x24;
/// Tecla K — `Alt + K` mueve el foco a la ventana anterior.
const TECLA_K: u8 = 0x25;
/// Tecla H — `Alt + H` mueve la ventana enfocada atras en el orden.
const TECLA_H: u8 = 0x23;
/// Tecla L — `Alt + L` mueve la ventana enfocada adelante en el orden.
const TECLA_L: u8 = 0x26;
/// Tecla Enter — `Alt + Enter` promueve la ventana enfocada a maestra.
const ENTER: u8 = 0x1C;
/// Tecla F — `Alt + F` alterna la ventana enfocada entre teselada y flotante.
const TECLA_F: u8 = 0x21;
/// Tecla Q — `Alt + Q` cierra la aplicacion enfocada (baja en vivo).
const TECLA_Q: u8 = 0x10;
/// Tecla N — `Alt + N` lanza una aplicacion nueva (alta en vivo, rotativa).
const TECLA_N: u8 = 0x31;
/// Tecla G — `Alt + G` fuerza una pasada del compactador del grafo (Fase 57).
const TECLA_G: u8 = 0x22;
/// Tecla P — `Alt + P` abre/cierra el launcher grafico (Fase 58).
const TECLA_P: u8 = 0x19;

/// Un canal de teclado: la cola lock-free de scancodes de UNA aplicacion.
pub type CanalTeclado = Arc<ArrayQueue<u8>>;

/// Censo de canales, INDEXADO por el `indice_app` de cada aplicacion. Una
/// ranura `None` es una app que no abrio canal o que fue desalojada. El
/// indexado estable permite que el foco —un simple indice— elija el canal.
static CANALES: Once<Mutex<Vec<Option<CanalTeclado>>>> = Once::new();

/// ¿Esta la tecla Alt pulsada? El modificador de los mandos del sistema. Lo
/// escribe y lo lee SOLO el manejador de IRQ1 — un atomico, sin cerrojo.
static ALT_PULSADO: AtomicBool = AtomicBool::new(false);

/// Funda el censo de canales del teclado. Requiere el heap ya activo; debe
/// invocarse una sola vez, antes de habilitar las interrupciones.
pub fn init() {
    CANALES.call_once(|| Mutex::new(Vec::new()));
}

/// Crea un canal de teclado nuevo, AUN sin inscribir en el censo. Cada
/// aplicacion reclama el suyo al empezar a cargarse.
pub fn crear_canal() -> CanalTeclado {
    Arc::new(ArrayQueue::new(CAPACIDAD_COLA))
}

/// Inscribe el canal de la app `indice` en el censo. Desde este instante, una
/// tecla ordinaria llega a esta app cuando tiene el foco. Se invoca al final de
/// la carga de una app: una carga fallida no debe dejar canales huerfanos.
pub fn registrar_canal(indice: usize, canal: &CanalTeclado) {
    if let Some(censo) = CANALES.get() {
        // El cerrojo lo disputa el manejador de IRQ1: tomarlo con las
        // interrupciones acalladas hace imposible el interbloqueo.
        interrupts::without_interrupts(|| {
            let mut censo = censo.lock();
            while censo.len() <= indice {
                censo.push(None);
            }
            censo[indice] = Some(canal.clone());
        });
    }
}

/// Da de baja el canal de la app `indice`. Lo invoca el `Drop` de una
/// aplicacion desalojada: la ranura queda en `None` y la IRQ deja de enrutarle
/// teclas, sin desplazar los indices de las demas.
pub fn cerrar_canal(indice: usize) {
    if let Some(censo) = CANALES.get() {
        interrupts::without_interrupts(|| {
            let mut censo = censo.lock();
            if let Some(ranura) = censo.get_mut(indice) {
                *ranura = None;
            }
        });
    }
}

/// Punto de entrada DESDE el manejador de IRQ1. Rastrea el modificador Alt,
/// intercepta los mandos del sistema y enruta la tecla ordinaria a la app
/// enfocada. Deliberadamente breve y libre de panicos: corre en contexto de
/// interrupcion y no bloquea ningun cerrojo cooperativo.
pub fn recibir_scancode(scancode: u8) {
    // 1. Rastrear la tecla Alt — el modificador de los mandos del sistema. Se
    //    consume: el modificador nunca se difunde a una app.
    match scancode {
        ALT_MAKE => {
            ALT_PULSADO.store(true, Ordering::Relaxed);
            return;
        }
        ALT_BREAK => {
            ALT_PULSADO.store(false, Ordering::Relaxed);
            return;
        }
        _ => {}
    }

    // 2. Con Alt pulsada, los make codes son MANDOS del compositor. Se traducen
    //    a una orden en la cola lock-free y se consumen — jamas llegan a una app.
    if ALT_PULSADO.load(Ordering::Relaxed) {
        match scancode {
            ESPACIO => compositor::solicitar(Mando::CiclarLayout),
            TECLA_J => compositor::solicitar(Mando::FocoSiguiente),
            TECLA_K => compositor::solicitar(Mando::FocoAnterior),
            ENTER => compositor::solicitar(Mando::Promover),
            TECLA_L => compositor::solicitar(Mando::MoverAdelante),
            TECLA_H => compositor::solicitar(Mando::MoverAtras),
            TECLA_F => compositor::solicitar(Mando::Flotar),
            TECLA_Q => compositor::solicitar(Mando::Cerrar),
            TECLA_N => compositor::solicitar(Mando::Lanzar),
            TECLA_G => compositor::solicitar(Mando::CompactarGrafo),
            TECLA_P => compositor::solicitar(Mando::ToggleLauncher),
            _ => {}
        }
        return;
    }

    // 3. Tecla ordinaria: se entrega SOLO a la app que tiene el foco. El foco
    //    es un indice atomico; el censo, un vector indexado por `indice_app`.
    if let Some(censo) = CANALES.get() {
        if let Some(Some(canal)) = censo.lock().get(compositor::foco()) {
            // Si el canal desborda, se descarta el scancode en silencio: mas
            // vale perder una tecla que colapsar dentro de una interrupcion.
            let _ = canal.push(scancode);
        }
    }
}
