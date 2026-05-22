// =============================================================================
//  renaser :: async_system/waker.rs — Fase 3 :: el despertador de tareas
// -----------------------------------------------------------------------------
//  Un `Waker` es la promesa de que una tarea dormida volvera a ejecutarse. El
//  nuestro es minimo: al invocarse, reinyecta el `TaskId` de su tarea en la
//  cola de listas del ejecutor. Quien lo invoque —incluido un manejador de
//  IRQ— vuelve a poner esa tarea en circulacion.
// =============================================================================

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::task::Wake;
use core::task::Waker;

use spin::Mutex;

use super::task::TaskId;

/// La cola de tareas listas para avanzar, compartida entre el ejecutor y todos
/// los wakers que ha repartido.
pub type ColaListas = Arc<Mutex<VecDeque<TaskId>>>;

/// El despertador de UNA tarea concreta.
struct WakerTarea {
    id: TaskId,
    cola: ColaListas,
}

impl WakerTarea {
    /// Reinyecta la tarea en la cola de ejecucion.
    fn reinyectar(&self) {
        // El spinlock de la cola lo tocan tanto el hilo principal como los
        // manejadores de IRQ. Tomarlo con las interrupciones acalladas hace
        // IMPOSIBLE el interbloqueo: ninguna IRQ puede interrumpir al hilo
        // principal justo mientras este sostiene el cerrojo.
        x86_64::instructions::interrupts::without_interrupts(|| {
            self.cola.lock().push_back(self.id);
        });
    }
}

impl Wake for WakerTarea {
    fn wake(self: Arc<Self>) {
        self.reinyectar();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.reinyectar();
    }
}

/// Crea un `Waker` estandar que, al invocarse, reinyecta `id` en `cola`.
pub fn crear(id: TaskId, cola: ColaListas) -> Waker {
    Waker::from(Arc::new(WakerTarea { id, cola }))
}
