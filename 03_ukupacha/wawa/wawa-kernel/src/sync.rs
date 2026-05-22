// =============================================================================
//  renaser :: kernel/src/sync.rs — la celda de inicializacion unica
// -----------------------------------------------------------------------------
//  Las estructuras globales del kernel (GDT, TSS, IDT, el heap...) nacen una
//  sola vez, durante el arranque secuencial y de un solo hilo, y despues solo
//  se leen. `CeldaSync` envuelve ese unico `unsafe` en una abstraccion comun:
//  un contrato de unicidad que el codigo de arranque garantiza por construccion.
// =============================================================================

use core::cell::UnsafeCell;

/// Celda `Sync` para estado global de inicializacion unica.
pub(crate) struct CeldaSync<T>(UnsafeCell<T>);

// SEGURIDAD: cada celda se escribe una sola vez, durante el arranque, antes de
// que existan interrupciones o concurrencia; despues es de solo lectura.
unsafe impl<T> Sync for CeldaSync<T> {}

impl<T> CeldaSync<T> {
    /// Crea una celda con su valor inicial.
    pub(crate) const fn nueva(valor: T) -> Self {
        CeldaSync(UnsafeCell::new(valor))
    }

    /// Puntero crudo al contenido. Quien lo usa asume el contrato de unicidad.
    pub(crate) fn puntero(&self) -> *mut T {
        self.0.get()
    }
}
