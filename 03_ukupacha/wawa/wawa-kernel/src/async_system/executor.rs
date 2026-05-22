// =============================================================================
//  renaser :: async_system/executor.rs — Fase 3 :: el reactor cooperativo
// -----------------------------------------------------------------------------
//  El ejecutor es el corazon reactivo de renaser. Mantiene el censo de tareas
//  vivas y una cola de las que estan listas para avanzar. Cuando no queda nada
//  por hacer, no malgasta la CPU en un bucle ocupado: la duerme con `hlt`
//  hasta que el proximo impulso de hardware la despierte.
// =============================================================================

use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use core::future::Future;
use core::task::{Context, Poll, Waker};

use spin::Mutex;
use x86_64::instructions::interrupts;

use super::task::{Task, TaskId};
use super::waker::{self, ColaListas};

/// El ejecutor cooperativo de renaser.
pub struct Executor {
    /// Censo de todas las tareas vivas, indexadas por su identidad.
    tareas: BTreeMap<TaskId, Task>,
    /// Cola de tareas listas para su proximo avance.
    cola_listas: ColaListas,
    /// Cache de wakers: uno por tarea, reutilizado entre avances.
    cache_wakers: BTreeMap<TaskId, Waker>,
}

impl Executor {
    /// Crea un ejecutor vacio. Requiere que el heap ya este fundado.
    pub fn nuevo() -> Executor {
        Executor {
            tareas: BTreeMap::new(),
            cola_listas: Arc::new(Mutex::new(VecDeque::new())),
            cache_wakers: BTreeMap::new(),
        }
    }

    /// Da de alta una tarea nueva y la marca como lista para su primer avance.
    pub fn spawn(&mut self, futuro: impl Future<Output = ()> + Send + 'static) {
        let tarea = Task::nueva(futuro);
        let id = tarea.id;
        if self.tareas.insert(id, tarea).is_some() {
            panic!("renaser :: TaskId duplicado — lo imposible ha ocurrido");
        }
        interrupts::without_interrupts(|| self.cola_listas.lock().push_back(id));
    }

    /// Avanza, hasta agotarla, la cola de tareas listas.
    fn avanzar_listas(&mut self) {
        loop {
            // Sacar el siguiente id con las interrupciones acalladas: la cola
            // es compartida con los wakers que se disparan desde las IRQ.
            let siguiente = interrupts::without_interrupts(|| self.cola_listas.lock().pop_front());
            let id = match siguiente {
                Some(id) => id,
                None => return,
            };
            let tarea = match self.tareas.get_mut(&id) {
                Some(t) => t,
                None => continue, // la tarea concluyo en una vuelta previa
            };
            // Un waker por tarea, reutilizado: reinyecta esta tarea en la cola.
            let cola = self.cola_listas.clone();
            let despertador = self
                .cache_wakers
                .entry(id)
                .or_insert_with(|| waker::crear(id, cola));
            let mut contexto = Context::from_waker(despertador);
            if let Poll::Ready(()) = tarea.poll(&mut contexto) {
                // La tarea termino: se retira del censo y se libera su waker.
                self.tareas.remove(&id);
                self.cache_wakers.remove(&id);
            }
        }
    }

    /// Si no queda trabajo, duerme la CPU hasta la proxima interrupcion. El
    /// chequeo y el `hlt` se hacen con las interrupciones acalladas para que
    /// ningun despertar se pierda en la rendija entre uno y otro.
    fn dormir_si_inactivo(&self) {
        interrupts::disable();
        let hay_trabajo = !self.cola_listas.lock().is_empty();
        if hay_trabajo {
            // Llego trabajo justo ahora: reactivar y seguir sin dormir.
            interrupts::enable();
        } else {
            // `sti; hlt` atomico: habilita y duerme sin condicion de carrera.
            interrupts::enable_and_hlt();
        }
    }

    /// Cede el hilo principal al reactor. No retorna jamas: desde aqui, renaser
    /// vive del latir de sus interrupciones.
    pub fn run(&mut self) -> ! {
        loop {
            self.avanzar_listas();
            self.dormir_si_inactivo();
        }
    }
}
