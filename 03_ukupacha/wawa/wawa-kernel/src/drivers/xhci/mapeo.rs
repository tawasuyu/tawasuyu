// =============================================================================
//  renaser :: kernel/src/drivers/xhci/mapeo.rs — Mapper para la crate `xhci`
// -----------------------------------------------------------------------------
//  La crate `xhci` toma una direccion FISICA del BAR0 y pide al consumidor que
//  la mapee a virtual cuando necesita un registro. Esa indireccion permite
//  que la crate sea portable (ARM/RISC-V/etc) — el sistema decide como hacer
//  el mapeo. En renaser, reusamos `memory::mmio::mapear`, que ya cubre el
//  caso virtio: abre paginas en la tabla L4 con flags NO_CACHE|WRITE_THROUGH
//  para que los registros del controlador no caigan en cache.
//
//  Una vez mapeado, el offset virtual = `phys + physical_memory_offset` (el
//  truco de mapeo de memoria fisica del bootloader). El Mapper devuelve esa
//  virtual a la crate `xhci`, que la usa con `volatile` reads/writes.
// =============================================================================

use core::num::NonZeroUsize;

use xhci::accessor::Mapper;

use crate::memory::mmio;

/// Mapper para la crate `xhci`. Sin estado — la traduccion fisica → virtual
/// usa el offset global que `memory::mmio` ya conoce. La crate `xhci` lo
/// clona por cada slot que abre; mantenerlo sin estado evita sincronizacion.
#[derive(Clone, Copy, Debug)]
pub struct MapeadorXhci;

impl Mapper for MapeadorXhci {
    /// La crate `xhci` invoca aqui para mapear una region MMIO del BAR0 (o
    /// de un secondary BAR de extended capabilities). Si el cargador ya
    /// mapeo la pagina, `mmio::mapear` la respeta; si no, abre la pagina
    /// nueva en la tabla L4 con flags MMIO (NO_CACHE|WRITE_THROUGH).
    ///
    /// Devuelve la direccion VIRTUAL que el consumidor puede desreferenciar
    /// con escrituras volatil.
    unsafe fn map(&mut self, phys_base: usize, bytes: usize) -> NonZeroUsize {
        mmio::mapear(phys_base as u64, bytes);
        let virt = mmio::a_virtual(phys_base as u64);
        // SEGURIDAD: `a_virtual` devuelve `phys_base + physical_memory_offset`,
        // que en un sistema con `physical_memory_offset > 0` es siempre no nulo.
        // Si el offset fuera 0 y `phys_base` fuera 0 — caso imposible en x86_64
        // porque ningun BAR PCI legitimo apunta a fisica 0 — el `expect` lo
        // delataria antes de que la crate `xhci` derefencie nulo.
        NonZeroUsize::new(virt as usize).expect("xhci :: BAR mapeado a virtual 0")
    }

    /// La crate `xhci` invoca aqui cuando deja de usar una region. renaser no
    /// libera mapeos MMIO — viven lo que vive el kernel —; el unmap es un
    /// no-op consciente. La pagina queda en la tabla L4 hasta el reboot.
    fn unmap(&mut self, _virt_base: usize, _bytes: usize) {}
}
