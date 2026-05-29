// =============================================================================
//  renaser :: kernel/src/drivers/xhci/dma.rs — buferes DMA del controlador xHCI
// -----------------------------------------------------------------------------
//  XHCI exige varias estructuras DMA-coherentes alineadas a 64 bytes minimo
//  (DCBAA), 16 bytes (TRBs de los rings) o 4 KiB (Device Context, Input
//  Context). Como los marcos de la arena DMA del kernel ya son de 4 KiB
//  alineados a pagina, cualquier estructura xHCI cabe sin tocar alineamiento
//  manual — basta pedir N marcos contiguos.
//
//  Este modulo envuelve la primitiva `drivers::disco::asignar_paginas_dma`
//  en una vista tipada con direccion fisica + puntero virtual + tamano,
//  lista para entregarsela a `xhci::Registers` o programar como base address
//  en CRCR/ERSTBA/DCBAAP.
// =============================================================================

use core::ptr::NonNull;

use crate::drivers::disco;
use crate::memory::mmio;

const TAM_PAGINA: usize = 4096;

/// Un bufer DMA contiguo en la arena del kernel: la fisica que el dispositivo
/// usa como base, y un puntero virtual que el driver dereferencia. La memoria
/// queda zero-filled tras `asignar_zero`. El bufer NO se libera al hacer drop
/// — el driver xHCI vive lo que vive el kernel, igual que el grafo de
/// objetos; el COW de marcos no aplica en este caso.
#[derive(Debug)]
pub struct BuferDma {
    /// Direccion fisica del primer marco. La que se programa en los
    /// registros del controlador.
    pub fisica: u64,
    /// Puntero virtual escribible al primer byte. Sirve para llenar TRBs,
    /// Slot Contexts, etc.
    pub virtual_: NonNull<u8>,
    /// Tamano en bytes — siempre multiplo de 4 KiB.
    pub bytes: usize,
}

impl BuferDma {
    /// Asigna `bytes` redondeados al techo de pagina, zero-fillea y devuelve
    /// el bufer. Devuelve `None` si la arena DMA esta exhausta — el
    /// controlador xHCI quedara sin montar pero el resto del kernel sigue.
    pub fn asignar_zero(bytes: usize) -> Option<Self> {
        let paginas = bytes.div_ceil(TAM_PAGINA).max(1);
        let fisica = disco::asignar_paginas_dma(paginas)?;
        let virt = mmio::a_virtual(fisica) as *mut u8;
        // SEGURIDAD: `asignar_paginas_dma` entrego marcos exclusivos y el
        // mapeo lineal del bootloader los hace alcanzables en `fisica + offset`.
        // Zero-fill es requisito de xHCI para DCBAA y los rings (cycle bit
        // empieza limpio).
        unsafe {
            core::ptr::write_bytes(virt, 0, paginas * TAM_PAGINA);
        }
        Some(BuferDma {
            fisica,
            virtual_: NonNull::new(virt)?,
            bytes: paginas * TAM_PAGINA,
        })
    }

    /// Vista del bufer como slice escribible. Util para indexar TRBs por
    /// posicion en el ring.
    ///
    /// SEGURIDAD: el bufer apunta a memoria propiedad de esta `BuferDma`,
    /// que no se mueve ni se libera. El slice vive lo que viva el `&mut`.
    #[allow(dead_code)]
    pub fn como_slice_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.virtual_.as_ptr(), self.bytes) }
    }
}

// SEGURIDAD: el bufer envuelve un puntero crudo a memoria DMA exclusiva, no
// se accede concurrentemente fuera de su Mutex. Send es legal.
unsafe impl Send for BuferDma {}
