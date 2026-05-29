// =============================================================================
//  renaser :: kernel/src/memory/cache.rs — atributos de cache para MMIO video
// -----------------------------------------------------------------------------
//  El cargador `bootloader 0.11` mapea el framebuffer GOP con flags PRESENT |
//  WRITABLE | NO_EXECUTE — sin tocar la matriz IA32_PAT y sin setear PCD/PWT en
//  las PTE. Con el PAT por defecto (slot 0 = WB) eso convierte a la pantalla
//  fisica en memoria CACHED. En QEMU pasa desapercibido porque el "framebuffer"
//  es RAM normal con coherencia automatica; en metal real, cada `memcpy` del
//  lienzo entra en L1/L2/L3 y solo alcanza la RAM de video cuando la linea de
//  cache se desaloja. El scanout del LCD ve pixeles viejos en el intermedio
//  -> parpadeo. Y la polucion de cache mata el resto del kernel -> lentitud.
//
//  La cura es WRITE-COMBINING: las escrituras se acumulan en el Write
//  Combining Buffer y se flushean cuando el buffer se llena o ante una
//  instruccion serializadora. Cero polucion de cache, latencia minima de
//  scanout, throughput cercano al maximo del bus de memoria de video.
//
//  Pasos:
//    1. `init_pat()` reprograma IA32_PAT para meter `WC` en el slot 4.
//       Mantiene los slots 0-3 con su semantica historica de modo que
//       cualquier mapeo legacy con flags por defecto siga viendose como WB.
//    2. `marcar_wc(virt, longitud)` recorre las PTE del rango y enciende el
//       bit PAT (bit 7 en una PTE de 4 KiB) — combinado con PCD=0 y PWT=0,
//       eso selecciona el slot 4 = WC. Reusa el `OffsetPageTable` del
//       modulo `memory::mmio`.
//
//  Despues del remap, el primer `presentar()` deberia sentirse instantaneo
//  y el parpadeo desaparecer.
// =============================================================================

use core::fmt::Write;

use x86_64::instructions::tlb;
use x86_64::registers::model_specific::Msr;
use x86_64::structures::paging::mapper::{FlagUpdateError, Mapper};
use x86_64::structures::paging::{Page, PageTableFlags, Size4KiB};
use x86_64::VirtAddr;

/// El MSR de la Page Attribute Table. Define como se interpreta cada uno de
/// los 8 slots del PAT en terminos de tipos de memoria (UC/WB/WT/WC/UC-).
const IA32_PAT: u32 = 0x277;

/// Codigos de tipo de memoria de la Intel SDM (Vol. 3A, §11.12).
const TIPO_UC: u8 = 0x00;
const TIPO_WC: u8 = 0x01;
const TIPO_WT: u8 = 0x04;
const TIPO_WB: u8 = 0x06;
const TIPO_UC_MENOS: u8 = 0x07;

/// Valor PAT personalizado: mantenemos los slots 0..=3 idénticos al default
/// (WB, WT, UC-, UC) para no romper mapeos heredados del bootloader, y en el
/// slot 4 metemos `WC`. Los slots 5-7 vuelven a valores razonables — no los
/// usamos hoy, pero conviene que sigan siendo tipos legitimos por si una
/// futura PTE selecciona uno por accidente.
///
/// Codificacion: cada byte del u64 es un slot. Byte 0 = slot 0, byte 7 = slot 7.
const PAT_PERSONALIZADO: u64 = u64::from_le_bytes([
    TIPO_WB,        // slot 0 — default WB (PAT=0,PCD=0,PWT=0)
    TIPO_WT,        // slot 1 — default WT (PAT=0,PCD=0,PWT=1)
    TIPO_UC_MENOS,  // slot 2 — default UC- (PAT=0,PCD=1,PWT=0)
    TIPO_UC,        // slot 3 — default UC  (PAT=0,PCD=1,PWT=1)
    TIPO_WC,        // slot 4 — WRITE-COMBINING (PAT=1,PCD=0,PWT=0) <-- el nuestro
    TIPO_WT,        // slot 5 — WT (PAT=1,PCD=0,PWT=1)
    TIPO_UC_MENOS,  // slot 6 — UC- (PAT=1,PCD=1,PWT=0)
    TIPO_UC,        // slot 7 — UC  (PAT=1,PCD=1,PWT=1)
]);

/// Reprograma IA32_PAT para colocar Write-Combining en el slot 4. Se invoca
/// UNA sola vez muy temprano en `kernel_main`, antes de hacer cualquier
/// remapeo de framebuffer con `marcar_wc`. Como wawa corre en un solo
/// nucleo no hace falta sincronizar entre CPUs; en multi-core habria que
/// invocar la misma escritura en cada AP.
///
/// SEGURIDAD: La SDM exige que entre la escritura del MSR y el uso de las
/// nuevas semanticas el TLB este vacio para los rangos afectados. Por eso
/// hacemos `tlb::flush_all` despues; los mapeos creados antes de la
/// reprogramacion siguen siendo correctos porque los slots 0-3 retienen sus
/// valores historicos.
pub fn init_pat() {
    let mut msr = Msr::new(IA32_PAT);
    unsafe {
        msr.write(PAT_PERSONALIZADO);
    }
    tlb::flush_all();
    let _ = writeln!(
        crate::baliza::Serie,
        "cache :: IA32_PAT reprogramado :: slot 4 = WC ({:#018x})",
        PAT_PERSONALIZADO,
    );
}

/// Marca el rango virtual `[inicio, inicio + longitud)` como Write-Combining,
/// encendiendo el bit PAT (bit 7 de la PTE de 4 KiB) en cada pagina. El bit
/// PAT, combinado con PCD=0 y PWT=0 del mapeo original, selecciona el slot
/// 4 del PAT — que `init_pat()` configuro como WC.
///
/// Caveat: la crate `x86_64` no expone el bit PAT con un nombre propio para
/// PTEs de 4 KiB; reusa `PageTableFlags::HUGE_PAGE` (= bit 7) que tiene
/// semantica distinta segun el nivel. Para una PTE L1 (que es donde apunta
/// `Size4KiB::update_flags`), bit 7 ES el bit PAT. El nombre `HUGE_PAGE`
/// es enganoso aqui pero el valor numerico es correcto.
///
/// Requiere que `memory::mmio::init` ya haya fundado el `MAPEADOR`, porque
/// reusa su `OffsetPageTable` para escribir las PTE. No alocar marcos
/// nuevos — el rango ya esta mapeado por el bootloader; solo cambian flags.
pub fn marcar_wc(inicio_virt: u64, longitud: usize) {
    let Some(mapeador) = crate::memory::mmio::mapeador() else {
        let _ = writeln!(
            crate::baliza::Serie,
            "cache :: marcar_wc :: MAPEADOR no fundado — abortando remap",
        );
        return;
    };
    if longitud == 0 {
        return;
    }
    let fin_virt = inicio_virt.saturating_add(longitud as u64 - 1);
    let pagina_inicio = Page::<Size4KiB>::containing_address(VirtAddr::new(inicio_virt));
    let pagina_fin = Page::<Size4KiB>::containing_address(VirtAddr::new(fin_virt));

    // Flags resultantes de la PTE: el bootloader mapeo con PRESENT|WRITABLE|
    // NO_EXECUTE; nosotros AGREGAMOS HUGE_PAGE (= bit PAT en L1) para
    // seleccionar el slot 4 del PAT. PCD y PWT quedan apagados — eso, con
    // PAT=1, indexa al slot 4 = WC.
    let nuevas_flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::NO_EXECUTE
        | PageTableFlags::HUGE_PAGE; // = bit PAT para PTE de 4 KiB

    let mut mapeador = mapeador.lock();
    let mut paginas_remarcadas: usize = 0;
    let mut paginas_no_mapeadas: usize = 0;
    let mut pagina = pagina_inicio;
    while pagina <= pagina_fin {
        // SEGURIDAD: solo cambiamos flags de paginas ya mapeadas por el
        // bootloader; no creamos mapeos nuevos ni movemos frames fisicos.
        // El TLB se flushea por pagina via el `MapperFlush`.
        let resultado = unsafe { mapeador.update_flags(pagina, nuevas_flags) };
        match resultado {
            Ok(flush) => {
                flush.flush();
                paginas_remarcadas += 1;
            }
            Err(FlagUpdateError::PageNotMapped) => {
                paginas_no_mapeadas += 1;
            }
            Err(FlagUpdateError::ParentEntryHugePage) => {
                // El bootloader mapeo esta region con paginas grandes (2 MiB
                // o 1 GiB). El tratamiento serial paginas grandes esta fuera
                // del alcance v1; para el framebuffer del bootloader es
                // improbable (mapea por 4 KiB explicitamente). Si ocurre,
                // dejamos traza y seguimos — esa pagina queda WB todavia.
                paginas_no_mapeadas += 1;
            }
        }
        pagina += 1;
    }

    let _ = writeln!(
        crate::baliza::Serie,
        "cache :: marcar_wc :: [{:#x}, {:#x}) :: {} paginas remarcadas, {} omitidas",
        inicio_virt,
        inicio_virt + longitud as u64,
        paginas_remarcadas,
        paginas_no_mapeadas,
    );
}
