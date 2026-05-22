// =============================================================================
//  renaser :: async_system/task.rs — Fase 3 :: la unidad de trabajo asincrono
// =============================================================================

use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU64, Ordering};
use core::task::{Context, Poll};

/// Identificador unico de una tarea. Autoincremental y atomico: dos tareas
/// jamas comparten identidad.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(u64);

impl TaskId {
    /// Acuña un identificador nuevo, distinto de todos los anteriores.
    fn nuevo() -> TaskId {
        static SIGUIENTE: AtomicU64 = AtomicU64::new(0);
        TaskId(SIGUIENTE.fetch_add(1, Ordering::Relaxed))
    }
}

/// Una tarea asincrona: un `Future` encapsulado, anclado (`Pin`) en el heap
/// para que su direccion nunca cambie mientras avanza.
pub struct Task {
    /// La identidad de la tarea, su nombre ante el ejecutor.
    pub id: TaskId,
    futuro: Pin<Box<dyn Future<Output = ()> + Send + 'static>>,
}

impl Task {
    /// Envuelve un `Future` en una tarea con identidad propia.
    pub fn nueva(futuro: impl Future<Output = ()> + Send + 'static) -> Task {
        Task {
            id: TaskId::nuevo(),
            futuro: Box::pin(futuro),
        }
    }

    /// Hace avanzar la tarea un paso. `Poll::Ready` significa que concluyo.
    pub fn poll(&mut self, contexto: &mut Context) -> Poll<()> {
        self.futuro.as_mut().poll(contexto)
    }
}
