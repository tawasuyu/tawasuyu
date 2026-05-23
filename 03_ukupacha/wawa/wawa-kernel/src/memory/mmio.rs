// =============================================================================
//  renaser :: kernel/src/memory/mmio.rs — el mapeador de regiones MMIO
// -----------------------------------------------------------------------------
//  El cargador `bootloader_api` mapea la memoria fisica de la maquina, pero la
//  «ventana PCI de 64 bits» —donde OVMF aloja los BAR prefetchables de virtio
//  en QEMU q35 modernos— suele caer FUERA de ese mapeo: phys 32 GiB, 768 GiB o
//  mas, segun la fase de la luna. Sin mapeo, leer el primer registro del disco
//  era un #PF inmediato; con el, el kernel puede hablar con el dispositivo.
//
//  Este modulo abre paginas en la tabla L4 que el cargador nos cedio. Reutiliza
//  como asignador de marcos el de DMA del disco (`drivers::disco`): los marcos
//  para las tablas intermedias salen del mismo banco que los buferes virtio.
//  No mapea de mas: solo lo que se le pide, por paginas.
// =============================================================================

use core::sync::atomic::{AtomicU64, Ordering};

use spin::{Mutex, Once};

use x86_64::registers::control::Cr3;
use x86_64::structures::paging::mapper::{MapToError, Mapper};
use x86_64::structures::paging::{
    FrameAllocator, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr};

/// Desplazamiento al que el cargador mapeo la memoria fisica. Lo necesitamos
/// tanto para alcanzar la tabla L4 actual como para traducir la direccion
/// fisica del BAR a su virtual de destino.
static OFFSET_FISICO: AtomicU64 = AtomicU64::new(0);

/// El mapeador del kernel: una vista mutable de la tabla L4 actual envuelta en
/// el truco de `OffsetPageTable`, que lo asume todo accesible via el mapeo de
/// memoria fisica que el cargador ya dejo activo. Se funda en el arranque, una
/// sola vez, antes de cualquier llamada a `mapear`.
static MAPEADOR: Once<Mutex<OffsetPageTable<'static>>> = Once::new();

/// Funda el mapeador: localiza la tabla L4 activa (via CR3) y la envuelve en un
/// `OffsetPageTable` que aprovecha el mapeo de memoria fisica del cargador. A
/// partir de aqui `mapear` puede abrir paginas nuevas en la tabla.
pub fn init(offset_fisico: u64) {
    OFFSET_FISICO.store(offset_fisico, Ordering::Relaxed);
    let (l4_frame, _) = Cr3::read();
    let l4_virt = l4_frame.start_address().as_u64() + offset_fisico;
    // SEGURIDAD: el cargador mapeo toda la RAM fisica (incluida la tabla L4
    // activa) en `l4_phys + offset_fisico`. Esa direccion es valida, esta
    // alineada a pagina y la tabla vive lo que vive el kernel. La tomamos como
    // referencia mutable porque renaser es de un solo nucleo y todo acceso al
    // mapeador queda serializado tras el `Mutex` de `MAPEADOR`.
    let l4: &'static mut PageTable = unsafe { &mut *(l4_virt as *mut PageTable) };
    let mapeador = unsafe { OffsetPageTable::new(l4, VirtAddr::new(offset_fisico)) };
    MAPEADOR.call_once(|| Mutex::new(mapeador));
}

/// Asigna marcos para las tablas de paginas intermedias. Toma del banco DMA
/// del disco —no hay heap aqui— y deja cada marco a CEROS, como una tabla
/// vacia exige. Si el banco esta exhausto, devuelve `None` y el mapeo falla
/// en lugar de incendiar el kernel.
struct Marcos;

unsafe impl FrameAllocator<Size4KiB> for Marcos {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        let fisica = crate::drivers::disco::asignar_marco_para_tabla()?;
        let offset = OFFSET_FISICO.load(Ordering::Relaxed);
        // SEGURIDAD: `asignar_marco_para_tabla` entrego un marco exclusivo y
        // mapeado a `fisica + offset` por el cargador. Las tablas de paginas
        // exigen empezar a cero — si no, la CPU las leeria como llenas de
        // basura— asi que lo limpiamos antes de cederlo.
        unsafe {
            core::ptr::write_bytes((fisica + offset) as *mut u8, 0, 4096);
        }
        Some(PhysFrame::containing_address(PhysAddr::new(fisica)))
    }
}

/// Abre en la tabla L4 las paginas que cubren la region MMIO [fisica, fisica +
/// tam), de modo que cada pagina sea accesible en `fisica + offset_fisico` con
/// las banderas habituales del MMIO (escribible, sin cache). Si la pagina ya
/// estaba mapeada, se respeta la entrada existente sin gritar — un BAR puede
/// solaparse con regiones que el cargador ya cubrio—. Si el mapeo falla en
/// medio, registramos el problema en COM1 y seguimos: que la app que llame al
/// MMIO se entere por su propio fallo.
pub fn mapear(fisica: u64, tam: usize) {
    use core::fmt::Write;

    let Some(mapeador) = MAPEADOR.get() else {
        return;
    };
    if tam == 0 {
        return;
    }
    let offset = OFFSET_FISICO.load(Ordering::Relaxed);
    let virt_inicio = fisica + offset;
    let virt_fin = virt_inicio + tam as u64 - 1;

    let pagina_inicio = Page::<Size4KiB>::containing_address(VirtAddr::new(virt_inicio));
    let pagina_fin = Page::<Size4KiB>::containing_address(VirtAddr::new(virt_fin));
    let frame_inicio = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(fisica));

    let banderas = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::NO_CACHE
        | PageTableFlags::WRITE_THROUGH;

    let mut mapeador = mapeador.lock();
    let mut frame = frame_inicio;
    let mut pagina = pagina_inicio;
    while pagina <= pagina_fin {
        // SEGURIDAD: cada par (pagina, frame) describe un mapeo MMIO honesto:
        // la pagina virtual `fisica + offset` apunta a la pagina fisica
        // `fisica` que el dispositivo posee. El `Marcos` cede marcos limpios
        // para las tablas intermedias.
        let resultado = unsafe { mapeador.map_to(pagina, frame, banderas, &mut Marcos) };
        match resultado {
            Ok(flush) => flush.flush(),
            // El cargador ya habia mapeado la region como una pagina 4 KiB
            // (PageAlreadyMapped) o como una pagina huge — 2 MiB / 1 GiB —
            // (ParentEntryHugePage). En ambos casos el acceso ya funciona; no
            // hay nada que añadir y SEGUIMOS sin detenernos. Solo abortamos si
            // se nos agotan los marcos para una tabla intermedia nueva.
            Err(MapToError::PageAlreadyMapped(_)) | Err(MapToError::ParentEntryHugePage) => {}
            Err(MapToError::FrameAllocationFailed) => {
                let _ = writeln!(
                    crate::baliza::Serie,
                    "mmio :: sin marcos para la tabla — mapeo incompleto en {:#x}",
                    pagina.start_address().as_u64(),
                );
                return;
            }
        }
        pagina += 1;
        frame += 1;
    }
}
