// =============================================================================
//  renaser :: async_system/puntero.rs — el canal de eventos del raton por app
// -----------------------------------------------------------------------------
//  Fase 23 :: la geometria de entrada como contexto inyectado. El compositor
//  drena la cola GLOBAL del raton (`drivers::raton::siguiente_evento`) y, ya
//  con los marcos teselados a la vista, traduce cada (x, y) absoluta a la
//  coordenada LOCAL del marco de la ventana enfocada. Si la coordenada cae
//  fuera del lienzo natural de la app, el evento se descarta en silencio: una
//  app jamas recibe un clic geometricamente ajeno a su perimetro.
//
//  Cada app abre su propio canal de eventos —un `ArrayQueue` lock-free—; el
//  censo se indexa por `indice_app` y el `enrutar` del compositor mete el
//  evento ya traducido en la ranura del foco. La capacidad `sys_puntero` del
//  userspace solo drena su propia cola: no hay forma de mirar la del vecino.
//
//  POSIX entrega coordenadas globales a cualquier proceso con permiso de
//  display, y ese permiso a su vez se filtra por X11/Wayland en una catarata
//  de chequeos contradictorios. Aqui la frontera es geometrica y SE COMPUTA
//  EN EL KERNEL: la app ni siquiera ve el numero absoluto.
// =============================================================================

use alloc::sync::Arc;
use alloc::vec::Vec;

use crossbeam_queue::ArrayQueue;
use spin::{Mutex, Once};
use x86_64::instructions::interrupts;

/// Capacidad de la cola de eventos del puntero de cada app. Holgada: un usuario
/// no produce mas de unos pocos clics por segundo, y un fotograma drena uno o
/// dos a lo sumo.
const CAPACIDAD_COLA: usize = 64;

/// Un evento del puntero TRADUCIDO al marco de la app que lo recibe. Las
/// coordenadas son LOCALES al lienzo natural: (0, 0) es la esquina superior
/// izquierda de la propia app, sin importar donde este la ventana en pantalla.
/// El campo `botones` viaja como en el paquete crudo: bit 0 izquierdo, bit 1
/// derecho, bit 2 central.
#[derive(Clone, Copy)]
pub struct EventoPuntero {
    pub local_x: u16,
    pub local_y: u16,
    pub botones: u8,
}

/// Un canal del puntero: la cola lock-free de eventos de UNA aplicacion.
pub type CanalPuntero = Arc<ArrayQueue<EventoPuntero>>;

/// Censo de canales, indexado por `indice_app`. Misma disciplina que el
/// censo del teclado: ranura `None` para apps sin canal o desalojadas.
static CANALES: Once<Mutex<Vec<Option<CanalPuntero>>>> = Once::new();

/// Funda el censo. Requiere heap activo; se invoca una sola vez, antes de
/// habilitar las interrupciones.
pub fn init() {
    CANALES.call_once(|| Mutex::new(Vec::new()));
}

/// Engendra un canal del puntero. La app lo reclama al cargarse; el
/// `ContextoCapacidades` se lo guarda y `sys_puntero` lo drena.
pub fn crear_canal() -> CanalPuntero {
    Arc::new(ArrayQueue::new(CAPACIDAD_COLA))
}

/// Inscribe el canal de la app `indice` en el censo. Desde aqui, el
/// enrutador del compositor sabe a quien entregarle un evento traducido.
pub fn registrar_canal(indice: usize, canal: &CanalPuntero) {
    if let Some(censo) = CANALES.get() {
        interrupts::without_interrupts(|| {
            let mut censo = censo.lock();
            while censo.len() <= indice {
                censo.push(None);
            }
            censo[indice] = Some(canal.clone());
        });
    }
}

/// Da de baja el canal de la app `indice`. Lo invoca el `Drop` de la app
/// desalojada: la ranura queda en `None` y el compositor deja de enrutarle
/// eventos.
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

/// Enruta un evento ABSOLUTO a la app de indice `indice`, traduciendolo al
/// marco que el compositor le dicta. El compositor llama aqui DESPUES de
/// haber atendido el clic/arrastre (foco + drag), de modo que el evento llega
/// a la app correcta y en sus propias coordenadas.
///
/// `marco_x`/`marco_y` son la esquina superior izquierda del marco asignado;
/// `nat_ancho`/`nat_alto`, el lienzo natural que la app pinta (centrado dentro
/// del marco por la composicion estandar del kernel). Solo se encola si el
/// punto absoluto cae DENTRO del lienzo natural: la app es ciega a clics que
/// caen sobre el cromo de su propia ventana o sobre otras ventanas.
pub fn enrutar(
    indice: usize,
    abs_x: usize,
    abs_y: usize,
    botones: u8,
    marco_x: usize,
    marco_y: usize,
    marco_ancho: usize,
    marco_alto: usize,
    nat_ancho: usize,
    nat_alto: usize,
) {
    // El compositor centra el lienzo natural dentro del marco; la traduccion
    // descuenta ambas: el origen del marco y el padding lateral del centrado.
    let off_x = marco_x + marco_ancho.saturating_sub(nat_ancho) / 2;
    let off_y = marco_y + marco_alto.saturating_sub(nat_alto) / 2;
    if abs_x < off_x || abs_y < off_y {
        return;
    }
    let local_x = abs_x - off_x;
    let local_y = abs_y - off_y;
    if local_x >= nat_ancho || local_y >= nat_alto {
        return;
    }

    if let Some(censo) = CANALES.get() {
        // Sin `without_interrupts`: este camino corre desde el reactor
        // cooperativo, no desde la IRQ. El cerrojo solo lo disputa el registro
        // de canales, brevemente al cargar/cerrar apps.
        if let Some(Some(canal)) = censo.lock().get(indice) {
            // Cola llena: descartar el evento mas viejo y sustituirlo. Una
            // app que no drena su cola perdera los eventos ANTIGUOS, no los
            // ultimos: mas util cuando se ponga al dia.
            let evento = EventoPuntero {
                local_x: local_x as u16,
                local_y: local_y as u16,
                botones,
            };
            if canal.push(evento).is_err() {
                let _ = canal.pop();
                let _ = canal.push(evento);
            }
        }
    }
}
