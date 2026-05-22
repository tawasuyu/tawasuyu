// =============================================================================
//  renaser :: kernel/src/main.rs — el punto de entrada y la orquestacion
// -----------------------------------------------------------------------------
//  Aqui no nace una terminal: nace una superficie. renaser es un kernel
//  asincrono de Espacio de Direccionamiento Unico (SASOS) que rompe con el
//  paradigma POSIX — sin TTYs, sin archivos planos, sin capas GNU.
//
//  Este archivo es deliberadamente delgado: solo el arranque y el cableado.
//  Cada subsistema vive en su propio modulo (ver `ARCHITECTURE.md`):
//
//    grafico      — color, framebuffer fisico y lienzo de doble bufer.
//    consola      — superficie de texto/imagen; rasteriza glifos con fontdue.
//    baliza       — red de seguridad visual; manejadores de panico y de OOM.
//    sync         — `CeldaSync`, la celda de inicializacion unica.
//    gdt          — GDT + TSS + stack de emergencia del doble fallo.
//    interrupts   — IDT, excepciones de CPU e interrupciones de hardware.
//    pic          — el par 8259 remapeado y el temporizador (PIT).
//    drivers      — descubrimiento de hardware y E/S de disco asincrona por
//                   interrupcion: el bus PCI y el virtio-blk (Fases 6.1, 6.2).
//    almacen      — el grafo de objetos direccionado por contenido (Fase 6.1c).
//    manifiesto   — el Manifiesto de Genesis: que apps nacen del grafo (Fase 7).
//    memory       — el heap dinamico del kernel (`#[global_allocator]`).
//    async_system — el reactor cooperativo: ejecutor, tareas, wakers, teclado
//                   y el reloj que marca el compas de los fotogramas (Fase 5).
//    texto        — rasterizacion de tipografia vectorial (fontdue).
//    wasm         — el runtime WebAssembly, la matriz de capacidades y el
//                   escudo de combustible que acota el tiempo de cada app.
// =============================================================================

#![no_std] // Prohibido `std`: bajo nosotros no hay sistema operativo alguno.
#![no_main] // El punto de entrada lo define el cargador, no la convencion C.
#![feature(abi_x86_interrupt)] // ABI de las rutinas de excepcion (Fase 2).
#![feature(alloc_error_handler)] // Manejador propio de agotamiento de heap (Fase 3).
#![deny(unsafe_op_in_unsafe_fn)] // Cada operacion `unsafe` se justifica explicitamente.

extern crate alloc; // El heap esta vivo: `alloc::*` queda disponible (Fase 3).

use alloc::format;

use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::info::{FrameBufferInfo, MemoryRegionKind, MemoryRegions, PixelFormat};
use bootloader_api::{entry_point, BootInfo};
use spin::Mutex;

// --- Subsistemas del kernel ---
mod almacen;
mod async_system;
mod baliza;
mod consola;
mod drivers;
mod gdt;
mod grafico;
mod interrupts;
mod manifiesto;
mod memory;
mod pic;
mod sync;
mod texto;
mod wasm;

// Reexportaciones para que los submodulos conserven rutas `crate::` estables.
pub(crate) use consola::volcar_marco_wasm;
pub(crate) use sync::CeldaSync;

use async_system::executor::Executor;
use baliza::BALIZA_PANICO;
use consola::{Consola, CONSOLA};
use grafico::{
    codificar, reclamar_memoria_lienzo, Color, Lienzo, Pantalla, ALTO_MAX, ANCHO_MAX,
};

/// Configuracion que el cargador `bootloader` aplicara antes de cedernos la CPU.
static CONFIG_ARRANQUE: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    // Pedimos la memoria fisica mapeada: cimiento para futuras fases.
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

// `entry_point!` genera el simbolo `_start`, valida la firma de `kernel_main`
// y nos transfiere el control con seguridad de tipos.
entry_point!(kernel_main, config = &CONFIG_ARRANQUE);

/// Detiene la CPU de forma definitiva: `hlt` la duerme hasta una interrupcion.
pub(crate) fn detener() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

/// Tarea cooperativa de una aplicacion WASM. En cada pulso del reloj le concede
/// un `tick` —un fotograma de trabajo— y cede la CPU hasta el siguiente; entre
/// medias corren sus vecinas. Si la app falla o agota su combustible, se la
/// DESALOJA: se tatua su region con la baliza de purpura y la tarea concluye.
/// El ejecutor la retira del censo, su memoria se libera, el kernel sigue vivo.
async fn tarea_aplicacion(mut app: wasm::AplicacionWasm) {
    loop {
        async_system::reloj::EsperaFrame::nueva().await;
        if let Err(falla) = app.tick() {
            // El color de la baliza delata la causa: purpura si agoto su tiempo
            // o aborto, amarillo si reviento su techo de memoria.
            consola::pintar_desalojo(app.region(), falla.color_baliza());
            return;
        }
    }
}

/// FASE 6.2 — la prueba viva de la E/S asincrona. Esta tarea del reactor lee el
/// sector 0 del disco SIN bloquear: cede la CPU mientras el disco trabaja —las
/// apps siguen pintando entre tanto— y la IRQ del disco la reanuda cuando el
/// bloque esta listo. Deja en la consola el resultado y cuantas interrupciones
/// de disco se atendieron — el testigo de que la E/S por sondeo quedo atras.
async fn tarea_sonda_disco() {
    let resultado = drivers::disco::leer_bloques(0, 1).await;
    let Some(consola) = CONSOLA.get() else {
        return;
    };
    let mut consola = consola.lock();
    match resultado {
        Ok(_) => consola.escribir(&format!(
            "disco :: sonda asincrona OK -- {} IRQ de disco atendidas\n",
            drivers::disco::pulsos_disco(),
        )),
        Err(motivo) => {
            consola.escribir(&format!("disco :: sonda asincrona fallida -- {motivo}\n"))
        }
    }
    consola.presentar();
}

/// Da vida a una aplicacion del userspace a partir de su `EntradaApp` del
/// manifiesto: recupera su bytecode del grafo, la carga en su region y la
/// despacha como tarea cooperativa del reactor. Si el bytecode falta, esta
/// corrupto, o la carga fracasa, se salda pintando la region de la app con
/// la baliza de desalojo — el kernel no se inmuta y sigue con las demas.
fn encender_app(ejecutor: &mut Executor, entrada: &manifiesto::EntradaApp) {
    let region = entrada.region();
    // Recuperar el bytecode del grafo. `recuperar` recomputa el hash del
    // objeto y verifica su integridad: un bytecode corrupto se delata aqui
    // —y la app se niega, no se instancia un modulo en el que no se confia.
    let bytecode = match almacen::recuperar(&entrada.bytecode) {
        Ok(Some(objeto)) => objeto.datos,
        _ => {
            consola::pintar_desalojo(region, Color::DESALOJO);
            return;
        }
    };
    match wasm::AplicacionWasm::cargar(&bytecode, region, entrada.techo_memoria as usize) {
        Ok(app) => ejecutor.spawn(tarea_aplicacion(app)),
        Err(_) => consola::pintar_desalojo(region, Color::DESALOJO),
    }
}

/// Escribe una linea en la consola global y la presenta. Atajo para los
/// informes de arranque; no hace nada si la consola aun no existe.
fn reportar(linea: &str) {
    if let Some(consola) = CONSOLA.get() {
        let mut consola = consola.lock();
        consola.escribir(linea);
        consola.escribir("\n");
        consola.presentar();
    }
}

/// FASE 7 :: puebla el userspace DESDE EL GRAFO. Carga el Manifiesto de
/// Genesis; si el disco no tiene uno —disco virgen—, lo siembra y lo vuelve a
/// cargar. Por cada `EntradaApp`, enciende su aplicacion. Toda falla se
/// reporta a la consola y NO detiene el arranque: el kernel se levanta con
/// las apps que pueda — o con ninguna, si el grafo no tiene userspace.
fn cargar_userspace(ejecutor: &mut Executor) {
    let manifiesto = match manifiesto::cargar() {
        Ok(Some(m)) => Some(m),
        // Disco sin manifiesto: sembrar la genesis y volver a cargarlo.
        Ok(None) => match manifiesto::sembrar_genesis() {
            Ok(_) => {
                reportar("manifiesto :: genesis sembrada en disco virgen");
                manifiesto::cargar().ok().flatten()
            }
            Err(motivo) => {
                reportar(&format!("manifiesto :: siembra fallida -- {motivo}"));
                None
            }
        },
        Err(motivo) => {
            reportar(&format!("manifiesto :: carga fallida -- {motivo}"));
            None
        }
    };

    match &manifiesto {
        Some(m) => reportar(&format!(
            "manifiesto :: {} apps nacidas del grafo",
            m.apps.len(),
        )),
        None => reportar("manifiesto :: sin userspace -- el kernel se levanta solo"),
    }

    if let Some(m) = manifiesto {
        for entrada in &m.apps {
            encender_app(ejecutor, entrada);
        }
    }
}

/// Localiza la mayor region de RAM libre que el cargador reporto — la cantera
/// de la que el DMA del disco tomara sus marcos fisicos.
fn mayor_region_usable(regiones: &MemoryRegions) -> Option<(u64, u64)> {
    regiones
        .iter()
        .filter(|region| matches!(region.kind, MemoryRegionKind::Usable))
        .map(|region| (region.start, region.end))
        .max_by_key(|&(inicio, fin)| fin - inicio)
}

/// FASE 6.1c — funda el grafo de objetos. Monta el disco virtio-blk, lee o
/// forja el superbloque, reconstruye el indice recorriendo el log y deja
/// constancia visual: cuantos sectores tiene el disco, cuantos objetos viven
/// ya en el grafo y si el arranque encontro —o no— una raiz de la que tirar.
fn informar_almacen() {
    // Fundar el almacen ANTES de tomar el cerrojo de la consola: el montaje
    // del disco hace E/S por sondeo y nada de ello reclama la consola.
    let resultado = almacen::init();
    let Some(consola) = CONSOLA.get() else {
        return;
    };
    let mut consola = consola.lock();
    match resultado {
        Ok(resumen) => {
            let estado = if resumen.formateado {
                "disco formateado"
            } else {
                "grafo montado"
            };
            consola.escribir(&format!(
                "almacen :: {} :: {} sectores :: {} objetos :: raiz {}\n",
                estado,
                resumen.capacidad,
                resumen.objetos,
                if resumen.raiz { "presente" } else { "ausente" },
            ));
        }
        Err(motivo) => {
            consola.escribir(&format!("almacen :: fallo :: {motivo}\n"));
        }
    }
    // FASE 6.2 — dejar constancia de la linea de IRQ por la que el disco
    // anunciara, ya sin sondeo, el fin de cada transferencia.
    match drivers::disco::irq() {
        Some(irq) => {
            consola.escribir(&format!("disco :: virtio-blk en IRQ {irq} -- E/S asincrona\n"))
        }
        None => consola.escribir("disco :: IRQ no enrutada -- E/S por sondeo\n"),
    }
    consola.presentar();
}

// =============================================================================
//  PUNTO DE ENTRADA DEL KERNEL
// =============================================================================

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    // --- 1. Recuperar el framebuffer GOP que el firmware nos confio. ---
    let framebuffer = match boot_info.framebuffer.as_mut() {
        Some(fb) => fb,
        None => detener(),
    };
    let info: FrameBufferInfo = framebuffer.info();
    let formato: PixelFormat = info.pixel_format;
    let pantalla = Pantalla::adoptar(framebuffer, info);

    // Datos para la sonda de disco (Fase 6.1b): el offset al que el cargador
    // mapeo la memoria fisica y la mayor region de RAM libre para el DMA.
    let offset_fisico = boot_info.physical_memory_offset.into_option();
    let region_dma = mayor_region_usable(&boot_info.memory_regions);

    // --- 2. Encender la baliza: la red de seguridad visual va primero. ---
    BALIZA_PANICO.encender(
        &pantalla,
        codificar(formato, Color::ALERTA),
        codificar(formato, Color::OOM),
    );

    // --- 3. Cimientos de fallos e interrupciones (Fases 2.0 y 2.1). ---
    gdt::init();
    interrupts::init();
    pic::init();

    // --- 4. FASE 3 :: fundar el heap. A partir de aqui, `alloc` esta vivo. ---
    memory::init();

    // --- 5. Con el heap activo, fundar lo que depende de el: el canal de
    //        scancodes, el reloj de fotogramas y la tipografia vectorial. ---
    async_system::teclado::init();
    async_system::reloj::init();
    texto::init();

    // --- 6. Construir el lienzo y la consola; pintar el rotulo inicial,
    //        ya rasterizado por fontdue, y publicar la consola globalmente. ---
    let memoria = match reclamar_memoria_lienzo() {
        Some(m) => m,
        None => detener(),
    };
    let mut lienzo = Lienzo::nuevo(
        memoria,
        info.width.min(ANCHO_MAX),
        info.height.min(ALTO_MAX),
        formato,
    );
    lienzo.limpiar(Color::LIENZO_EN_REPOSO);

    let mut consola = Consola::nueva(lienzo, pantalla);
    consola.escribir("renaser :: fase 6.2 -- E/S de disco asincrona por interrupcion\n");
    consola.presentar();
    CONSOLA.call_once(|| Mutex::new(consola));

    // --- 6.5. FASE 6.1c :: fundar el subsistema de disco y, sobre el, el grafo
    //          de objetos: enumerar el bus PCI, montar el transporte virtio-blk,
    //          y leer o forjar el superbloque del almacen direccionado por
    //          contenido. El kernel adquiere, por fin, una memoria que perdura. ---
    match (offset_fisico, region_dma) {
        (Some(offset), Some((inicio, fin))) => {
            drivers::disco::init(offset, inicio, fin);
            informar_almacen();
        }
        _ => {
            if let Some(consola) = CONSOLA.get() {
                let mut consola = consola.lock();
                consola.escribir("virtio-blk :: omitido -- memoria fisica sin mapear\n");
                consola.presentar();
            }
        }
    }

    // --- 7. FASE 7 :: levantar el reactor y poblar el userspace DESDE EL
    //        GRAFO. El kernel ya no empotra los modulos WASM: lee el
    //        Manifiesto de Genesis —si el disco esta virgen, lo siembra— e
    //        instancia cada `EntradaApp` recuperando su bytecode del grafo de
    //        objetos. Las cinco apps de genesis (dos instancias de hello, la
    //        discola, la glotona y la cronista) nacen ahora del disco, no del
    //        binario del kernel.
    //
    //        Las interrupciones se habilitan AHORA: el temporizador marcara el
    //        compas de los fotogramas y la IRQ del teclado difundira cada
    //        scancode a los canales que las apps consultan. ---
    let mut ejecutor = Executor::nuevo();
    cargar_userspace(&mut ejecutor);
    // FASE 6.2 :: una tarea mas del reactor — no una app WASM— que sondea el
    // disco de forma ASINCRONA: la demostracion de que la IRQ del disco
    // conduce la E/S sin detener a las aplicaciones visuales.
    ejecutor.spawn(tarea_sonda_disco());
    x86_64::instructions::interrupts::enable();
    ejecutor.run();
}
