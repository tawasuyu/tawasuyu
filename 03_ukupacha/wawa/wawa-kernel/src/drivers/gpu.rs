// =============================================================================
//  renaser :: kernel/src/drivers/gpu.rs — Fase 64 :: driver virtio-gpu NATIVO
//                                          multi-scanout (multi-monitor)
// -----------------------------------------------------------------------------
//  Hasta la Fase 63 el escritorio vivia sobre UN scanout: el kernel montaba el
//  `VirtIOGpu` de la crate `virtio-drivers`, que cablea en duro `SCANOUT_ID = 0`
//  y un unico framebuffer. La crate ni siquiera EXPONE `num_scanouts`, y su
//  respuesta de `GET_DISPLAY_INFO` solo lee el PRIMER pmode —ignora las cabezas
//  1..15—. Para multi-monitor habia que sortear esa abstraccion.
//
//  En vez de FORKEAR la crate entera, este driver se construye sobre su capa
//  PUBLICA —`PciTransport`, `VirtQueue`, el `trait Hal` (que el kernel ya
//  implementa como `KernelHal`)— y habla el protocolo virtio-gpu directamente.
//  El protocolo 2D son ~8 comandos diminutos; los replicamos con los mismos
//  layouts `#[repr(C)]` que la crate, serializados por `zerocopy`. La unica
//  pieza que la crate hacia mal —enumerar cabezas— la hacemos bien: leemos
//  `num_scanouts` del config space y creamos UN recurso 2D + framebuffer DMA por
//  cabeza, cada uno fijado a su `scanout_id`.
//
//  Decision deliberada: NO leemos `GET_DISPLAY_INFO` para heredar la resolucion
//  nativa de cada monitor. IMPONEMOS la resolucion del lienzo del kernel
//  (acotada a Full HD) a cada scanout. Asi (a) el framebuffer de cada cabeza
//  calza exacto con su sub-region del lienzo global —blit fila-a-fila sin
//  reescalar— y (b) el consumo de DMA queda ACOTADO y predecible (el parseo de
//  display-info multi-pmode era el unico paso fragil; lo evitamos por completo).
//
//  Modelo de presentacion (identico a Fase 60 pero por-cabeza): el framebuffer
//  es memoria del HUESPED; pintar ahi no se ve hasta un `flush`
//  (`TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH`). `presentar()` vuelca TODAS las
//  cabezas. Si no hay dispositivo —metal real sin virtio-gpu, o QEMU sin el—,
//  `montar` devuelve `Err` SIN tocar estado: el kernel recae al framebuffer GOP
//  del firmware con UN solo output, como siempre.
// =============================================================================

use core::sync::atomic::{AtomicBool, Ordering};

use alloc::vec::Vec;
use bitflags::bitflags;
use spin::{Mutex, Once};
use virtio_drivers::queue::VirtQueue;
use virtio_drivers::transport::pci::bus::{Command, DeviceFunction, PciRoot};
use virtio_drivers::transport::pci::PciTransport;
use virtio_drivers::transport::Transport;
use virtio_drivers::{BufferDirection, Hal};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use super::disco::KernelHal;
use super::pci::CamPuertos;

/// Vendor ID de VirtIO; Device ID de un adaptador grafico. `virtio-gpu` es un
/// dispositivo de la era virtio-1.0 (sin gemelo «transicional» legacy): su unico
/// ID es el moderno `0x1040 + 16 = 0x1050`. `virtio-vga` —el mismo nucleo con
/// compatibilidad VGA, que es como QEMU sirve un GOP para el arranque— expone
/// tambien `0x1050`.
const VENDOR_VIRTIO: u16 = 0x1AF4;
const VIRTIO_GPU_IDS: [u16; 1] = [0x1050];

/// Cuantas cabezas gobierna el kernel COMO MAXIMO. virtio-gpu admite hasta 16
/// scanouts; el kernel se acota a 2 para que el lienzo global (envolvente) y el
/// DMA de los framebuffers queden dentro de los techos fundados (lienzo
/// `PIXELES_MAX`, arena `MAX_MARCOS`). Subir esto exige acompañar ambos.
const MAX_CABEZAS: usize = 2;

/// Tamaño del recurso de cursor por hardware: un cuadrado fijo de 64×64 px.
const CURSOR_LADO: u32 = 64;

/// IDs de recurso. Cada framebuffer de scanout toma `RES_FB_BASE + i`; el cursor
/// tiene el suyo aparte. Valores arbitrarios distintos de cero (0 = «sin
/// recurso» en el protocolo).
const RES_FB_BASE: u32 = 0xbabe;
const RES_CURSOR: u32 = 0xdade;

/// Una cabeza viva, vista por el camino de PRESENTACION: el recurso 2D enlazado
/// a su scanout y su rectangulo. `volcar_todas` itera estas para transferir y
/// presentar cada cabeza. El framebuffer (`base`) lo conserva el kernel via la
/// `Pantalla` que construye con la `InfoGpu` que `montar` devuelve, no aqui.
struct Cabeza {
    /// ID del recurso 2D enlazado a este scanout.
    recurso: u32,
    /// Rectangulo del recurso (siempre `0,0,ancho,alto`).
    rect: Rect,
}

/// El adaptador grafico nativo del kernel. Encierra el transporte PCI, las dos
/// colas virtio (control + cursor), los buferes de peticion/respuesta y la lista
/// de cabezas gobernadas.
struct GpuRaw {
    transport: PciTransport,
    control: VirtQueue<KernelHal, { QUEUE_SIZE as usize }>,
    cursor: VirtQueue<KernelHal, { QUEUE_SIZE as usize }>,
    buf_envio: alloc::boxed::Box<[u8]>,
    buf_recibo: alloc::boxed::Box<[u8]>,
    cabezas: Vec<Cabeza>,
}

// SEGURIDAD: `GpuRaw` encierra punteros crudos a las colas virtio, al MMIO del
// dispositivo y a las regiones DMA de los framebuffers. renaser es un kernel de
// un solo nucleo y todo acceso al adaptador se serializa tras el `Mutex` global
// `GPU`; jamas se comparte entre hilos reales. No hay manejador de IRQ que lo
// dispute: la presentacion es sincrona, guiada por el reactor cooperativo.
unsafe impl Send for GpuRaw {}

/// El adaptador global de renaser. Se monta una sola vez, en `montar`.
static GPU: Once<Mutex<GpuRaw>> = Once::new();

/// ¿Esta vivo el cursor por HARDWARE? Lo enciende `instalar_cursor` con exito.
/// Mientras este `true`, el compositor NO estampa el puntero por software en el
/// framebuffer (lo compone el host en un plano aparte) y los movimientos del
/// raton viajan por `mover_cursor` —un comando diminuto en la cola de cursor— en
/// vez de forzar un `presentar` de pantalla entera.
static CURSOR_HW: AtomicBool = AtomicBool::new(false);

/// Lo que `montar` entrega al kernel por cada cabeza para que construya su
/// `Pantalla` sobre el scanout correspondiente. El format es B8G8R8A8 (BGRA, 4
/// bytes por pixel) — el unico que el recurso 2D de virtio-gpu admite—; los
/// bytes salen B,G,R,A, justo lo que codifica `PixelFormat::Bgr` (la A queda en
/// 0, que el scanout ignora).
pub struct InfoGpu {
    /// Ancho del scanout, en pixeles.
    pub ancho: usize,
    /// Alto del scanout, en pixeles.
    pub alto: usize,
    /// Direccion virtual del primer byte del framebuffer DMA del huesped.
    pub base: *mut u8,
    /// Bytes entre el inicio de una fila y la siguiente: `ancho * 4`, sin
    /// relleno (el recurso 2D de virtio-gpu es compacto).
    pub paso_bytes: usize,
}

/// Enumera el bus PCI, localiza el primer `virtio-gpu`, monta su transporte
/// moderno y crea —para CADA scanout que el dispositivo reporta, hasta
/// `MAX_CABEZAS`— un recurso 2D de `ancho`×`alto` enlazado a memoria DMA, que
/// fija como ese scanout. Deja el adaptador tras el `Mutex` global y devuelve un
/// `InfoGpu` POR cabeza, en orden de `scanout_id`, con que el kernel construye
/// sus `Pantalla`s y dispone los outputs en el espacio compuesto. Toda falla
/// —ausencia de dispositivo, transporte indomito, recurso rechazado— se devuelve
/// como `Err` sin dejar estado a medias: el llamante recae al GOP con UN output.
pub fn montar(ancho: usize, alto: usize) -> Result<Vec<InfoGpu>, &'static str> {
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
    //    dispositivo no puede leer por DMA los framebuffers que le enlazamos.
    raiz.set_command(
        device_function,
        Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
    );

    // 3. Montar el transporte PCI moderno y negociar features.
    let mut transport = PciTransport::new::<KernelHal, _>(&mut raiz, device_function)
        .map_err(|_| "no se pudo montar el transporte PCI de virtio-gpu")?;
    let negociadas = transport.begin_init(FEATURES_SOPORTADAS);

    // 4. Cuantas cabezas reporta el dispositivo. El config space de virtio-gpu
    //    es {events_read u32 @0, events_clear u32 @4, num_scanouts u32 @8}. La
    //    crate lo lee internamente pero NO lo expone; aqui lo leemos directo.
    let num_scanouts = transport
        .read_config_space::<u32>(8)
        .map_err(|_| "virtio-gpu: no se pudo leer num_scanouts del config space")?;
    let cabezas_objetivo = (num_scanouts as usize).clamp(1, MAX_CABEZAS);

    // 5. Fundar las dos colas: control (peticiones 2D) y cursor.
    let control = VirtQueue::new(
        &mut transport,
        QUEUE_CONTROL,
        negociadas.contains(Features::RING_INDIRECT_DESC),
        negociadas.contains(Features::RING_EVENT_IDX),
    )
    .map_err(|_| "virtio-gpu: cola de control rechazada")?;
    let cursor = VirtQueue::new(
        &mut transport,
        QUEUE_CURSOR,
        negociadas.contains(Features::RING_INDIRECT_DESC),
        negociadas.contains(Features::RING_EVENT_IDX),
    )
    .map_err(|_| "virtio-gpu: cola de cursor rechazada")?;

    transport.finish_init();

    let mut gpu = GpuRaw {
        transport,
        control,
        cursor,
        buf_envio: nuevo_buf(),
        buf_recibo: nuevo_buf(),
        cabezas: Vec::with_capacity(cabezas_objetivo),
    };

    // 6. Crear un recurso 2D + framebuffer DMA por cabeza y fijarlo a su scanout.
    //    Si la PRIMERA cabeza falla, propagamos el error (sin scanout primario no
    //    hay escritorio). Si falla una SECUNDARIA, paramos ahi y seguimos con las
    //    que si montaron —un monitor de menos no justifica tumbar el arranque—.
    let mut infos = Vec::with_capacity(cabezas_objetivo);
    for i in 0..cabezas_objetivo {
        let scanout = i as u32;
        let recurso = RES_FB_BASE + scanout;
        match gpu.alta_cabeza(scanout, recurso, ancho as u32, alto as u32) {
            Ok(base) => {
                gpu.cabezas.push(Cabeza {
                    recurso,
                    rect: Rect::new(ancho as u32, alto as u32),
                });
                infos.push(InfoGpu {
                    ancho,
                    alto,
                    base,
                    paso_bytes: ancho * 4,
                });
            }
            Err(e) if i == 0 => return Err(e),
            Err(_) => break,
        }
    }

    GPU.call_once(|| Mutex::new(gpu));
    Ok(infos)
}

impl GpuRaw {
    /// Da de alta UNA cabeza: crea el recurso 2D, le reserva un framebuffer DMA
    /// de la arena, se lo enlaza como backing y lo fija al `scanout`. Devuelve el
    /// puntero base del framebuffer. Las paginas reservadas NO se liberan (el
    /// dispositivo las lee mientras viva el kernel).
    fn alta_cabeza(
        &mut self,
        scanout: u32,
        recurso: u32,
        ancho: u32,
        alto: u32,
    ) -> Result<*mut u8, &'static str> {
        let rect = Rect::new(ancho, alto);
        self.resource_create_2d(recurso, ancho, alto)
            .map_err(|_| "virtio-gpu: RESOURCE_CREATE_2D rechazado")?;

        let bytes = (ancho as usize) * (alto as usize) * 4;
        let paginas = bytes.div_ceil(PAGINA);
        // `KernelHal::dma_alloc` reserva paginas contiguas de la arena DMA y
        // devuelve (fisica, virtual). Conservamos solo la virtual: el backing se
        // enlaza por fisica al device, y el kernel pinta por la virtual.
        let (paddr, vaddr) = KernelHal::dma_alloc(paginas, BufferDirection::DriverToDevice);

        self.resource_attach_backing(recurso, paddr as u64, bytes as u32)
            .map_err(|_| "virtio-gpu: ATTACH_BACKING rechazado")?;
        self.set_scanout(rect, scanout, recurso)
            .map_err(|_| "virtio-gpu: SET_SCANOUT rechazado")?;

        Ok(vaddr.as_ptr())
    }

    /// Envia una peticion de control y bloquea por la respuesta (un `CtrlHeader`).
    /// Mismo patron que la crate: serializa al bufer de envio, encola+notifica+
    /// espera+desencola en una sola llamada sincrona, deserializa la cabecera.
    fn pedir<Req: IntoBytes + Immutable>(&mut self, req: Req) -> Result<CtrlHeader, ()> {
        req.write_to_prefix(&mut self.buf_envio).map_err(|_| ())?;
        self.control
            .add_notify_wait_pop(
                &[&self.buf_envio],
                &mut [&mut self.buf_recibo],
                &mut self.transport,
            )
            .map_err(|_| ())?;
        let (hdr, _) = CtrlHeader::read_from_prefix(&self.buf_recibo).map_err(|_| ())?;
        Ok(hdr)
    }

    /// Envia un comando de cursor (sin respuesta) por la cola de cursor.
    fn pedir_cursor<Req: IntoBytes + Immutable>(&mut self, req: Req) -> Result<(), ()> {
        req.write_to_prefix(&mut self.buf_envio).map_err(|_| ())?;
        self.cursor
            .add_notify_wait_pop(&[&self.buf_envio], &mut [], &mut self.transport)
            .map_err(|_| ())?;
        Ok(())
    }

    fn resource_create_2d(&mut self, recurso: u32, ancho: u32, alto: u32) -> Result<(), ()> {
        let h = self.pedir(ResourceCreate2D {
            header: CtrlHeader::de_tipo(CMD_RESOURCE_CREATE_2D),
            resource_id: recurso,
            format: FORMATO_B8G8R8A8,
            width: ancho,
            height: alto,
        })?;
        h.ok_nodata()
    }

    fn resource_attach_backing(&mut self, recurso: u32, paddr: u64, largo: u32) -> Result<(), ()> {
        let h = self.pedir(ResourceAttachBacking {
            header: CtrlHeader::de_tipo(CMD_RESOURCE_ATTACH_BACKING),
            resource_id: recurso,
            nr_entries: 1,
            addr: paddr,
            length: largo,
            _padding: 0,
        })?;
        h.ok_nodata()
    }

    fn set_scanout(&mut self, rect: Rect, scanout: u32, recurso: u32) -> Result<(), ()> {
        let h = self.pedir(SetScanout {
            header: CtrlHeader::de_tipo(CMD_SET_SCANOUT),
            rect,
            scanout_id: scanout,
            resource_id: recurso,
        })?;
        h.ok_nodata()
    }

    fn transfer_to_host_2d(&mut self, rect: Rect, recurso: u32) -> Result<(), ()> {
        let h = self.pedir(TransferToHost2D {
            header: CtrlHeader::de_tipo(CMD_TRANSFER_TO_HOST_2D),
            rect,
            offset: 0,
            resource_id: recurso,
            _padding: 0,
        })?;
        h.ok_nodata()
    }

    fn resource_flush(&mut self, rect: Rect, recurso: u32) -> Result<(), ()> {
        let h = self.pedir(ResourceFlush {
            header: CtrlHeader::de_tipo(CMD_RESOURCE_FLUSH),
            rect,
            resource_id: recurso,
            _padding: 0,
        })?;
        h.ok_nodata()
    }

    /// Vuelca al anfitrion TODAS las cabezas: por cada una, transfiere su
    /// framebuffer del huesped al host y lo presenta. Un fallo en una cabeza se
    /// ignora (un fotograma perdido lo corrige el siguiente); seguimos con las
    /// demas para no dejar un monitor congelado por culpa de otro.
    fn volcar_todas(&mut self) {
        // Clonar la lista (scanout/recurso/rect son Copy) evita aliasing del
        // borrow de `self.cabezas` mientras llamamos a `&mut self`.
        let cabezas: Vec<(u32, Rect)> = self.cabezas.iter().map(|c| (c.recurso, c.rect)).collect();
        for (recurso, rect) in cabezas {
            if self.transfer_to_host_2d(rect, recurso).is_ok() {
                let _ = self.resource_flush(rect, recurso);
            }
        }
    }

    /// Instala el sprite de cursor (64×64 B8G8R8A8) en el scanout PRIMARIO.
    fn alta_cursor(&mut self, imagen: &[u8], hot_x: u32, hot_y: u32) -> Result<(), &'static str> {
        let bytes = (CURSOR_LADO * CURSOR_LADO * 4) as usize;
        if imagen.len() != bytes {
            return Err("virtio-gpu: imagen de cursor con tamaño invalido");
        }
        let paginas = bytes.div_ceil(PAGINA);
        let (paddr, vaddr) = KernelHal::dma_alloc(paginas, BufferDirection::DriverToDevice);
        // SEGURIDAD: `dma_alloc` devuelve memoria propia, alineada y del tamaño
        // pedido; copiamos la imagen una vez. Las paginas no se liberan (el
        // recurso de cursor vive mientras el sistema).
        unsafe {
            core::ptr::copy_nonoverlapping(imagen.as_ptr(), vaddr.as_ptr(), bytes);
        }
        let rect = Rect::new(CURSOR_LADO, CURSOR_LADO);
        self.resource_create_2d(RES_CURSOR, CURSOR_LADO, CURSOR_LADO)
            .map_err(|_| "virtio-gpu: recurso de cursor rechazado")?;
        self.resource_attach_backing(RES_CURSOR, paddr as u64, bytes as u32)
            .map_err(|_| "virtio-gpu: backing de cursor rechazado")?;
        self.transfer_to_host_2d(rect, RES_CURSOR)
            .map_err(|_| "virtio-gpu: transfer de cursor rechazado")?;
        self.update_cursor(0, 0, hot_x, hot_y, false)
            .map_err(|_| "virtio-gpu: update de cursor rechazado")?;
        Ok(())
    }

    fn update_cursor(
        &mut self,
        pos_x: u32,
        pos_y: u32,
        hot_x: u32,
        hot_y: u32,
        es_move: bool,
    ) -> Result<(), ()> {
        self.pedir_cursor(UpdateCursor {
            header: CtrlHeader::de_tipo(if es_move { CMD_MOVE_CURSOR } else { CMD_UPDATE_CURSOR }),
            pos: CursorPos {
                scanout_id: 0,
                x: pos_x,
                y: pos_y,
                _padding: 0,
            },
            resource_id: RES_CURSOR,
            hot_x,
            hot_y,
            _padding: 0,
        })
    }
}

/// ¿Gobierna el kernel un scanout virtio-gpu? `true` solo tras un `montar` con
/// exito. Los caminos de presentacion lo consultan implicitamente al fallar el
/// `GPU.get()`; este predicado es para diagnostico y para el arranque.
pub fn disponible() -> bool {
    GPU.get().is_some()
}

/// Cuantas cabezas (monitores) gobierna el kernel. `0` si no monto la GPU.
pub fn cabezas() -> usize {
    GPU.get().map(|g| g.lock().cabezas.len()).unwrap_or(0)
}

/// Vuelca TODAS las cabezas al anfitrion y las presenta. La cierra cada
/// fotograma `consola::presentar` tras blittear los lienzos. No-op silencioso si
/// el kernel no gobierna la GPU (arranque sobre GOP).
pub fn presentar() {
    if let Some(gpu) = GPU.get() {
        gpu.lock().volcar_todas();
    }
}

/// FASE 62 :: instala el CURSOR POR HARDWARE sobre el scanout primario. Sube
/// `imagen` (64×64 B8G8R8A8) y lo posa en el origen con su punto caliente en
/// `(hot_x, hot_y)`. A partir de aqui el host compone el puntero en su propio
/// plano: moverlo es un comando en la cola de cursor, sin tocar framebuffer ni
/// flush — la cura del lag del puntero. Si falla, `CURSOR_HW` queda en `false` y
/// el puntero recae limpiamente en el estampado por software.
pub fn instalar_cursor(imagen: &[u8], hot_x: u32, hot_y: u32) -> Result<(), &'static str> {
    let gpu = GPU.get().ok_or("virtio-gpu no gobernado por el kernel")?;
    gpu.lock().alta_cursor(imagen, hot_x, hot_y)?;
    CURSOR_HW.store(true, Ordering::Release);
    Ok(())
}

/// ¿Gobierna el kernel un cursor por hardware vivo? Lo consultan el compositor
/// (para no estampar el puntero por software) y `refrescar_puntero` (para mover
/// el plano de cursor en vez de re-presentar la pantalla).
pub fn cursor_hardware() -> bool {
    CURSOR_HW.load(Ordering::Acquire)
}

/// Mueve el cursor por hardware a `(x, y)` en pixeles del scanout primario. Es un
/// comando `MOVE_CURSOR` —~24 bytes, sin transferir framebuffer—, de modo que el
/// puntero sigue al raton con latencia de cola, no de fotograma del compositor.
/// No-op si no hay cursor por hardware. Un fallo se ignora.
pub fn mover_cursor(x: usize, y: usize) {
    if !CURSOR_HW.load(Ordering::Acquire) {
        return;
    }
    if let Some(gpu) = GPU.get() {
        let _ = gpu.lock().update_cursor(x as u32, y as u32, 0, 0, true);
    }
}

/// Variante de `presentar` para los manejadores de fallo (panic / OOM / aborto
/// carmesi). Usa `try_lock`: si el cerrojo de la GPU estaba tomado —el colapso
/// ocurrio en mitad de un volcado— NO espera (un spin con las interrupciones
/// acalladas colgaria la maquina) y cede; la traza serial llega igual. Cuando lo
/// consigue, vuelca todas las cabezas que la baliza acaba de teñir.
pub fn presentar_baliza() {
    if let Some(gpu) = GPU.get() {
        if let Some(mut guardia) = gpu.try_lock() {
            guardia.volcar_todas();
        }
    }
}

// =============================================================================
//  PROTOCOLO virtio-gpu — comandos 2D (espejo de los layouts de la crate)
// =============================================================================

/// Tamaño de pagina (idéntico a `virtio_drivers::PAGE_SIZE`, reexpuesto aqui
/// para no depender de su visibilidad).
const PAGINA: usize = 0x1000;

/// Tamaño del bufer de peticion/respuesta de cada cola: una pagina holgada para
/// el mayor comando (todos caben en < 64 B).
fn nuevo_buf() -> alloc::boxed::Box<[u8]> {
    alloc::vec![0u8; PAGINA].into_boxed_slice()
}

const QUEUE_CONTROL: u16 = 0;
const QUEUE_CURSOR: u16 = 1;
const QUEUE_SIZE: u16 = 2;

// Tipos de comando (peticion) y respuesta del protocolo virtio-gpu.
const CMD_RESOURCE_CREATE_2D: u32 = 0x101;
const CMD_SET_SCANOUT: u32 = 0x103;
const CMD_RESOURCE_FLUSH: u32 = 0x104;
const CMD_TRANSFER_TO_HOST_2D: u32 = 0x105;
const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x106;
const CMD_UPDATE_CURSOR: u32 = 0x300;
const CMD_MOVE_CURSOR: u32 = 0x301;
const RESP_OK_NODATA: u32 = 0x1100;

/// Format B8G8R8A8_UNORM — el unico que usamos (calza con `PixelFormat::Bgr`).
const FORMATO_B8G8R8A8: u32 = 1;

bitflags! {
    /// Features que el driver negocia. VERSION_1 (virtio moderno) es obligatorio;
    /// INDIRECT_DESC y EVENT_IDX optimizan las colas si el device los ofrece.
    #[derive(Copy, Clone, Debug)]
    struct Features: u64 {
        const RING_INDIRECT_DESC = 1 << 28;
        const RING_EVENT_IDX = 1 << 29;
        const VERSION_1 = 1 << 32;
    }
}

const FEATURES_SOPORTADAS: Features = Features::RING_INDIRECT_DESC
    .union(Features::RING_EVENT_IDX)
    .union(Features::VERSION_1);

#[repr(C)]
#[derive(Clone, Copy, Debug, FromBytes, Immutable, IntoBytes, KnownLayout)]
struct CtrlHeader {
    hdr_type: u32,
    flags: u32,
    fence_id: u64,
    ctx_id: u32,
    _padding: u32,
}

impl CtrlHeader {
    fn de_tipo(hdr_type: u32) -> CtrlHeader {
        CtrlHeader {
            hdr_type,
            flags: 0,
            fence_id: 0,
            ctx_id: 0,
            _padding: 0,
        }
    }

    /// `Ok` si la respuesta es OK_NODATA; `Err` en cualquier otro caso (el device
    /// rechazo el comando).
    fn ok_nodata(&self) -> Result<(), ()> {
        if self.hdr_type == RESP_OK_NODATA {
            Ok(())
        } else {
            Err(())
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, FromBytes, Immutable, IntoBytes, KnownLayout)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl Rect {
    fn new(width: u32, height: u32) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        }
    }
}

#[repr(C)]
#[derive(Debug, Immutable, IntoBytes, KnownLayout)]
struct ResourceCreate2D {
    header: CtrlHeader,
    resource_id: u32,
    format: u32,
    width: u32,
    height: u32,
}

#[repr(C)]
#[derive(Debug, Immutable, IntoBytes, KnownLayout)]
struct ResourceAttachBacking {
    header: CtrlHeader,
    resource_id: u32,
    nr_entries: u32, // siempre 1
    addr: u64,
    length: u32,
    _padding: u32,
}

#[repr(C)]
#[derive(Debug, Immutable, IntoBytes, KnownLayout)]
struct SetScanout {
    header: CtrlHeader,
    rect: Rect,
    scanout_id: u32,
    resource_id: u32,
}

#[repr(C)]
#[derive(Debug, Immutable, IntoBytes, KnownLayout)]
struct TransferToHost2D {
    header: CtrlHeader,
    rect: Rect,
    offset: u64,
    resource_id: u32,
    _padding: u32,
}

#[repr(C)]
#[derive(Debug, Immutable, IntoBytes, KnownLayout)]
struct ResourceFlush {
    header: CtrlHeader,
    rect: Rect,
    resource_id: u32,
    _padding: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Immutable, IntoBytes, KnownLayout)]
struct CursorPos {
    scanout_id: u32,
    x: u32,
    y: u32,
    _padding: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Immutable, IntoBytes, KnownLayout)]
struct UpdateCursor {
    header: CtrlHeader,
    pos: CursorPos,
    resource_id: u32,
    hot_x: u32,
    hot_y: u32,
    _padding: u32,
}
