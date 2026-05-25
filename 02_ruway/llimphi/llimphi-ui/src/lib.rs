//! llimphi-ui — Árbol de Estado Monádico (DAG UI).
//!
//! Estado inmutable + unidireccional estilo Elm. Cada evento genera una
//! nueva versión del estado. Nada de Rc<RefCell<...>> disperso.
//!
//! Fase 4: pendiente.
//!
//! Bucle:
//!   1. Input (click, tecla)
//!   2. update(state, event) -> new_state
//!   3. view(new_state) -> Tree
//!   4. llimphi-layout calcula Rects
//!   5. llimphi-raster genera Scene
//!   6. llimphi-hal hace swap

/// Marcador del árbol UI inmutable (DAG).
pub trait Node {
    fn id(&self) -> &str;
}
