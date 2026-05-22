// =============================================================================
//  renaser :: async_system/executor.rs — Fase 3 :: el reactor cooperativo
// -----------------------------------------------------------------------------
//  El ejecutor es el corazon reactivo de renaser. Mantiene el censo de tareas
//  vivas y una cola de las que estan listas para avanzar. Cuando no queda nada
//  por hacer, no malgasta la CPU en un bucle ocupado: la duerme con `hlt`
//  hasta que el proximo impulso de hardware la despierte.
//
//  FASE 10 :: el censo deja de ser inmutable tras el arranque. Una cola de
//  NACIMIENTOS permite engendrar tareas EN VIVO —con el reactor ya en marcha—:
//  el orquestador deposita un futuro y el ejecutor lo adopta en su proxima
//  vuelta. Asi el userspace puede crecer despues del arranque.
// =============================================================================

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use spin::{Mutex, Once};
use x86_64::instructions::interrupts;

use super::task::{Task, TaskId};
use super::waker::{self, ColaListas};

/// Un futuro de tarea ya anclado en el heap: la moneda de los nacimientos.
pub type FuturoTarea = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Cola de NACIMIENTOS: tareas engendradas EN VIVO, mientras el reactor ya
/// corre. Una tarea cooperativa deposita aqui un futuro; el ejecutor lo recoge
/// al inicio de su proxima vuelta y lo da de alta. No la toca ningun manejador
/// de IRQ —solo tareas cooperativas y el propio ejecutor—, asi que un `Mutex`
/// llano basta: en un solo nucleo cooperativo nadie disputa el cerrojo.
static NACIMIENTOS: Once<Mutex<Vec<FuturoTarea>>> = Once::new();

/// Engendra una tarea nueva mientras el reactor ya corre (Fase 10). El ejecutor
/// la adoptara en su proxima vuelta. La invocan los orquestadores del kernel
/// —jamas una IRQ—.
pub fn engendrar(futuro: FuturoTarea) {
    if let Some(nacimientos) = NACIMIENTOS.get() {
        nacimientos.lock().push(futuro);
    }
}

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
        NACIMIENTOS.call_once(|| Mutex::new(Vec::new()));
        Executor {
            tareas: BTreeMap::new(),
            cola_listas: Arc::new(Mutex::new(VecDeque::new())),
            cache_wakers: BTreeMap::new(),
        }
    }

    /// Da de alta una tarea nueva y la marca como lista para su primer avance.
    pub fn spawn(&mut self, futuro: impl Future<Output = ()> + Send + 'static) {
        self.alistar(Task::nueva(futuro));
    }

    /// Inscribe una tarea ya construida en el censo y la encola para su primer
    /// avance. Es la via comun de `spawn` y de la recoleccion de nacimientos.
    fn alistar(&mut self, tarea: Task) {
        let id = tarea.id;
        if self.tareas.insert(id, tarea).is_some() {
            panic!("renaser :: TaskId duplicado — lo imposible ha ocurrido");
        }
        interrupts::without_interrupts(|| self.cola_listas.lock().push_back(id));
    }

    /// Recoge las tareas engendradas EN VIVO desde la ultima vuelta y las da de
    /// alta. Se invoca al inicio de cada ciclo del reactor (Fase 10).
    fn recoger_nacimientos(&mut self) {
        let Some(nacimientos) = NACIMIENTOS.get() else {
            return;
        };
        let recien: Vec<FuturoTarea> = {
            let mut cola = nacimientos.lock();
            if cola.is_empty() {
                return;
            }
            core::mem::take(&mut *cola)
        };
        for futuro in recien {
            self.alistar(Task::adoptar(futuro));
        }
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
    /// ningun despertar se pierda en la rendija entre uno y otro. Una tarea
    /// recien engendrada cuenta como trabajo: no se duerme con nacimientos
    /// pendientes de adoptar.
    fn dormir_si_inactivo(&self) {
        interrupts::disable();
        let hay_listas = !self.cola_listas.lock().is_empty();
        let hay_nacimientos = NACIMIENTOS
            .get()
            .map(|nacimientos| !nacimientos.lock().is_empty())
            .unwrap_or(false);
        if hay_listas || hay_nacimientos {
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
            self.recoger_nacimientos();
            self.avanzar_listas();
            self.dormir_si_inactivo();
        }
    }
}
