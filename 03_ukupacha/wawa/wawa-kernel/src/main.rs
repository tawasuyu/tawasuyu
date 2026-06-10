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
//    compositor   — el teselado de las ventanas con `mirada-layout` (Fase 8).
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

use alloc::boxed::Box;
use alloc::format;

use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::info::{FrameBufferInfo, MemoryRegionKind, MemoryRegions, PixelFormat};
use bootloader_api::{entry_point, BootInfo};
use spin::{Mutex, Once};

// --- Subsistemas del kernel ---
mod akasha;
mod almacen;
mod async_system;
mod baliza;
mod claves;
mod compositor;
mod consola;
mod control;
mod drivers;
mod gdt;
mod grafico;
mod interrupts;
mod manifiesto;
mod memory;
mod pantallas;
mod pic;
mod sync;
mod texto;
mod tinkuy;
mod wasm;

// Reexportacion para que los submodulos conserven rutas `crate::` estables.
pub(crate) use sync::CeldaSync;

use alloc::vec::Vec;
use core::fmt::Write;
use core::sync::atomic::{AtomicUsize, Ordering};

use async_system::executor::Executor;
use baliza::BALIZA_PANICO;
use consola::{Consola, CONSOLA};
use grafico::{
    codificar, reclamar_memoria_lienzo, Color, Lienzo, Pantalla, RegionPantalla, ALTO_MAX,
    ANCHO_MAX,
};

/// Configuracion que el cargador `bootloader` aplicara antes de cedernos la CPU.
static CONFIG_ARRANQUE: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    // Pedimos la memoria fisica mapeada: cimiento para futuras fases.
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    // El default del bootloader (80 KiB) es marginal: el kernel corre la
    // pila UNICA del reactor cooperativo, y sobre ella se apilan la
    // inicializacion de virtio-drivers (el struct `VirtIOSound` con sus 4
    // virtqueues mas el `OwningQueue` ronda los ~20 KiB en un solo frame),
    // el interprete wasmi de cada app de genesis y el compositor. Montar
    // virtio-sound desbordaba la guard page -> #PF -> doble fallo (#DF). 1
    // MiB da ~50x de holgura sobre el frame mas profundo; sobre 256 MiB de
    // RAM el coste es nulo.
    config.kernel_stack_size = 1024 * 1024;
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

/// Deja una traza por el puerto serie (COM1) — la enruta QEMU a la terminal
/// donde se ejecuto `cargo run`. Diagnostico barato del arranque: cada hito del
/// `kernel_main` deja una linea, asi una caida muestra HASTA DONDE llego.
fn traza(rotulo: &str) {
    let _ = writeln!(baliza::Serie, "boot :: {rotulo}");
}

/// FASE 10 :: el molde de una aplicacion para los lanzamientos EN VIVO. Guarda
/// su bytecode —cacheado en RAM al arrancar, para no volver al disco despues—
/// y la geometria, cuota de memoria y presupuesto de combustible con que
/// instanciarla.
///
/// FASE 58 v9 :: `Clone` para poder copiar la plantilla afuera del lock de
/// `PLANTILLAS` antes de instanciarla — asi `nacer_ventana` no anida cerrojos.
/// El bytecode (Vec<u8>) son unos pocos KiB por app: el clon es barato.
#[derive(Clone)]
struct Plantilla {
    /// Nombre legible de la app — el del manifiesto, que la barra de tareas
    /// (Fase 14) muestra en la pestaña.
    nombre: alloc::string::String,
    bytecode: Vec<u8>,
    /// Hash del objeto-bytecode (= `EntradaApp.bytecode`). Lo guardamos porque
    /// la verificacion FRESH de la concesion (§14.1.3) en cada `Alt+N` necesita
    /// atar la firma a ESTE hash — `format::hash(&bytecode)` no sirve: la
    /// concesion cubre el hash del OBJETO del grafo, no el de los bytes crudos.
    bytecode_hash: format::Hash,
    nat_ancho: usize,
    nat_alto: usize,
    techo: usize,
    fuel: u64,
    /// Bitfield de permisos DECLARADOS, heredado del `EntradaApp`. Cada instancia
    /// que `Alt+N` engendra de esta plantilla nace con los mismos permisos
    /// del manifiesto: un clon no se gana, ni se pierde, capacidades.
    permisos: format::Permisos,
    /// Hash de la [`format::ConcesionCapacidad`] de esta app, o `None` (§14.1.3).
    /// Cada instanciacion re-verifica FRESH e intersecta con `permisos` para los
    /// permisos EFECTIVOS — la plantilla porta la concesion, no el veredicto.
    concesion: Option<format::Hash>,
}

/// Las plantillas de las apps lanzables. Se fundan en el arranque con la lista
/// del manifiesto; `Alt+N` instancia la siguiente en rotacion, `Alt+P + Enter`
/// instancia la elegida por indice.
///
/// FASE 58 v9 :: el `Mutex` permite agregar plantillas POST-BOOT — la
/// instalacion viva de apps que llegan via `mudanza` o `cronista` despues
/// del arranque (ver `instalar_app`).
static PLANTILLAS: Once<Mutex<Vec<Plantilla>>> = Once::new();

/// El cursor rotatorio sobre `PLANTILLAS`: que app nace en el proximo `Alt+N`.
static CURSOR_PLANTILLA: AtomicUsize = AtomicUsize::new(0);

/// Tarea cooperativa de una aplicacion WASM. En cada pulso del reloj le concede
/// un `tick` —un fotograma de trabajo— y cede la CPU hasta el siguiente; entre
/// medias corren sus vecinas. Si la app falla o agota su combustible, se la
/// DESALOJA: el compositor tatua su ventana con la baliza y la tarea concluye.
/// El ejecutor la retira del censo, su memoria se libera, el kernel sigue vivo.
async fn tarea_aplicacion(mut app: wasm::AplicacionWasm) {
    loop {
        async_system::reloj::EsperaFrame::nueva().await;
        // ¿El compositor pidio cerrar esta ventana (`Alt+Q`)? La tarea concluye
        // por su propia voluntad: al retornar, `AplicacionWasm` se libera —su
        // memoria lineal, su combustible, su canal de teclado— y el ejecutor la
        // retira del censo. Una baja LIMPIA, sin baliza (Fase 10).
        if compositor::ventana_cerrada(app.indice()) {
            return;
        }
        if let Err(falla) = app.tick() {
            // El color de la baliza delata la causa: purpura si agoto su tiempo
            // o aborto, amarillo si reviento su techo de memoria. El compositor
            // la pinta en el marco actual de la ventana y la marca como muerta.
            compositor::desalojar(app.indice(), falla.color_baliza());
            return;
        }
    }
}

/// FASE 8 :: la tarea del compositor. En cada fotograma drena la cola de mandos
/// que el teclado dejo —ciclar el modo de teselado, mover el foco— y los
/// aplica. Corre en el reactor cooperativo: el unico contexto donde es seguro
/// re-teselar el escritorio y recomponer el lienzo desde las caches.
async fn tarea_compositor() {
    loop {
        async_system::reloj::EsperaFrame::nueva().await;
        compositor::atender_mandos();
        // FASE 61 :: drenar la tableta virtio-input ANTES de atender el raton:
        // traduce sus eventos evdev a la posicion absoluta del puntero por el
        // mismo sumidero que el PS/2, de modo que `atender_raton` los procese
        // sin distinguir origen. No-op si no se monto tableta.
        drivers::tableta::atender();
        // FASE X3 :: polear el raton USB HID — drena su endpoint de interrupcion
        // sin bloquear y entrega los deltas al mismo sumidero que el PS/2 y la
        // tableta. No-op si no hay raton USB. Va ANTES de `atender_raton` para
        // que los clics de este fotograma se procesen en el acto.
        drivers::xhci::controlador::atender_raton_hid();
        // FASE 13 :: atender los eventos del raton (clic-para-enfocar y
        // arrastre de flotantes), y refrescar el puntero si se movio en una
        // vuelta tranquila en que ninguna app pinto.
        compositor::atender_raton();
        compositor::refrescar_puntero();
        // FASE 15 :: atender la voz del kernel — pasar a la nota siguiente
        // de la secuencia agendada, o silenciar al acabar.
        drivers::altavoz::atender();
        // FASE 16 :: avanzar el reloj de la barra de tareas — recompone si el
        // segundo cambio respecto al ultimo mostrado. Si no, vuelve enseguida.
        compositor::tick_reloj();
        // FASE 20 :: pulso del oficio AoE — el demultiplexor de RX en cada
        // tic (cero coste sin trafico), y el faro `AnunciarRaiz` cada 5 s
        // (medido contra el reloj monotono, no contra los awaits).
        akasha::tic_compositor();
        // FASE 10 :: atender las altas en vivo. Por cada `Alt+N` pendiente,
        // dar a luz una aplicacion nueva — el compositor solo conto la
        // peticion; instanciar el WASM es trabajo del orquestador.
        for _ in 0..compositor::partos_pendientes() {
            lanzar_app();
        }
        // FASE 58 :: partos DIRIGIDOS desde el launcher grafico. Cada indice
        // apunta a la plantilla N-esima del catalogo. Se drena despues de
        // los partos por rotacion para que un Alt+N tardio no canibalice
        // un Alt+P + Enter del mismo fotograma.
        for indice in compositor::partos_por_indice_pendientes() {
            lanzar_app_por_indice(indice);
        }
        // FASE 58 v10 :: polling del manifiesto vivo para apps recien
        // ancladas. Cada `INTERVALO_REFRESCO_APPS` fotogramas (~6 s a
        // 100 Hz) se relee el manifiesto del disco; si tiene mas entradas
        // que las `PLANTILLAS` actuales, las nuevas se instalan via
        // `instalar_app` y el launcher las refleja sin reboot. Si la
        // lectura falla, vuelve en silencio — el siguiente tic reintenta.
        let refresco = CONTADOR_REFRESCO_APPS.fetch_add(1, Ordering::Relaxed) + 1;
        if refresco >= INTERVALO_REFRESCO_APPS {
            CONTADOR_REFRESCO_APPS.store(0, Ordering::Relaxed);
            refrescar_apps_desde_manifiesto();
        }
        // FASE 24 :: tic ocioso del compactador semantico. Cuando el log ha
        // ganado bastantes escrituras desde la ultima pasada, aspirar los
        // nodos huerfanos. La operacion es BLOQUEANTE para la E/S de disco
        // pero ocurre una vez cada decenas de fotogramas, asi que el
        // estrangulamiento es invisible al usuario. Si falla, lo dejamos
        // en la traza serial y seguimos: el GC NO debe tumbar el compositor.
        if almacen::conviene_compactar() {
            match almacen::compactar() {
                Ok(stats) => {
                    let _ = writeln!(
                        baliza::Serie,
                        "gc :: compactado :: vivos={} muertos={} sectores={}->{}",
                        stats.nodos_vivos,
                        stats.nodos_muertos,
                        stats.sectores_antes,
                        stats.sectores_despues
                    );
                }
                Err(motivo) => {
                    let _ = writeln!(baliza::Serie, "gc :: compactar fallido :: {motivo}");
                }
            }
        }
    }
}

/// FASE 18 — saludo inicial a la red. La tarea es CORTA y de un solo tiro:
/// deja estabilizarse la cola RX del dispositivo y envia un ARP request al
/// gateway de QEMU para anunciarse en capa-3. El demuxer Akasha (Fase 20)
/// vive en el tic del compositor; el faro periodico, en `tarea_akasha_faro`.
async fn tarea_red(mac: drivers::red::Mac) {
    for _ in 0..10 {
        async_system::reloj::EsperaFrame::nueva().await;
    }
    let frame = drivers::red::componer_arp_request(
        mac,
        drivers::red::IP_RENASER,
        drivers::red::IP_GATEWAY,
    );
    match drivers::red::enviar(&frame) {
        Ok(()) => {
            let _ = writeln!(
                baliza::Serie,
                "red :: ARP REQUEST enviado :: ¿quien tiene 10.0.2.2?"
            );
        }
        Err(motivo) => {
            let _ = writeln!(baliza::Serie, "red :: envio fallido :: {motivo}");
        }
    }
}

/// FASE 20 — el faro Akasha. Cada `INTERVALO_FARO` fotogramas (= 5 s a
/// 100 Hz) difunde por broadcast `AnunciarRaiz(manifiesto)`. La primera
/// difusion se demora unos pocos fotogramas para que el grafo termine de
/// montarse — si `manifiesto()` aun es `None`, el envio es no-op y el
/// siguiente faro lo reintentara. Tarea sin fin, con la misma forma que
/// `tarea_compositor` (probada y latiente).
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

/// FASE 62 :: la tarea de sonido. En cada fotograma bombea el flujo de salida
/// de virtio-sound: recupera los periodos PCM que el dispositivo ya consumio y
/// rellena la tuberia (con audio si hay nota o tono de app, con silencio si
/// no), de modo que el flujo jamas cae en underrun. Solo se engendra si el
/// dispositivo se monto; el bombeo en si no bloquea —usa transferencia no
/// bloqueante—, asi que el escritorio sigue fluido mientras suena.
async fn tarea_sonido() {
    loop {
        async_system::reloj::EsperaFrame::nueva().await;
        drivers::sonido::bombear();
    }
}

/// FASE 67 / WAWA §14.1.3 :: resuelve los permisos EFECTIVOS de una app — la
/// INTERSECCION de lo que DECLARA (su `EntradaApp.permisos`) y lo que una
/// [`format::ConcesionCapacidad`] valida CONCEDE para su bytecode.
///
/// - `concesion == None`: no hay techo per-bytecode. Rigen los declarados — la
///   firma del manifiesto (`ManifiestoFirmado`/`RaizFirmada`) ya cubre el
///   bitfield, asi que es el camino legacy/escalonado.
/// - `concesion == Some(h)`: se recupera el objeto del grafo y se VERIFICA FRESH
///   contra el `AGORA_AUTH_RING`. Si esta ausente, corrupto, es para OTRO
///   bytecode, o lo firmo una llave ajena ⇒ `concedidos = 0` ⇒ cero capacidades
///   gateadas (FAIL-CLOSED: declarar una concesion y que sea invalida CIERRA, no
///   abre). El resultado nunca excede ni los declarados ni los concedidos.
///
/// FRESH en cada carga: ni cache ni TOCTOU. La concesion vive en el grafo
/// direccionado por contenido; un swap en disco entre arranque y `Alt+N` se
/// nota en la proxima instanciacion.
/// Flip del rollout escalonado → estricto (WAWA.md §14.1.3 / SDD-capacidades §3.6).
///
/// `false` (HOY, escalonado): una `EntradaApp` SIN concesión rige por sus
///   permisos declarados — la firma del manifiesto sigue gobernando. El génesis
///   bootea aunque el operador no haya sembrado concesiones todavía.
/// `true` (END-STATE estricto): sin concesión ⇒ CERO capacidades gateadas. Cierra
///   la escalada por re-firma de manifiesto incluso para apps que nunca
///   declararon concesión.
///
/// **No flipear a `true` hasta completar la ceremonia de génesis** (SDD §3.3:
/// firmar offline una `ConcesionCapacidad` por cada app génesis con permisos y
/// sembrarlas en `wawa-kernel/assets/concesiones/`). Flipear antes deja a
/// `mudanza`/`cronista`/`asistente`/etc. sin permisos en el próximo arranque.
const MODO_CAPACIDAD_ESTRICTO_GLOBAL: bool = false;

fn permisos_efectivos_de(
    declarados: format::Permisos,
    concesion: Option<&format::Hash>,
    bytecode: &format::Hash,
) -> format::Permisos {
    let Some(h) = concesion else {
        // Sin concesión: escalonado ⇒ rigen los declarados; estricto ⇒ cero.
        return if MODO_CAPACIDAD_ESTRICTO_GLOBAL { 0 } else { declarados };
    };
    let concedidos = match almacen::recuperar(h) {
        Ok(Some(objeto)) => match format::ConcesionCapacidad::deserializar(&objeto.datos) {
            Ok(c)
                if &c.bytecode == bytecode
                    && claves::verificar_concesion_capacidad(&c).is_ok() =>
            {
                c.permisos
            }
            _ => 0,
        },
        _ => 0,
    };
    format::permisos_efectivos(declarados, concedidos)
}

/// Da vida a una aplicacion del userspace a partir de su `EntradaApp` del
/// manifiesto: recupera su bytecode del grafo, lo carga en la ventana `indice`
/// del escritorio del compositor y despacha la app como tarea cooperativa del
/// reactor. Si el bytecode falta, esta corrupto, o la carga fracasa, el
/// compositor desaloja esa ventana — el kernel sigue con las demas.
fn encender_app(
    ejecutor: &mut Executor,
    indice: usize,
    entrada: &manifiesto::EntradaApp,
) -> Option<Plantilla> {
    // El tamaño NATURAL del lienzo de la app —lo que sabe pintar, fijo— lo
    // dicta su `EntradaApp`; el compositor decide en que marco lo coloca.
    let natural = manifiesto::region(entrada);
    // Recuperar el bytecode del grafo. `recuperar` recomputa el hash del
    // objeto y verifica su integridad: un bytecode corrupto se delata aqui
    // —y la app se niega, no se instancia un modulo en el que no se confia.
    let bytecode = match almacen::recuperar(&entrada.bytecode) {
        Ok(Some(objeto)) => objeto.datos,
        _ => {
            compositor::desalojar(indice, Color::DESALOJO);
            return None;
        }
    };
    // §14.1.3 :: los permisos EFECTIVOS con que se enlaza el `Linker` salen de
    // la INTERSECCION de los declarados con la concesion firmada — no de
    // `entrada.permisos` a secas. Sin concesion: rigen los declarados.
    let efectivos =
        permisos_efectivos_de(entrada.permisos, entrada.concesion.as_ref(), &entrada.bytecode);
    // `indice` es la identidad de la app: su ventana en el escritorio del
    // compositor y su ranura de estado persistido (Fase 7c).
    match wasm::AplicacionWasm::cargar(
        &bytecode,
        natural.ancho,
        natural.alto,
        entrada.techo_memoria as usize,
        entrada.fuel_fotograma as u64,
        indice,
        efectivos,
    ) {
        Ok(app) => ejecutor.spawn(tarea_aplicacion(app)),
        Err(_) => compositor::desalojar(indice, Color::DESALOJO),
    }
    // FASE 10 :: el bytecode, ya recuperado y verificado, queda como PLANTILLA:
    // un molde en RAM con que `Alt+N` instanciara copias en vivo, sin volver al
    // disco —que la E/S por sondeo en mitad del reactor seria un mal vecino—.
    Some(Plantilla {
        nombre: entrada.nombre.clone(),
        bytecode,
        bytecode_hash: entrada.bytecode,
        nat_ancho: natural.ancho,
        nat_alto: natural.alto,
        techo: entrada.techo_memoria as usize,
        fuel: entrada.fuel_fotograma as u64,
        permisos: entrada.permisos,
        concesion: entrada.concesion,
    })
}

/// FASE 10 :: da a luz una aplicacion EN VIVO. Elige la siguiente plantilla en
/// rotacion, abre su ventana en el compositor —que le asigna su indice—,
/// instancia su WASM con ese indice y engendra su tarea en el reactor ya en
/// marcha. Si la carga falla, la ventana recien nacida se desaloja; el kernel
/// sigue. La invoca la tarea del compositor al atender un `Alt+N`.
fn lanzar_app() {
    // Tomamos el lock SOLO para clonar la plantilla elegida; instanciar la
    // app vive afuera para evitar anidar el lock de `PLANTILLAS` con el del
    // escritorio (que toma `nacer_ventana` dentro de `instanciar_plantilla`).
    let plantilla = {
        let Some(mutex) = PLANTILLAS.get() else {
            return;
        };
        let plantillas = mutex.lock();
        if plantillas.is_empty() {
            return;
        }
        // El cursor rota sobre las plantillas: cada `Alt+N` engendra la siguiente.
        let cursor = CURSOR_PLANTILLA.fetch_add(1, Ordering::Relaxed);
        plantillas[cursor % plantillas.len()].clone()
    };
    instanciar_plantilla(&plantilla);
}

/// FASE 58 :: variante DIRIGIDA del lanzamiento — instancia la plantilla
/// `indice`-esima del manifiesto. La invoca el drenado de partos por indice,
/// que viene del launcher grafico tras pulsar Alt+Enter. Indice fuera de
/// rango: se ignora (un launcher con catalogo viejo).
fn lanzar_app_por_indice(indice: usize) {
    let plantilla = {
        let Some(mutex) = PLANTILLAS.get() else {
            return;
        };
        let plantillas = mutex.lock();
        let Some(plantilla) = plantillas.get(indice) else {
            return;
        };
        plantilla.clone()
    };
    instanciar_plantilla(&plantilla);
}

/// FASE 58 v9 :: instala una app POST-BOOT a partir de su `EntradaApp` —
/// recupera su bytecode del grafo, construye una `Plantilla` y la incorpora
/// a `PLANTILLAS`, refrescando luego el catalogo del launcher para que
/// `Alt+P` la vea de inmediato. NO instancia la app —solo la deja lanzable—:
/// el operador decide cuando pulsarla.
///
/// Devuelve el indice de la nueva plantilla (= indice del catalogo del
/// launcher), o `None` si el bytecode no esta en el grafo o `PLANTILLAS`
/// aun no fue fundado. La llamada llega del orquestador despues de un
/// re-ancla de manifiesto (via `mudanza`) o de un alta puntual via
/// Akasha/cronista; eventualmente se cableara automatica, hoy es API
/// publica esperando consumidor.
#[allow(dead_code)]
pub fn instalar_app(entrada: &manifiesto::EntradaApp) -> Option<usize> {
    let natural = manifiesto::region(entrada);
    let bytecode = match almacen::recuperar(&entrada.bytecode) {
        Ok(Some(objeto)) => objeto.datos,
        _ => return None,
    };
    let plantilla = Plantilla {
        nombre: entrada.nombre.clone(),
        bytecode,
        bytecode_hash: entrada.bytecode,
        nat_ancho: natural.ancho,
        nat_alto: natural.alto,
        techo: entrada.techo_memoria as usize,
        fuel: entrada.fuel_fotograma as u64,
        permisos: entrada.permisos,
        concesion: entrada.concesion,
    };
    let mutex = PLANTILLAS.get()?;
    let nuevo_idx;
    let nombres: Vec<alloc::string::String> = {
        let mut plantillas = mutex.lock();
        plantillas.push(plantilla);
        nuevo_idx = plantillas.len() - 1;
        plantillas.iter().map(|p| p.nombre.clone()).collect()
    };
    // Refrescar el catalogo del launcher AFUERA del lock de PLANTILLAS:
    // `fijar_catalogo` toma el cerrojo del escritorio y, si el launcher
    // esta abierto, dispara una recomposicion — no queremos esos locks
    // anidados con el de PLANTILLAS.
    compositor::fijar_catalogo(nombres);
    Some(nuevo_idx)
}

/// FASE 58 v10 :: contador de fotogramas para el polling del manifiesto vivo
/// (`refrescar_apps_desde_manifiesto`). Se incrementa en cada tic del
/// compositor; al alcanzar `INTERVALO_REFRESCO_APPS` se dispara el refresco
/// y vuelve a cero.
static CONTADOR_REFRESCO_APPS: AtomicUsize = AtomicUsize::new(0);

/// FASE 58 v10 :: cada cuantos fotogramas del compositor se relee el
/// manifiesto del disco. 600 fotogramas ≈ 6 s a 100 Hz (PIT) — bastante
/// frecuente para que el operador no espere, bastante raro para que la
/// lectura de disco no domine el tic del compositor.
const INTERVALO_REFRESCO_APPS: usize = 600;

/// FASE 58 v10 :: el consumidor automatico de `instalar_app`. Relee el
/// manifiesto del disco (el `sys_manifiesto_proponer` de `mudanza` re-ancla
/// el superbloque pero no toca `PLANTILLAS`); si el manifiesto del disco
/// tiene mas entradas que las plantillas vigentes, las nuevas se ingresan
/// via `instalar_app` y el launcher las refleja en su proximo `Alt+P`.
///
/// Tolerante a errores: si `manifiesto::cargar` falla (disco con problemas,
/// manifiesto corrupto), vuelve en silencio. El siguiente tic reintentara.
/// El protocolo NO retira plantillas instaladas — un manifiesto que ENCOGE
/// no anula apps activas; eso requeriria invalidar ventanas vivas y queda
/// como politica explicita futura.
fn refrescar_apps_desde_manifiesto() {
    let Ok(Some(manifiesto)) = manifiesto::cargar() else {
        return;
    };
    let plantillas_actuales = PLANTILLAS
        .get()
        .map(|m| m.lock().len())
        .unwrap_or(0);
    if manifiesto.apps.len() <= plantillas_actuales {
        return;
    }
    for entrada in &manifiesto.apps[plantillas_actuales..] {
        match instalar_app(entrada) {
            Some(idx) => {
                let _ = writeln!(
                    baliza::Serie,
                    "launcher :: app instalada en vivo :: idx={} nombre={}",
                    idx, entrada.nombre,
                );
            }
            None => {
                let _ = writeln!(
                    baliza::Serie,
                    "launcher :: instalacion fallida :: nombre={} (bytecode ausente?)",
                    entrada.nombre,
                );
            }
        }
    }
}

/// Camino comun de `lanzar_app` y `lanzar_app_por_indice`: abre la ventana,
/// carga el WASM y engendra la tarea — o tatua la baliza si la carga falla.
fn instanciar_plantilla(plantilla: &Plantilla) {
    // La ventana nace primero: el compositor le entrega su indice —su
    // identidad—, que el WASM necesita para hallar su ventana y su canal.
    let indice = compositor::nacer_ventana(plantilla.nat_ancho, plantilla.nat_alto, &plantilla.nombre);
    // §14.1.3 :: re-resuelve los permisos efectivos FRESH en cada parto — la
    // concesion se re-verifica contra el grafo y el anillo, no se cachea su
    // veredicto en la plantilla.
    let efectivos = permisos_efectivos_de(
        plantilla.permisos,
        plantilla.concesion.as_ref(),
        &plantilla.bytecode_hash,
    );
    match wasm::AplicacionWasm::cargar(
        &plantilla.bytecode,
        plantilla.nat_ancho,
        plantilla.nat_alto,
        plantilla.techo,
        plantilla.fuel,
        indice,
        efectivos,
    ) {
        // La tarea se ENGENDRA, no se hace `spawn`: el reactor ya corre y el
        // ejecutor la adoptara en su proxima vuelta (Fase 10).
        Ok(app) => async_system::executor::engendrar(Box::pin(tarea_aplicacion(app))),
        Err(_) => compositor::desalojar(indice, Color::DESALOJO),
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
/// Genesis que `boot` sembro en la imagen de disco, lo instala como el
/// manifiesto VIVO del kernel y, por cada `EntradaApp`, enciende su
/// aplicacion. Toda falla se reporta a la consola y NO detiene el arranque: el
/// kernel se levanta con las apps que pueda — o con ninguna, si el grafo no
/// tiene userspace.
fn cargar_userspace(ejecutor: &mut Executor, ancho_pantalla: usize, alto_pantalla: usize) {
    let manifiesto = match manifiesto::cargar() {
        Ok(Some(m)) => Some(m),
        // Disco sin manifiesto anclado: `boot` no lo sembro. El kernel se
        // levanta sin userspace —pero se levanta—; en la practica, ninguna
        // imagen forjada por `boot` llega aqui sin su Manifiesto de Genesis.
        Ok(None) => {
            reportar("manifiesto :: el disco no trae uno -- el kernel se levanta solo");
            None
        }
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
        // Instalar el manifiesto VIVO ANTES de instanciar las apps: el `init`
        // de cada app puede consultar su estado persistido (Fase 7c), y esa
        // consulta lee del manifiesto vivo.
        manifiesto::instalar(m.clone());

        // Aplicar el overlay de revocación que el manifiesto ancla (si ancla
        // uno) ANTES de aceptar propuesta soberana alguna: enciende los slots
        // del AGORA_AUTH_RING revocados por quórum, de modo que una clave
        // soberana filtrada quede denegada ya en este arranque, sin esperar al
        // reflash (SDD-rotacion-revocacion §4).
        let slots_revocados = manifiesto::aplicar_overlay();
        if slots_revocados > 0 {
            reportar(&format!(
                "claves :: overlay de revocacion activo -- {slots_revocados} slot(s) del anillo denegado(s)",
            ));
        }

        // FASE 8 :: fundar el escritorio del compositor — una ventana por app,
        // con su cache de respaldo y su marco teselado por `mirada-layout`— y
        // pintar el escenario antes de encender las apps: el teselado se ve
        // aunque alguna app no llegue a pintar su primer fotograma.
        let naturales: Vec<(usize, usize, &str)> = m
            .apps
            .iter()
            .map(|e| (e.region_ancho as usize, e.region_alto as usize, e.nombre.as_str()))
            .collect();
        // FASE 59 v1 :: fundar el registro de outputs ANTES del escritorio,
        // de modo que el compositor pueda consultar `pantallas::primario()`
        // si necesita. Hoy hay UN output cubriendo todo el framebuffer —sea el
        // scanout virtio-gpu que el kernel gobierna (Fase 60) o el GOP del
        // firmware—; el multi-scanout (`num_scanouts > 1`) registrara los
        // adicionales con `pantallas::registrar` cuando se aborde.
        pantallas::fundar(ancho_pantalla, alto_pantalla);
        compositor::fundar(ancho_pantalla, alto_pantalla, &naturales);
        compositor::componer_escenario();

        let mut plantillas: Vec<Plantilla> = Vec::new();
        for (indice, entrada) in m.apps.iter().enumerate() {
            if let Some(plantilla) = encender_app(ejecutor, indice, entrada) {
                plantillas.push(plantilla);
            }
        }
        // FASE 10 :: fijar las plantillas de las apps. A partir de aqui, cada
        // `Alt+N` instancia una copia viva, en rotacion.
        // FASE 58 v9 :: PLANTILLAS vive ahora en un Mutex — `instalar_app`
        // agrega plantillas post-boot via push, sin necesidad de re-funder.
        PLANTILLAS.call_once(|| Mutex::new(plantillas));

        // FASE 58 :: poblar el catalogo del launcher con los nombres de las
        // plantillas YA fijadas. El indice de cada nombre coincide con el de
        // su plantilla — el orquestador resuelve `partos_por_indice` por esa
        // posicion. Si no hay plantillas (manifiesto sin apps), el catalogo
        // queda vacio y el launcher se cierra solo en Alt+Enter.
        if let Some(mutex) = PLANTILLAS.get() {
            let nombres: Vec<alloc::string::String> = {
                let plantillas = mutex.lock();
                plantillas.iter().map(|p| p.nombre.clone()).collect()
            };
            compositor::fijar_catalogo(nombres);
        }

        // La tarea del compositor: atiende los mandos del teclado —ciclar el
        // teselado, mover el foco, cerrar y lanzar apps— en cada fotograma.
        ejecutor.spawn(tarea_compositor());
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
    // FASE 60 :: delatar quien gobierna el barrido de pantalla.
    if drivers::gpu::disponible() {
        let n = drivers::gpu::cabezas();
        if n > 1 {
            consola.escribir(&format!(
                "gpu :: virtio-gpu -- el kernel gobierna {n} scanouts (multi-monitor)\n"
            ));
        } else {
            consola.escribir("gpu :: virtio-gpu -- el kernel gobierna el scanout\n");
        }
    } else {
        consola.escribir("gpu :: ausente -- escritorio sobre el framebuffer GOP\n");
    }
    consola.presentar();
}

// =============================================================================
//  PUNTO DE ENTRADA DEL KERNEL
// =============================================================================

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    traza("kernel_main entrado");
    // --- 1. Recuperar el framebuffer GOP que el firmware nos confio. ---
    let framebuffer = match boot_info.framebuffer.as_mut() {
        Some(fb) => fb,
        None => detener(),
    };
    let info: FrameBufferInfo = framebuffer.info();
    let format: PixelFormat = info.pixel_format;
    // La resolucion del lienzo intermedio se acota a Full HD y se fija YA: la
    // GPU (Fase 60) crea su scanout a esta misma medida, antes de la consola.
    let ancho_lienzo = info.width.min(ANCHO_MAX);
    let alto_lienzo = info.height.min(ALTO_MAX);
    // `pantalla` arranca sobre el framebuffer GOP del firmware; si la Fase 60
    // logra montar virtio-gpu, se reemplaza por el scanout que el kernel posee.
    let mut pantalla = Pantalla::adoptar(framebuffer, info);
    traza("framebuffer adoptado");

    // Datos para la sonda de disco (Fase 6.1b): el offset al que el cargador
    // mapeo la memoria fisica y la mayor region de RAM libre para el DMA.
    let offset_fisico = boot_info.physical_memory_offset.into_option();
    let region_dma = mayor_region_usable(&boot_info.memory_regions);
    let _ = writeln!(
        baliza::Serie,
        "boot :: physical_memory_offset={:#x?} region_dma={:#x?}",
        offset_fisico,
        region_dma,
    );

    // --- 1.5. CAPA R :: ramdisk. Cuando `wawa-boot` forja la imagen con
    //          `set_ramdisk(disk.img)`, el cargador carga el grafo entero en
    //          RAM y nos pasa su direccion+longitud por `BootInfo.ramdisk_*`.
    //          Lo registramos AQUI, antes de `informar_almacen`, para que
    //          `drivers::disco::montar` derive al slice en vez de sondear el bus
    //          PCI (no hay virtio-blk en metal). Si el cargador no embebio
    //          ramdisk (camino QEMU), `ramdisk_addr` es None y el almacen sigue
    //          contra virtio-blk con persistencia. El slice es `'static`: el
    //          cargador no reusa esa RAM y nadie la libera. ---
    if let Some(addr) = boot_info.ramdisk_addr.into_option() {
        let len = boot_info.ramdisk_len as usize;
        if len > 0 {
            // SEGURIDAD: el cargador mapeo `[addr, addr+len)` y garantiza que
            // perdura mientras viva el kernel; nadie mas escribe ese rango.
            let datos = unsafe { core::slice::from_raw_parts(addr as *const u8, len) };
            drivers::disco::establecer_ramdisk(datos);
            let _ = writeln!(
                baliza::Serie,
                "ramdisk :: registrado :: addr={:#x} len={} ({} sectores)",
                addr,
                len,
                len / 512,
            );
        }
    }

    // --- 2. Encender la baliza: la red de seguridad visual va primero. ---
    BALIZA_PANICO.encender(
        &pantalla,
        codificar(format, Color::ALERTA),
        codificar(format, Color::OOM),
        codificar(format, Color::FATAL_CARMESI),
    );
    traza("baliza encendida (alerta + oom + carmesi)");

    // --- 3. Cimientos de fallos e interrupciones (Fases 2.0 y 2.1). ---
    gdt::init();
    interrupts::init();
    pic::init();
    // Fase 38 :: COM1 polling para el canal del firmador externo. Sin IRQ,
    // sin asignador — solo configurar el UART una vez al boot.
    drivers::serial::init();
    traza("gdt + idt + pic + serial");

    // --- 4. FASE 3 :: fundar el heap. A partir de aqui, `alloc` esta vivo. ---
    memory::init();
    traza("heap fundado");

    // --- 4.5. Mapeador de MMIO: envuelve la tabla L4 activa para abrir paginas
    //          nuevas hacia los BAR MMIO de virtio (que pueden caer fuera de
    //          lo que el cargador mapeo). Necesita `physical_memory_offset`
    //          para alcanzar la L4 via el mapeo de memoria fisica. ---
    if let Some(offset) = offset_fisico {
        memory::mmio::init(offset);
        traza("mmio :: mapeador fundado");

        // --- 4.5b. Framebuffer Write-Combining en metal real. El bootloader
        //          mapea el GOP como WB-cached; en hardware fisico eso causa
        //          parpadeo (el scanout LCD ve pixeles viejos hasta el evict de
        //          cache) y lentitud (la polucion de cache mata al resto del
        //          kernel). Reprogramar IA32_PAT (slot 4 = WC) y remarcar las PTE
        //          del framebuffer lo cura. En QEMU es inocuo: el "framebuffer"
        //          es RAM coherente. Va ANTES de cualquier `presentar()` y
        //          mientras `pantalla` aun es el GOP — si virtio-gpu toma el
        //          scanout despues (Fase 60), esa memoria es DMA del kernel y no
        //          necesita el remap. Ver memory/cache.rs. ---
        memory::cache::init_pat();
        memory::cache::marcar_wc(pantalla.base as u64, info.byte_len);
        traza("cache :: framebuffer GOP remarcado WC");
    }

    // FASE 64 :: dimensiones del LIENZO GLOBAL (envolvente de todos los
    // monitores) y monitores SECUNDARIOS. Por defecto un solo output del tamaño
    // del lienzo per-scanout; el bloque multi-scanout de abajo los reescribe si
    // virtio-gpu reporta mas de una cabeza.
    let mut ancho_global = ancho_lienzo;
    let mut alto_global = alto_lienzo;
    let mut extras: Vec<Pantalla> = Vec::new();

    // --- 4.6. FASE 60 :: fundar la arena de marcos DMA y, sobre ella, TOMAR
    //          POSESION DEL SCANOUT via virtio-gpu. La arena —que el disco
    //          fundaba en su `init`— se adelanta aqui porque `KernelHal::
    //          dma_alloc` debe servir tambien a la GPU, que se monta antes que
    //          el disco y antes que la consola: su framebuffer SERA el de ella.
    //          Si falta offset/region, no hay dispositivo, o cualquier paso
    //          falla, el kernel se queda con el framebuffer GOP del firmware —el
    //          escritorio sigue, solo que el kernel no gobierna el barrido—. ---
    if let (Some(offset), Some((inicio, fin))) = (offset_fisico, region_dma) {
        drivers::disco::init(offset, inicio, fin);
        traza("disco :: arena DMA fundada");
        match drivers::gpu::montar(ancho_lienzo, alto_lienzo) {
            Ok(infos) if !infos.is_empty() => {
                // El primer scanout es el PRIMARIO, anclado en el origen del
                // lienzo global. La `pantalla` que la consola tomara es esta.
                let primario = &infos[0];
                pantalla = Pantalla::sobre_framebuffer(
                    primario.base,
                    primario.ancho,
                    primario.alto,
                    primario.paso_bytes,
                );
                // Re-apuntar la baliza al framebuffer de la GPU primaria: el
                // scanout que el operador ve es ahora el del kernel, no el GOP.
                BALIZA_PANICO.encender(
                    &pantalla,
                    codificar(pantalla.format, Color::ALERTA),
                    codificar(pantalla.format, Color::OOM),
                    codificar(pantalla.format, Color::FATAL_CARMESI),
                );
                // FASE 64 :: disponer los scanouts en el espacio compuesto (en
                // fila, izquierda→derecha) con la matematica pura ya probada de
                // `mirada-layout::outputs`. El lienzo global pasa a ser la
                // ENVOLVENTE; cada cabeza secundaria recibe una `Pantalla` con su
                // origen global, que blittea solo su sub-region.
                let tamanos: alloc::vec::Vec<(i32, i32)> = infos
                    .iter()
                    .map(|i| (i.ancho as i32, i.alto as i32))
                    .collect();
                let rects = mirada_layout::outputs::disponer(
                    &tamanos,
                    mirada_layout::outputs::Disposicion::Horizontal,
                );
                let env = mirada_layout::outputs::envolvente(&rects);
                ancho_global = env.w as usize;
                alto_global = env.h as usize;
                for (i, info) in infos.iter().enumerate().skip(1) {
                    let r = &rects[i];
                    extras.push(
                        Pantalla::sobre_framebuffer(
                            info.base,
                            info.ancho,
                            info.alto,
                            info.paso_bytes,
                        )
                        .con_origen(r.x as usize, r.y as usize),
                    );
                }
                // Con multi-scanout, fundar YA el registro de outputs (primario +
                // extras) que el compositor consulta para teselar por-monitor. Es
                // idempotente: el `pantallas::fundar` del arranque de userspace ve
                // el registro fundado y no lo pisa. Con un solo scanout NO se
                // funda aqui — lo hace el camino de siempre con las dims primarias.
                if infos.len() > 1 {
                    let regiones: alloc::vec::Vec<RegionPantalla> = rects
                        .iter()
                        .map(|r| RegionPantalla {
                            x: r.x as usize,
                            y: r.y as usize,
                            ancho: r.w as usize,
                            alto: r.h as usize,
                        })
                        .collect();
                    pantallas::fundar_outputs(&regiones);
                    let _ = writeln!(
                        baliza::Serie,
                        "gpu :: {} scanouts -> escritorio compuesto {}x{}",
                        infos.len(),
                        ancho_global,
                        alto_global,
                    );
                }
                traza("gpu :: scanout(s) en posesion del kernel");
                // FASE 62 :: subir el cursor por hardware sobre el scanout
                // primario. El puntero pasa a un plano que el host compone: moverlo
                // ya no fuerza un volcado de pantalla entera —la cura del lag—. Su
                // punto caliente es el vertice noroeste de la flecha (0,0). Si el
                // dispositivo lo rechaza, recae al estampado por software.
                match drivers::gpu::instalar_cursor(&grafico::cursor_bgra_64(), 0, 0) {
                    Ok(()) => traza("gpu :: cursor por hardware vivo"),
                    Err(motivo) => {
                        let _ = writeln!(baliza::Serie, "gpu :: cursor hw omitido :: {motivo}");
                        traza("gpu :: cursor por hardware OMITIDO (estampado software)");
                    }
                }
            }
            Ok(_) => {
                // `montar` devolvio una lista vacia (no deberia ocurrir): se
                // trata como ausencia de dispositivo — el escritorio sigue sobre
                // el GOP del firmware con un solo output.
                let _ = writeln!(baliza::Serie, "gpu :: sin scanouts utiles (fallback GOP)");
                traza("gpu :: OMITIDO (sin scanouts)");
            }
            Err(motivo) => {
                let _ = writeln!(baliza::Serie, "gpu :: {motivo} (fallback framebuffer GOP)");
                traza("gpu :: OMITIDO (fallback GOP)");
            }
        }
    }

    // --- 5. Con el heap activo, fundar lo que depende de el: el canal de
    //        scancodes, el reloj de fotogramas y la tipografia vectorial. ---
    async_system::teclado::init();
    async_system::puntero::init();
    async_system::reloj::init();
    texto::init();
    traza("teclado + puntero + reloj + texto");

    // --- 6. Construir el lienzo y la consola; pintar el rotulo inicial,
    //        ya rasterizado por fontdue, y publicar la consola globalmente. ---
    let memoria = match reclamar_memoria_lienzo() {
        Some(m) => m,
        None => detener(),
    };
    // El lienzo codifica sus pixeles al format de la pantalla ACTIVA: si la
    // GPU tomo el scanout es B8G8R8A8 (Bgr), si no, el del GOP. Asi el volcado
    // fila-a-fila es un `memcpy` puro, sin recodificar canal por canal.
    // FASE 64 :: el lienzo es GLOBAL — cubre la envolvente de todos los
    // monitores (con uno solo, == dims per-scanout de siempre). La composicion
    // pinta sobre el en coords globales; cada `Pantalla` blittea su sub-region.
    let mut lienzo = Lienzo::nuevo(memoria, ancho_global, alto_global, pantalla.format);
    lienzo.limpiar(Color::LIENZO_EN_REPOSO);

    let mut consola = Consola::nueva(lienzo, pantalla);
    // FASE 64 :: entregarle los monitores secundarios. Vacio con un solo output.
    consola.fijar_pantallas_extra(extras);
    // FASE 64 :: el texto de la consola se envuelve al ancho del PRIMARIO
    // (`ancho_lienzo`), no al del lienzo global — si no, se derrama al monitor 2.
    consola.fijar_ancho_util(ancho_lienzo);
    consola.escribir("renaser :: fase 6.2 -- E/S de disco asincrona por interrupcion\n");
    consola.presentar();
    CONSOLA.call_once(|| Mutex::new(consola));
    traza("consola publicada");

    // --- 6.5. FASE 6.1c :: fundar el subsistema de disco y, sobre el, el grafo
    //          de objetos: enumerar el bus PCI, montar el transporte virtio-blk,
    //          y leer o forjar el superbloque del almacen direccionado por
    //          contenido. El kernel adquiere, por fin, una memoria que perdura. ---
    match (offset_fisico, region_dma) {
        (Some(_), Some(_)) => {
            // La arena de marcos DMA ya se fundo en el paso 4.6 (la GPU la
            // necesitaba antes); aqui solo montamos el disco fisico y, sobre
            // el, el grafo de objetos direccionado por contenido.
            traza("almacen :: init");
            informar_almacen();
            traza("almacen :: listo");
        }
        _ => {
            if let Some(consola) = CONSOLA.get() {
                let mut consola = consola.lock();
                consola.escribir("virtio-blk :: omitido -- memoria fisica sin mapear\n");
                consola.presentar();
            }
            traza("disco :: OMITIDO (sin offset/region)");
        }
    }

    // --- 6.6. FASE 13 :: despertar el raton PS/2. El 8042 enciende su
    //          dispositivo auxiliar, el raton empieza a reportar, y el PIC
    //          desenmascara su IRQ12. Desde aqui hay un puntero en pantalla,
    //          y los clics pueden alcanzar al compositor.
    // FASE 64 :: el puntero se acota al ESCRITORIO GLOBAL (envolvente), no a un
    // solo monitor — asi puede cruzar de un scanout al vecino.
    drivers::raton::init(ancho_global, alto_global);
    traza("raton :: listo");

    // --- 6.65. FASE 61 :: montar la tableta virtio-input — un puntero ABSOLUTO
    //           que sigue 1:1 al cursor del host, sin la deriva del raton
    //           relativo PS/2. COMPLEMENTA al raton: si no hay tableta, `montar`
    //           devuelve `Err` y el puntero sigue siendo el del PS/2. Se sondea
    //           en cada fotograma desde la tarea del compositor, no por IRQ.
    match drivers::tableta::montar(ancho_global, alto_global) {
        Ok(()) => reportar("tableta :: virtio-input -- puntero absoluto"),
        Err(motivo) => reportar(&format!("tableta :: {motivo} -- puntero PS/2 relativo")),
    }
    traza("tableta :: listo");

    // --- 6.7. FASE 18 :: montar la tarjeta virtio-net. Si el firmware no
    //          enruta una linea de IRQ util o no hay dispositivo, el resto
    //          del arranque sigue — la red NO es critica.
    let mac_red = drivers::red::montar();
    match mac_red {
        Ok(mac) => {
            let _ = writeln!(
                baliza::Serie,
                "red :: virtio-net :: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} :: IRQ {:?}",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
                drivers::red::irq(),
            );
            // FASE 20 :: registrar la MAC en el respondedor Akasha, para que
            // pueda firmar sus frames AoE.
            akasha::montar(mac);
        }
        Err(motivo) => {
            let _ = writeln!(baliza::Serie, "red :: virtio-net :: {motivo}");
        }
    }
    traza("red :: listo");

    // --- 6.8. FASE 49 :: montar virtio-console — el canal del firmador
    //          externo de alta velocidad. Si el firmware no expone un
    //          virtconsole, `montar` devuelve `Err` y la syscall de
    //          firma cae al UART de la Fase 38: la boot story sigue
    //          intacta. La virtio-console NO es critica.
    match drivers::consola_virtio::montar() {
        Ok(()) => {
            let _ = writeln!(baliza::Serie, "consola :: virtio-console :: montado");
        }
        Err(motivo) => {
            let _ = writeln!(
                baliza::Serie,
                "consola :: virtio-console :: {motivo} (fallback UART)"
            );
        }
    }
    traza("consola_virtio :: listo");

    // --- 6.9. FASE 62 :: montar virtio-sound. Si lo hay, la voz del kernel y
    //          el `sys_tono` de las apps suenan como PCM real por DMA; si no,
    //          `altavoz` recae en la bocina del PIT. NO es critico: un fallo de
    //          montaje deja el sonido en la bocina y el arranque sigue. ---
    match drivers::sonido::montar() {
        Ok(()) => reportar("sonido :: virtio-sound -- PCM real (la bocina del PIT en reposo)"),
        Err(motivo) => {
            let _ = writeln!(baliza::Serie, "sonido :: {motivo} (fallback bocina del PIT)");
        }
    }
    traza("sonido :: listo");

    // --- 6.95. FASE X2/X3 :: montar el controlador USB (XHCI) y, si hay un
    //           raton USB HID conectado, configurarlo. Para maquinas cuyo UEFI
    //           NO emula el raton USB sobre el i8042 (el trackpad PS/2 anda pero
    //           el raton USB no llega): este es el camino NATIVO al puntero. NO
    //           es critico — sin XHCI o sin raton USB el arranque sigue con
    //           PS/2/tableta. El raton USB se polea por fotograma desde el
    //           reactor (`atender_raton_hid`), aun no por IRQ. ---
    match drivers::xhci::controlador::montar() {
        Ok(caps) => {
            let _ = writeln!(
                baliza::Serie,
                "xhci :: montado :: version={:#06x} slots={} puertos={}",
                caps.version, caps.max_slots, caps.max_puertos,
            );
            // A PANTALLA: en metal sin COM1 esto es lo unico visible. Dice que
            // dispositivos USB vio el kernel y si monto el raton.
            for linea in drivers::xhci::controlador::resumen_usb() {
                reportar(&linea);
            }
        }
        Err(motivo) => {
            reportar(&format!("usb :: {motivo}"));
            let _ = writeln!(baliza::Serie, "xhci :: {motivo} (sin USB nativo)");
        }
    }
    traza("xhci :: listo");

    // PAUSA DE LECTURA (depuración USB en metal sin COM1): el compositor pintará
    // sus ventanas encima del log apenas arranque el reactor, tapando las líneas
    // `usb ...`. Aquí —antes del reactor, con las IRQs aún apagadas— un spin
    // acotado mantiene el log en pantalla ~10s para poder leerlo. Es temporal
    // mientras cazamos el raton USB; se quita despues. (OJO: `spin_loop`/PAUSE
    // cuesta ~100 ciclos; 3e8 ≈ 10s a GHz — NO subir a miles de millones.)
    reportar(">> PAUSA ~10s, SIGUE SOLA — lee las lineas 'usb ...' de arriba <<");
    for _ in 0..300_000_000u32 {
        core::hint::spin_loop();
    }

    // --- 7. FASE 7 :: levantar el reactor y poblar el userspace DESDE EL
    //        GRAFO. El kernel ya no empotra los modulos WASM: lee el
    //        Manifiesto de Genesis que `boot` sembro en la imagen de disco e
    //        instancia cada `EntradaApp` recuperando su bytecode del grafo de
    //        objetos. Las cinco apps de genesis (dos instancias de hello, la
    //        discola, la glotona y la cronista) nacen del disco, no del
    //        binario del kernel.
    //
    //        Las interrupciones se habilitan AHORA: el temporizador marcara el
    //        compas de los fotogramas y la IRQ del teclado difundira cada
    //        scancode a los canales que las apps consultan. ---
    let mut ejecutor = Executor::nuevo();
    traza("ejecutor :: creado");
    cargar_userspace(&mut ejecutor, ancho_lienzo, alto_lienzo);
    traza("userspace :: cargado");
    // FASE 6.2 :: una tarea mas del reactor — no una app WASM— que sondea el
    // disco de forma ASINCRONA: la demostracion de que la IRQ del disco
    // conduce la E/S sin detener a las aplicaciones visuales.
    ejecutor.spawn(tarea_sonda_disco());
    // FASE 18 :: si la tarjeta de red se monto, una tarea corta envia un ARP
    // al gateway para anunciarse en capa-3. El oficio AoE (Fase 20) — demuxer
    // de entrada + faro periodico — va clavado al tic del compositor.
    if let Ok(mac) = mac_red {
        ejecutor.spawn(tarea_red(mac));
    }
    // FASE 62 :: si hay virtio-sound, una tarea del reactor bombea su flujo de
    // salida en cada fotograma —recupera periodos consumidos y rellena la
    // tuberia con PCM (audio o silencio)—. Sin dispositivo, no se engendra: la
    // voz del kernel suena por la bocina del PIT como hasta la Fase 61.
    if drivers::sonido::disponible() {
        ejecutor.spawn(tarea_sonido());
    }
    // FASE 63 :: si hay virtio-console, una tarea del reactor escucha el canal
    // de control host->kernel (`wawactl gc`). Sin dispositivo, no se engendra:
    // la palanca de compactacion sigue viva por `Alt+G` y por el tic ocioso.
    if drivers::consola_virtio::montada() {
        ejecutor.spawn(control::tarea_consola_control());
    }
    // FASE 15 :: la voz del sistema da los buenos dias con un acorde de Do
    // mayor. La tarea del compositor lo hara sonar nota a nota una vez que
    // el reactor arranque y las interrupciones empiecen a llegar. La
    // bienvenida suena incluso sin tarjeta de sonido (bocina PIT): es el
    // pitido fundacional, no una voz frecuente silenciable.
    drivers::altavoz::agendar_bienvenida();
    traza("ejecutor :: arrancando reactor");
    x86_64::instructions::interrupts::enable();
    ejecutor.run();
}
