// =============================================================================
//  renaser :: async_system/reloj.rs — Fase 5 :: el compas de los fotogramas
// -----------------------------------------------------------------------------
//  El temporizador (PIT, IRQ0) ya no solo despierta al ejecutor de su `hlt`:
//  ahora marca el COMPAS del userspace. Cada pulso a 100 Hz es un fotograma —
//  una oportunidad de cesion cooperativa. `EsperaFrame` convierte ese pulso de
//  hardware en un `Future`: una tarea WASM hace su trabajo de un fotograma y
//  `.await`-ea el siguiente, cediendo la CPU a sus vecinas mientras tanto.
// =============================================================================

use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU64, Ordering};
use core::task::{Context, Poll, Waker};

use spin::{Mutex, Once};
use x86_64::instructions::interrupts;

/// Pulsos del temporizador acumulados desde el arranque. Lo incrementa la IRQ0;
/// lo consulta `EsperaFrame`. Atomico: la IRQ y el hilo principal lo comparten.
static CONTADOR_PULSOS: AtomicU64 = AtomicU64::new(0);

/// Censo de wakers en espera del proximo fotograma. Vive en el heap —de ahi el
/// `Once`—; tras su `Mutex`, lo disputan el lado tarea y el manejador de IRQ0.
static EN_ESPERA: Once<Mutex<Vec<Waker>>> = Once::new();

/// Funda el censo de wakers del reloj. Requiere el heap ya activo; debe
/// invocarse una sola vez, antes de habilitar las interrupciones.
pub fn init() {
    EN_ESPERA.call_once(|| Mutex::new(Vec::new()));
}

/// Punto de entrada DESDE el manejador de IRQ0. Avanza el contador de pulsos y
/// despierta a cuantas tareas aguardaban el fotograma. Deliberadamente breve y
/// libre de panicos: corre en contexto de interrupcion.
pub fn pulso() {
    // `Release`: el avance del contador se publica ANTES de que los wakers
    // reinyecten sus tareas; el `poll` que sigue lo leera con `Acquire` y vera,
    // garantizado, el valor nuevo.
    CONTADOR_PULSOS.fetch_add(1, Ordering::Release);
    if let Some(censo) = EN_ESPERA.get() {
        // En contexto de IRQ las interrupciones ya estan acalladas; tomar aqui
        // el cerrojo no puede interbloquear, pues el lado tarea siempre lo toma
        // con las interrupciones desactivadas (ver `inscribir`).
        for waker in censo.lock().drain(..) {
            waker.wake();
        }
    }
}

/// Numero de pulsos del temporizador desde el arranque.
fn pulsos() -> u64 {
    CONTADOR_PULSOS.load(Ordering::Acquire)
}

/// Inscribe un waker en el censo de espera del proximo fotograma.
fn inscribir(waker: Waker) {
    if let Some(censo) = EN_ESPERA.get() {
        // El cerrojo lo disputa el manejador de IRQ0: tomarlo con las
        // interrupciones acalladas hace IMPOSIBLE el interbloqueo.
        interrupts::without_interrupts(|| censo.lock().push(waker));
    }
}

/// Un `Future` que se resuelve en el proximo pulso del temporizador. Es la
/// unidad de cesion cooperativa del userspace: una tarea WASM hace su trabajo
/// de un fotograma y `.await`-ea un `EsperaFrame` para ceder hasta el siguiente.
pub struct EsperaFrame {
    /// Pulso a partir del cual la espera se da por cumplida.
    objetivo: u64,
}

impl EsperaFrame {
    /// Crea una espera que se resolvera en el siguiente pulso del temporizador.
    pub fn nueva() -> EsperaFrame {
        EsperaFrame {
            objetivo: pulsos() + 1,
        }
    }
}

impl Future for EsperaFrame {
    type Output = ();

    fn poll(self: Pin<&mut Self>, contexto: &mut Context<'_>) -> Poll<()> {
        // Lectura optimista: si el fotograma ya llego, no hay nada que esperar.
        if pulsos() >= self.objetivo {
            return Poll::Ready(());
        }
        // Aun no: inscribir el waker y RE-VERIFICAR. Si un pulso se colo entre
        // la lectura de arriba y la inscripcion, este segundo chequeo lo atrapa;
        // sin el, el despertar se perderia hasta el pulso siguiente.
        inscribir(contexto.waker().clone());
        if pulsos() >= self.objetivo {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
