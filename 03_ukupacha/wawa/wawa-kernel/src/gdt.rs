// =============================================================================
//  renaser :: kernel/src/gdt.rs — Fase 2 :: cimientos del manejo de fallos
// -----------------------------------------------------------------------------
//  La GDT y el TSS no se ven jamas, pero sostienen todo lo demas. Su cometido
//  esencial en renaser es reservar un stack de emergencia inquebrantable: si la
//  pila del kernel se desborda, el manejador de doble fallo aterrizara sobre
//  terreno firme en lugar de arrastrar al sistema hacia un fallo triple.
//
//  Una sola verdad, instalada una sola vez, que PERSISTE bajo cada interrupcion.
// =============================================================================

use x86_64::instructions::segmentation::{Segment, CS, DS, ES, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

use crate::CeldaSync;

/// Indice, dentro de la Interrupt Stack Table del TSS, del stack reservado
/// para el manejador de doble fallo.
pub const IST_DOBLE_FALLO: u16 = 0;

/// Tamaño del stack de emergencia: 20 KiB. Holgura suficiente para diagnosticar
/// el fallo y encender la baliza sin volver a tocar la pila comprometida.
const TAM_STACK_EMERGENCIA: usize = 4096 * 5;

/// Stack de emergencia del doble fallo. Vive en `.bss`; la IST apuntara a su
/// extremo superior, pues la pila de x86_64 crece hacia direcciones menores.
static STACK_EMERGENCIA: CeldaSync<[u8; TAM_STACK_EMERGENCIA]> =
    CeldaSync::nueva([0; TAM_STACK_EMERGENCIA]);

/// El Task State Segment. En 64 bits ya no describe «tareas»: lo conservamos
/// unicamente por su Interrupt Stack Table.
static TSS: CeldaSync<TaskStateSegment> = CeldaSync::nueva(TaskStateSegment::new());

/// La Global Descriptor Table propia de renaser.
static GDT: CeldaSync<GlobalDescriptorTable> = CeldaSync::nueva(GlobalDescriptorTable::new());

/// Instala una GDT propia con su TSS y arma el stack de emergencia.
///
/// Debe invocarse una sola vez, durante el arranque, ANTES de cargar la IDT:
/// la IDT inscribe en cada entrada el selector de codigo vigente en ese momento.
pub fn init() {
    // --- 1. Inscribir en el TSS la cima del stack de emergencia. ---
    // La pila crece hacia abajo: la IST exige la direccion MAS ALTA del bloque.
    let cima_stack = VirtAddr::from_ptr(STACK_EMERGENCIA.puntero()) + TAM_STACK_EMERGENCIA as u64;
    // SEGURIDAD: `init` corre una sola vez, en arranque secuencial de un solo
    // hilo; nada mas referencia el TSS todavia.
    unsafe {
        (*TSS.puntero()).interrupt_stack_table[IST_DOBLE_FALLO as usize] = cima_stack;
    }

    // --- 2. Construir la GDT: codigo de kernel, datos de kernel y el TSS. ---
    // SEGURIDAD: mismo argumento de unicidad; la GDT aun no esta cargada y
    // ningun otro flujo posee referencias a estas celdas.
    let gdt: &'static mut GlobalDescriptorTable = unsafe { &mut *GDT.puntero() };
    let tss: &'static TaskStateSegment = unsafe { &*TSS.puntero() };

    let sel_codigo: SegmentSelector = gdt.append(Descriptor::kernel_code_segment());
    let sel_datos: SegmentSelector = gdt.append(Descriptor::kernel_data_segment());
    let sel_tss: SegmentSelector = gdt.append(Descriptor::tss_segment(tss));

    // --- 3. Activar la GDT y recargar TODOS los registros de segmento. ---
    let gdt_estatica: &'static GlobalDescriptorTable = gdt;
    gdt_estatica.load();
    // SEGURIDAD: los tres selectores apuntan a entradas validas de la GDT
    // recien cargada. Recargar SS/DS/ES es IMPRESCINDIBLE, no opcional: el
    // cargador nos deja esos registros con selectores de SU tabla; si SS
    // conservara el valor heredado, el primer `iretq` de una rutina de
    // excepcion intentaria recargar un selector que en nuestra GDT ya no
    // describe un segmento de datos, y degeneraria en un #GP.
    unsafe {
        CS::set_reg(sel_codigo);
        SS::set_reg(sel_datos);
        DS::set_reg(sel_datos);
        ES::set_reg(sel_datos);
        load_tss(sel_tss);
    }
}
