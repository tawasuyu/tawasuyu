// =============================================================================
//  renaser :: kernel/src/drivers/tableta.rs — Fase 61 :: el puntero absoluto
// -----------------------------------------------------------------------------
//  El raton PS/2 (Fase 13) habla en DELTAS: cada paquete dice «moveme tanto».
//  El kernel integra esos deltas a una posicion. Funciona, pero arrastra los
//  males del raton relativo: si el host no captura el cursor, no hay deltas; y
//  cuando los hay, la posicion del huesped DERIVA de la del host —nunca calzan—.
//
//  La Fase 61 continua el arco «legacy -> virtio moderno» (consola en la 49,
//  scanout en la 60) con la entrada: un dispositivo `virtio-input` configurado
//  como TABLETA reporta coordenadas ABSOLUTAS (ejes `ABS_X`/`ABS_Y`), de modo
//  que el cursor del huesped sigue 1:1 al del host, sin captura ni deriva.
//
//  El driver NO reemplaza al PS/2: lo COMPLEMENTA. Traduce los eventos evdev de
//  virtio-input a una posicion en pixeles y la entrega por
//  `raton::actualizar_desde_tableta` —el MISMO sumidero que alimenta el PS/2—,
//  asi que el compositor no distingue origen. Si no hay tableta, `montar`
//  devuelve `Err` y el puntero sigue siendo el del raton relativo.
//
//  Se SONDEA en cada fotograma desde la tarea del compositor (como el demuxer
//  Akasha), no por IRQ: drenar la cola virtio en contexto de interrupcion
//  exigiria un cerrojo en IRQ, y a 100 Hz la latencia del puntero (<=10 ms) es
//  imperceptible. La linea INTx queda enmascarada en el PIC — cero IRQ espurias.
// =============================================================================

use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

use spin::{Mutex, Once};
use virtio_drivers::device::input::VirtIOInput;
use virtio_drivers::transport::pci::bus::{Command, DeviceFunction, PciRoot};
use virtio_drivers::transport::pci::PciTransport;

use super::disco::KernelHal;
use super::pci::CamPuertos;

/// Vendor ID de VirtIO; Device ID de un dispositivo de entrada. `virtio-input`
/// es de la era virtio-1.0: su unico ID es el moderno `0x1040 + 18 = 0x1052`.
/// `virtio-tablet-pci`, `virtio-mouse-pci` y `virtio-keyboard-pci` comparten
/// este ID; los distinguimos por sus capacidades (la tableta tiene eje ABS).
const VENDOR_VIRTIO: u16 = 0x1AF4;
const VIRTIO_INPUT_IDS: [u16; 1] = [0x1052];

// --- Constantes del protocolo evdev (las que la tableta usa) ---------------
/// Sincronizacion: un `EV_SYN`/`SYN_REPORT` cierra un lote de eventos coherente.
const EV_SYN: u16 = 0x00;
/// Tecla o boton: `code` es un `BTN_*`, `value` es 1 (pulsado) o 0 (suelto).
const EV_KEY: u16 = 0x01;
/// Eje absoluto: `code` es `ABS_X`/`ABS_Y`, `value` la coordenada en su rango.
const EV_ABS: u16 = 0x03;
/// Eje horizontal absoluto.
const ABS_X: u16 = 0x00;
/// Eje vertical absoluto.
const ABS_Y: u16 = 0x01;
/// Boton primario del puntero (izquierdo).
const BTN_LEFT: u16 = 0x110;
/// Boton secundario (derecho).
const BTN_RIGHT: u16 = 0x111;
/// Boton terciario (central).
const BTN_MIDDLE: u16 = 0x112;

/// La tableta, ya montada. Envuelve al `VirtIOInput` para que viva en un
/// `static` tras el `Mutex` global.
struct Tableta(VirtIOInput<KernelHal, PciTransport>);

// SEGURIDAD: `Tableta` encierra punteros crudos a las colas virtio y al MMIO
// del dispositivo. renaser es de un solo nucleo y todo acceso se serializa tras
// el `Mutex` global. No hay manejador de IRQ que lo dispute: el drenaje es por
// sondeo desde el reactor cooperativo, con las interrupciones habilitadas.
unsafe impl Send for Tableta {}

/// La tableta global. Se monta una sola vez, en `montar`.
static TABLETA: Once<Mutex<Tableta>> = Once::new();

// --- Rango de los ejes absolutos y dimensiones de la pantalla, para escalar --
static MIN_X: AtomicU32 = AtomicU32::new(0);
static MAX_X: AtomicU32 = AtomicU32::new(0);
static MIN_Y: AtomicU32 = AtomicU32::new(0);
static MAX_Y: AtomicU32 = AtomicU32::new(0);
static ANCHO: AtomicU32 = AtomicU32::new(0);
static ALTO: AtomicU32 = AtomicU32::new(0);

// --- Acumulador del lote evdev en curso, comprometido en cada `EV_SYN` -------
/// Ultima `ABS_X` cruda recibida (en el rango del eje, sin escalar aun).
static ACC_X: AtomicU32 = AtomicU32::new(0);
/// Ultima `ABS_Y` cruda recibida.
static ACC_Y: AtomicU32 = AtomicU32::new(0);
/// Estado de botones acumulado (bit 0 izq, 1 der, 2 central) — persiste entre
/// lotes: un boton sigue pulsado hasta que llega su `EV_KEY` con `value == 0`.
static ACC_BOTONES: AtomicU8 = AtomicU8::new(0);

/// Enumera el bus PCI, localiza un `virtio-input` que sea TABLETA (tiene eje
/// absoluto), lo monta y deja tras el `Mutex` global. Lee el rango de sus ejes
/// `ABS_X`/`ABS_Y` para escalar a pixeles. Toda falla —sin dispositivo, ninguno
/// con eje absoluto, transporte indomito— se devuelve como `Err`: el puntero
/// recae limpiamente en el raton PS/2 relativo.
pub fn montar(ancho: usize, alto: usize) -> Result<(), &'static str> {
    let mut raiz = PciRoot::new(CamPuertos);

    // 1. Reunir TODAS las funciones virtio-input del bus: puede haber varias
    //    (teclado, raton, tableta) y solo una nos sirve.
    let mut candidatos: alloc::vec::Vec<DeviceFunction> = alloc::vec::Vec::new();
    for bus in 0..=255u8 {
        for (device_function, info) in raiz.enumerate_bus(bus) {
            if info.vendor_id == VENDOR_VIRTIO && VIRTIO_INPUT_IDS.contains(&info.device_id) {
                candidatos.push(device_function);
            }
        }
    }
    if candidatos.is_empty() {
        return Err("virtio-input no hallado en el bus PCI");
    }

    // 2. Probar cada candidato: montar su transporte, instanciarlo y preguntar
    //    por su eje `ABS_X`. El que responda con un rango valido es la tableta;
    //    el resto (teclado, raton relativo) se descartan al soltar el `Drop`.
    for device_function in candidatos {
        raiz.set_command(
            device_function,
            Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
        );
        let transporte = match PciTransport::new::<KernelHal, _>(&mut raiz, device_function) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let mut input = match VirtIOInput::<KernelHal, _>::new(transporte) {
            Ok(d) => d,
            Err(_) => continue,
        };
        // ¿Tiene eje absoluto con rango util? Si `abs_info` falla o el rango es
        // degenerado, no es una tableta: descartar y seguir con el proximo.
        let (info_x, info_y) = match (input.abs_info(ABS_X as u8), input.abs_info(ABS_Y as u8)) {
            (Ok(x), Ok(y)) if x.max > x.min && y.max > y.min => (x, y),
            _ => continue,
        };

        MIN_X.store(info_x.min, Ordering::Relaxed);
        MAX_X.store(info_x.max, Ordering::Relaxed);
        MIN_Y.store(info_y.min, Ordering::Relaxed);
        MAX_Y.store(info_y.max, Ordering::Relaxed);
        ANCHO.store(ancho as u32, Ordering::Relaxed);
        ALTO.store(alto as u32, Ordering::Relaxed);
        // Arrancar el acumulador en el centro de los ejes — antes del primer
        // movimiento, el puntero no salta a una esquina.
        ACC_X.store((info_x.min + info_x.max) / 2, Ordering::Relaxed);
        ACC_Y.store((info_y.min + info_y.max) / 2, Ordering::Relaxed);

        TABLETA.call_once(|| Mutex::new(Tableta(input)));
        return Ok(());
    }

    Err("ningun virtio-input expone eje absoluto (no hay tableta)")
}

/// ¿Gobierna el kernel una tableta virtio-input? `true` solo tras un `montar`
/// con exito. API publica para diagnostico y para futuros consumidores (un
/// indicador en la barra de tareas, una syscall de capacidad de puntero).
#[allow(dead_code)]
pub fn disponible() -> bool {
    TABLETA.get().is_some()
}

/// Escala una coordenada cruda del eje absoluto `[min, max]` a un pixel de
/// `[0, dim)`. Si el rango es degenerado (no deberia, lo filtramos en `montar`)
/// devuelve 0 — el puntero a la esquina, una pista visible de configuracion mala.
fn escalar(valor: u32, min: u32, max: u32, dim: u32) -> usize {
    if max <= min || dim == 0 {
        return 0;
    }
    let valor = valor.clamp(min, max);
    let span = (max - min) as u64;
    (((valor - min) as u64 * (dim as u64 - 1)) / span) as usize
}

/// Drena los eventos evdev pendientes de la tableta y, en cada `EV_SYN`,
/// compromete la posicion absoluta y el estado de botones al sumidero del
/// puntero. La invoca la tarea del compositor en cada fotograma. No-op
/// silencioso si no hay tableta. Acumula `ABS_X`/`ABS_Y`/`EV_KEY` entre
/// sincronizaciones: el lote no se aplica a medias.
pub fn atender() {
    let Some(tableta) = TABLETA.get() else {
        return;
    };
    let mut guardia = tableta.lock();
    while let Some(evento) = guardia.0.pop_pending_event() {
        match evento.event_type {
            EV_ABS => match evento.code {
                ABS_X => ACC_X.store(evento.value, Ordering::Relaxed),
                ABS_Y => ACC_Y.store(evento.value, Ordering::Relaxed),
                _ => {}
            },
            EV_KEY => {
                let bit = match evento.code {
                    BTN_LEFT => 0b0000_0001,
                    BTN_RIGHT => 0b0000_0010,
                    BTN_MIDDLE => 0b0000_0100,
                    _ => 0,
                };
                if bit != 0 {
                    let antes = ACC_BOTONES.load(Ordering::Relaxed);
                    let ahora = if evento.value != 0 {
                        antes | bit
                    } else {
                        antes & !bit
                    };
                    ACC_BOTONES.store(ahora, Ordering::Relaxed);
                }
            }
            EV_SYN => {
                // Lote cerrado: escalar la ultima posicion absoluta y entregarla.
                let x = escalar(
                    ACC_X.load(Ordering::Relaxed),
                    MIN_X.load(Ordering::Relaxed),
                    MAX_X.load(Ordering::Relaxed),
                    ANCHO.load(Ordering::Relaxed),
                );
                let y = escalar(
                    ACC_Y.load(Ordering::Relaxed),
                    MIN_Y.load(Ordering::Relaxed),
                    MAX_Y.load(Ordering::Relaxed),
                    ALTO.load(Ordering::Relaxed),
                );
                crate::drivers::raton::actualizar_desde_tableta(
                    x,
                    y,
                    ACC_BOTONES.load(Ordering::Relaxed),
                );
            }
            _ => {}
        }
    }
}
