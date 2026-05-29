// =============================================================================
//  renaser :: kernel/src/drivers/gpu.rs — Fase 60 :: el kernel toma el scanout
// -----------------------------------------------------------------------------
//  Hasta la Fase 59 el escritorio vivia sobre el framebuffer LINEAL que el
//  firmware UEFI (GOP) dejaba tras de si: una ventana fija, de resolucion
//  ajena, que el kernel solo sabia rellenar. La Fase 60 le da posesion del
//  scanout. Con el MISMO patron del disco y la red —enumerar PCI, montar el
//  transporte moderno de virtio, ceder a `virtio-drivers` el dialogo de bajo
//  nivel— renaser reclama un dispositivo `virtio-gpu` y crea su PROPIO recurso
//  2D, lo enlaza a una region DMA y lo fija como scanout primario.
//
//  El modelo de presentacion cambia de naturaleza. Sobre el GOP, escribir un
//  pixel ERA presentarlo: el firmware barria esa memoria continuamente. Sobre
//  virtio-gpu el framebuffer es memoria del HUESPED; pintar ahi no se ve hasta
//  que un `flush` —`TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH`— lo transfiere al
//  anfitrion y lo vuelca a la pantalla. Por eso `consola::presentar` cierra
//  cada fotograma con `gpu::presentar()`: el doble bufer se vuelca al lienzo,
//  el lienzo al framebuffer DMA, y el `flush` cruza la frontera hacia el host.
//
//  Reutiliza `KernelHal` (el puente DMA del disco) y `CamPuertos` (el acceso
//  PCI). Si no hay dispositivo, o cualquier paso falla, `montar` devuelve `Err`
//  SIN tocar el estado: el kernel se queda con el framebuffer GOP del firmware
//  —el escritorio sigue, solo que no lo gobierna el kernel—.
// =============================================================================

use core::sync::atomic::{AtomicBool, Ordering};

use spin::{Mutex, Once};
use virtio_drivers::device::gpu::VirtIOGpu;
use virtio_drivers::transport::pci::bus::{Command, DeviceFunction, PciRoot};
use virtio_drivers::transport::pci::PciTransport;

use super::disco::KernelHal;
use super::pci::CamPuertos;

/// Vendor ID de VirtIO; Device ID de un adaptador grafico. `virtio-gpu` es un
/// dispositivo de la era virtio-1.0 (sin gemelo «transicional» legacy): su
/// unico ID es el moderno `0x1040 + 16 = 0x1050`. `virtio-vga` —el mismo nucleo
/// con compatibilidad VGA, que es como QEMU sirve un GOP para el arranque—
/// expone tambien `0x1050`.
const VENDOR_VIRTIO: u16 = 0x1AF4;
const VIRTIO_GPU_IDS: [u16; 1] = [0x1050];

/// El adaptador grafico, ya montado. Envuelve al `VirtIOGpu` para que pueda
/// vivir en un `static` tras el `Mutex` global.
struct Gpu(VirtIOGpu<KernelHal, PciTransport>);

// SEGURIDAD: `Gpu` encierra punteros crudos a las colas virtio, al MMIO del
// dispositivo y a la region DMA del framebuffer. renaser es un kernel de un
// solo nucleo y todo acceso al adaptador se serializa tras el `Mutex` global
// `GPU`; jamas se comparte entre hilos reales. No hay manejador de IRQ que lo
// dispute: la presentacion es sincrona, guiada por el reactor cooperativo.
unsafe impl Send for Gpu {}

/// El adaptador global de renaser. Se monta una sola vez, en `montar`.
static GPU: Once<Mutex<Gpu>> = Once::new();

/// ¿Esta vivo el cursor por HARDWARE? Lo enciende `instalar_cursor` con exito.
/// Mientras este `true`, el compositor NO estampa el puntero por software en el
/// framebuffer (lo compone el host en un plano aparte) y los movimientos del
/// raton viajan por `mover_cursor` —un comando diminuto en la cola de cursor—
/// en vez de forzar un `presentar` de pantalla entera.
static CURSOR_HW: AtomicBool = AtomicBool::new(false);

/// Lo que `montar` entrega al kernel para que construya su `Pantalla` sobre el
/// scanout que ahora gobierna: el framebuffer DMA del huesped y su geometria.
/// El format es B8G8R8A8 (BGRA, 4 bytes por pixel) — el unico que el recurso 2D
/// de virtio-gpu admite—; los bytes salen B,G,R,A, justo lo que codifica
/// `PixelFormat::Bgr` (la A queda en 0, que el scanout ignora).
pub struct InfoGpu {
    /// Ancho del scanout, en pixeles.
    pub ancho: usize,
    /// Alto del scanout, en pixeles.
    pub alto: usize,
    /// Direccion virtual del primer byte del framebuffer DMA. Valida durante
    /// toda la vida del kernel: el `Dma` que la respalda vive en el `VirtIOGpu`
    /// que `GPU` retiene para siempre.
    pub base: *mut u8,
    /// Bytes entre el inicio de una fila y la siguiente: `ancho * 4`, sin
    /// relleno (el recurso 2D de virtio-gpu es compacto).
    pub paso_bytes: usize,
}

/// Enumera el bus PCI, localiza el primer `virtio-gpu`, monta su transporte
/// moderno y crea un recurso 2D de `ancho`×`alto` enlazado a memoria DMA, que
/// fija como scanout primario. Deja el adaptador tras el `Mutex` global y
/// devuelve la `InfoGpu` con que el kernel construye su `Pantalla`. Toda falla
/// —ausencia de dispositivo, transporte indomito, recurso rechazado— se
/// devuelve como `Err` sin dejar estado a medias: el llamante recae al GOP.
///
/// `ancho`/`alto` se piden EXPLICITOS (no se hereda la resolucion por defecto
/// del display) para que el framebuffer del scanout calce exacto con el lienzo
/// intermedio del kernel — asi `presentar` blittea fila a fila sin reescalar.
pub fn montar(ancho: usize, alto: usize) -> Result<InfoGpu, &'static str> {
    let mut raiz = PciRoot::new(CamPuertos);

    // 1. Localizar el primer virtio-gpu recorriendo el bus.
    let mut hallado: Option<DeviceFunction> = None;
    'busqueda: for bus in 0..=255u8 {
        for (device_function, info) in raiz.enumerate_bus(bus) {
            if info.vendor_id == VENDOR_VIRTIO && VIRTIO_GPU_IDS.contains(&info.device_id) {
                hallado = Some(device_function);
                break 'busqueda;
            }
        }
    }
    let device_function = hallado.ok_or("virtio-gpu no hallado en el bus PCI")?;

    // 2. Habilitar E/S, espacio de memoria y BUS-MASTER. Sin bus-master el
    //    dispositivo no puede leer por DMA el framebuffer que le enlazamos.
    raiz.set_command(
        device_function,
        Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
    );

    // 3. Montar el transporte PCI moderno y el adaptador grafico.
    let transporte = PciTransport::new::<KernelHal, _>(&mut raiz, device_function)
        .map_err(|_| "no se pudo montar el transporte PCI de virtio-gpu")?;
    let mut gpu = VirtIOGpu::<KernelHal, _>::new(transporte)
        .map_err(|_| "no se pudo inicializar el dispositivo virtio-gpu")?;

    // 4. Crear el recurso 2D a la resolucion del lienzo, enlazarlo a DMA y
    //    fijarlo como scanout. `change_resolution` devuelve el framebuffer del
    //    huesped: una region DMA que el `VirtIOGpu` retiene mientras viva.
    let framebuffer = gpu
        .change_resolution(ancho as u32, alto as u32)
        .map_err(|_| "virtio-gpu rechazo crear el scanout a la resolucion pedida")?;
    let base = framebuffer.as_mut_ptr();
    // El prestamo de `framebuffer` termina aqui (NLL): `base` es un puntero
    // crudo copiado, no una referencia viva, asi que mover `gpu` al `static`
    // de abajo es legitimo.

    GPU.call_once(|| Mutex::new(Gpu(gpu)));

    Ok(InfoGpu {
        ancho,
        alto,
        base,
        paso_bytes: ancho * 4,
    })
}

/// ¿Gobierna el kernel un scanout virtio-gpu? `true` solo tras un `montar` con
/// exito. Los caminos de presentacion lo consultan implicitamente al fallar el
/// `GPU.get()`; este predicado es para diagnostico y para el arranque.
pub fn disponible() -> bool {
    GPU.get().is_some()
}

/// Vuelca el framebuffer DMA al anfitrion y lo presenta en pantalla: el
/// `flush` de virtio-gpu (`TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH`) sobre el
/// rectangulo entero del scanout. La cierra cada fotograma `consola::presentar`
/// tras blittear el lienzo y estampar el puntero. No-op silencioso si el kernel
/// no gobierna la GPU (arranque sobre GOP). Un `flush` fallido se ignora: un
/// fotograma perdido no justifica tumbar el reactor — el siguiente lo corrige.
///
/// Hoy vuelca la pantalla COMPLETA aunque el compositor solo haya tocado un
/// marco: el `flush` por sub-region exigiria primitivas que la crate no expone
/// en publico. A 100 Hz y resoluciones de escritorio el coste host es holgado;
/// el blit GUEST por region (el camino rapido del compositor) se conserva.
pub fn presentar() {
    if let Some(gpu) = GPU.get() {
        let _ = gpu.lock().0.flush();
    }
}

/// FASE 62 :: instala el CURSOR POR HARDWARE. Sube `imagen` (64×64 B8G8R8A8) al
/// recurso de cursor del dispositivo y lo posa en el origen con su punto caliente
/// en `(hot_x, hot_y)`. A partir de aqui el host (QEMU) compone el puntero sobre
/// el scanout en un plano propio: moverlo es un comando en la cola de cursor, sin
/// tocar el framebuffer ni hacer flush — la cura del lag del puntero. Si falla
/// —sin GPU, recurso rechazado—, devuelve `Err` y `CURSOR_HW` queda en `false`:
/// el puntero recae limpiamente en el estampado por software.
pub fn instalar_cursor(imagen: &[u8], hot_x: u32, hot_y: u32) -> Result<(), &'static str> {
    let gpu = GPU.get().ok_or("virtio-gpu no gobernado por el kernel")?;
    gpu.lock()
        .0
        .setup_cursor(imagen, 0, 0, hot_x, hot_y)
        .map_err(|_| "virtio-gpu rechazo el recurso de cursor")?;
    CURSOR_HW.store(true, Ordering::Release);
    Ok(())
}

/// ¿Gobierna el kernel un cursor por hardware vivo? Lo consultan el compositor
/// (para no estampar el puntero por software) y `refrescar_puntero` (para mover
/// el plano de cursor en vez de re-presentar la pantalla).
pub fn cursor_hardware() -> bool {
    CURSOR_HW.load(Ordering::Acquire)
}

/// Mueve el cursor por hardware a `(x, y)` en pixeles del scanout. Es un comando
/// `MOVE_CURSOR` en la cola de cursor —~24 bytes, sin transferir framebuffer—, de
/// modo que el puntero sigue al raton con latencia de ida-y-vuelta de cola, no de
/// fotograma del compositor. No-op si no hay cursor por hardware. Un fallo del
/// comando se ignora: un movimiento perdido lo corrige el siguiente.
pub fn mover_cursor(x: usize, y: usize) {
    if !CURSOR_HW.load(Ordering::Acquire) {
        return;
    }
    if let Some(gpu) = GPU.get() {
        let _ = gpu.lock().0.move_cursor(x as u32, y as u32);
    }
}

/// Variante de `presentar` para los manejadores de fallo (panic / OOM / aborto
/// carmesi). Usa `try_lock`: si el cerrojo de la GPU estaba tomado —el colapso
/// ocurrio en mitad de un `flush`— NO espera (un spin con las interrupciones
/// acalladas colgaria la maquina) y cede; la traza serial llega igual. Cuando
/// lo consigue, vuelca el framebuffer que la baliza acaba de tiñir, para que la
/// franja roja —o el carmesi entero— se VEA sobre el scanout que el kernel
/// gobierna, no solo en el puerto serie.
pub fn presentar_baliza() {
    if let Some(gpu) = GPU.get() {
        if let Some(mut guardia) = gpu.try_lock() {
            let _ = guardia.0.flush();
        }
    }
}
