// =============================================================================
//  renaser :: kernel/src/async_system — el reactor cooperativo (Fase 3 / 5)
// -----------------------------------------------------------------------------
//  Aqui renaser rompe con el modelo de hilos pesados de Linux. No hay cambio
//  de contexto en la CPU: las interrupciones de hardware no conmutan pilas,
//  DESPIERTAN tareas. El kernel avanza cooperativamente, una tarea cede y la
//  siguiente toma el relevo. Sobre estos cimientos corre, desde la Fase 5, el
//  bytecode WASM aislado por software de la Fase 4: cada aplicacion es una
//  tarea mas, y el `reloj` le marca el compas de sus fotogramas.
// =============================================================================

pub mod executor;
pub mod reloj;
pub mod task;
pub mod teclado;
pub mod waker;
