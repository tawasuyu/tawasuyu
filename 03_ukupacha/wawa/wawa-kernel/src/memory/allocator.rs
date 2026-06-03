// =============================================================================
//  renaser :: kernel/src/memory/allocator.rs — Fase 3 :: el asignador global
// -----------------------------------------------------------------------------
//  Reutilizamos un algoritmo probado (`linked_list_allocator`) en lugar de
//  arriesgar un asignador propio a bugs de fragmentacion. El heap es una region
//  estatica que PERSISTE durante toda la vida del kernel.
// =============================================================================

use linked_list_allocator::LockedHeap;

use crate::CeldaSync;

/// 64 MiB de heap para el kernel. La arquitectura estimaba 16, pero `fontdue`,
/// al analizar una tipografia real, exige mas holgura; el manejador de OOM —la
/// franja naranja— fue justamente lo que delato esa cota demasiado corta.
const TAM_HEAP: usize = 64 * 1024 * 1024;

/// Region de respaldo del heap, alineada a pagina, residente en `.bss`. Su
/// campo solo se alcanza via puntero crudo —asi lo exige `GlobalAlloc`—, de ahi
/// el `allow(dead_code)`: la memoria se usa, aunque no por una via «normal».
#[repr(align(4096))]
#[allow(dead_code)]
struct RegionHeap([u8; TAM_HEAP]);

/// La memoria fisica del heap. Nace a ceros, no engorda el binario.
static REGION_HEAP: CeldaSync<RegionHeap> = CeldaSync::nueva(RegionHeap([0u8; TAM_HEAP]));

/// El asignador global. Todo `alloc::*` —`Box`, `Vec`, `BTreeMap`, `Arc`...—
/// se apoya, en silencio, sobre este.
#[global_allocator]
static ASIGNADOR: LockedHeap = LockedHeap::empty();

/// Funda el heap del kernel. Debe invocarse UNA sola vez, en el arranque,
/// antes del primer uso de cualquier estructura de `alloc`.
pub fn init() {
    let inicio: *mut u8 = REGION_HEAP.puntero().cast::<u8>();
    // SEGURIDAD: la region es estatica, de uso exclusivo del asignador, vive
    // tanto como el kernel, y `init` se invoca una unica vez en el arranque.
    unsafe {
        ASIGNADOR.lock().init(inicio, TAM_HEAP);
    }
}

/// `(bytes_usados, bytes_totales)` del heap del kernel. Lo consume el medidor de
/// RAM del marco (`compositor::pata_marco`): el primer dato real del sistema que
/// `pata-core` pinta sobre el framebuffer de wawa. No llamar desde dentro de una
/// asignación (tomaría el lock del asignador dos veces).
pub fn stats() -> (usize, usize) {
    let heap = ASIGNADOR.lock();
    (heap.used(), heap.size())
}
