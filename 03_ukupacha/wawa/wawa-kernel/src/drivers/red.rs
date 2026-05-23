// =============================================================================
//  renaser :: kernel/src/drivers/red.rs — Fase 18 :: virtio-net
// -----------------------------------------------------------------------------
//  El kernel deja de hablar solo consigo mismo. Con el mismo patron del disco
//  —enumerar PCI, montar el transporte de virtio, ceder a `virtio-drivers` el
//  diálogo de bajo nivel— renaser abre una boca y una oreja al exterior: una
//  tarjeta de red virtio.
//
//  En esta primera version el kernel envia un ARP request al gateway de
//  QEMU (10.0.2.2) en cuanto arranca, y registra por COM1 cada paquete que
//  recibe. No hay pila TCP/IP — solo ethernet crudo. El proximo paso natural
//  seria una capa de capacidades `sys_net_*` para que los apps tambien
//  hablen, pero esa es otra fase.
// =============================================================================

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use spin::{Mutex, Once};
use virtio_drivers::device::net::VirtIONet;
use virtio_drivers::transport::pci::bus::{Command, DeviceFunction, PciRoot};
use virtio_drivers::transport::pci::PciTransport;
use x86_64::instructions::interrupts;

use super::disco::KernelHal;
use super::pci::CamPuertos;

/// Vendor ID de VirtIO; Device IDs de un dispositivo de red (legacy + modern).
const VENDOR_VIRTIO: u16 = 0x1AF4;
const VIRTIO_NET_IDS: [u16; 2] = [0x1000, 0x1041];

/// Tamaño maximo de paquete que reservamos por bufer (MTU 1500 + algo de holgura
/// para cabeceras virtio y futuras VLAN).
const PAQUETE_MAX: usize = 1600;

/// Profundidad de las colas RX y TX. 16 es pequeño pero suficiente para el
/// trafico de un demo.
const PROFUNDIDAD_COLA: usize = 16;

/// EtherType experimental (rango 0x88B5-0x88B6, reservado por IEEE para uso
/// local). renaser lo usaria si quisiera definir su propio protocolo.
pub const ETHER_TYPE_RENASER: u16 = 0x88B5;
/// EtherType de ARP.
pub const ETHER_TYPE_ARP: u16 = 0x0806;

/// Direccion fisica de la tarjeta de red, en seis bytes MAC.
pub type Mac = [u8; 6];

/// IP de la maquina renaser, en QEMU user-mode networking (10.0.2.0/24).
pub const IP_RENASER: [u8; 4] = [10, 0, 2, 15];
/// IP del gateway que QEMU expone hacia el host.
pub const IP_GATEWAY: [u8; 4] = [10, 0, 2, 2];

/// La tarjeta de red, ya montada. Envuelve a `VirtIONet` para que pueda vivir
/// en un `static`.
struct Tarjeta(VirtIONet<KernelHal, PciTransport, PROFUNDIDAD_COLA>);

// SEGURIDAD: `Tarjeta` encierra punteros crudos a las colas virtio y al MMIO
// del dispositivo. renaser es de un solo nucleo y todo acceso a la tarjeta se
// serializa tras el `Mutex` global. Los accesos cooperativos se hacen con las
// interrupciones acalladas para que la IRQ del dispositivo jamas las dispute.
unsafe impl Send for Tarjeta {}

/// La tarjeta global. Se monta una sola vez, en `montar`.
static TARJETA: Once<Mutex<Tarjeta>> = Once::new();

/// La direccion MAC que el dispositivo nos asigno, cacheada para consulta.
static MAC: Once<Mac> = Once::new();

/// La linea de IRQ asignada al dispositivo por el firmware.
static IRQ_RED: AtomicU8 = AtomicU8::new(0);

/// Cuenta de paquetes recibidos desde el arranque.
static PAQUETES_RX: AtomicU64 = AtomicU64::new(0);
/// Cuenta de paquetes transmitidos desde el arranque.
static PAQUETES_TX: AtomicU64 = AtomicU64::new(0);

// =============================================================================
//  Montaje
// =============================================================================

/// Enumera el bus PCI, localiza el virtio-net, monta su transporte moderno y
/// lo deja tras el `Mutex` global. Descubre su linea de IRQ y la enruta.
/// Devuelve la MAC que el dispositivo nos confiere. Toda falla se devuelve.
pub fn montar() -> Result<Mac, &'static str> {
    let mut raiz = PciRoot::new(CamPuertos);

    // 1. Localizar el primer virtio-net en el bus.
    let mut hallado: Option<DeviceFunction> = None;
    'busqueda: for bus in 0..=255u8 {
        for (device_function, info) in raiz.enumerate_bus(bus) {
            if info.vendor_id == VENDOR_VIRTIO && VIRTIO_NET_IDS.contains(&info.device_id) {
                hallado = Some(device_function);
                break 'busqueda;
            }
        }
    }
    let device_function = hallado.ok_or("virtio-net no hallado en el bus PCI")?;

    // 2. Habilitar E/S, MMIO y BUS-MASTER en la configuracion PCI.
    raiz.set_command(
        device_function,
        Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
    );

    // 3. Montar el transporte PCI moderno y el dispositivo de red.
    let transporte = PciTransport::new::<KernelHal, _>(&mut raiz, device_function)
        .map_err(|_| "no se pudo montar el transporte PCI de virtio-net")?;
    let mut nic =
        VirtIONet::<KernelHal, _, PROFUNDIDAD_COLA>::new(transporte, PAQUETE_MAX)
            .map_err(|_| "no se pudo inicializar el dispositivo virtio-net")?;

    let mac = nic.mac_address();
    nic.enable_interrupts();

    TARJETA.call_once(|| Mutex::new(Tarjeta(nic)));
    MAC.call_once(|| mac);

    // 4. Descubrir la linea de IRQ y enrutarla.
    let irq = super::pci::linea_irq(device_function);
    if (2..=15).contains(&irq) {
        crate::interrupts::registrar_irq_red(irq);
        crate::pic::desenmascarar(irq);
        IRQ_RED.store(irq, Ordering::SeqCst);
    }

    Ok(mac)
}

// =============================================================================
//  IRQ
// =============================================================================

/// Punto de entrada DESDE el manejador de IRQ de la red. Acknowledge en el
/// dispositivo —para que la linea baje— y se sale.
pub fn atender_irq() {
    if let Some(tarjeta) = TARJETA.get() {
        // SEGURIDAD: en contexto de IRQ las interrupciones ya estan acalladas;
        // tomar el cerrojo aqui no puede interbloquear con las tareas, que
        // siempre lo toman con `interrupts::without_interrupts`.
        let _ = tarjeta.lock().0.ack_interrupt();
    }
}

/// La linea de IRQ del dispositivo, si el firmware enruto una util.
pub fn irq() -> Option<u8> {
    let v = IRQ_RED.load(Ordering::SeqCst);
    if v == 0 {
        None
    } else {
        Some(v)
    }
}

// =============================================================================
//  Consulta y E/S — la interfaz para las tareas cooperativas
// =============================================================================

/// La MAC del dispositivo. `None` si la tarjeta aun no se ha montado.
#[allow(dead_code)]
pub fn mac() -> Option<Mac> {
    MAC.get().copied()
}

/// Numero de paquetes recibidos desde el arranque.
#[allow(dead_code)]
pub fn paquetes_rx() -> u64 {
    PAQUETES_RX.load(Ordering::Relaxed)
}

/// Numero de paquetes transmitidos desde el arranque.
#[allow(dead_code)]
pub fn paquetes_tx() -> u64 {
    PAQUETES_TX.load(Ordering::Relaxed)
}

/// Envia un frame Ethernet crudo (cabecera + payload, sin CRC — el dispositivo
/// se la añade). El llamante construye el frame entero.
pub fn enviar(frame: &[u8]) -> Result<(), &'static str> {
    let tarjeta = TARJETA.get().ok_or("red no montada")?;
    interrupts::without_interrupts(|| {
        let mut tarjeta = tarjeta.lock();
        let mut tx = tarjeta.0.new_tx_buffer(frame.len());
        tx.packet_mut().copy_from_slice(frame);
        tarjeta.0.send(tx).map_err(|_| "envio fallido")?;
        PAQUETES_TX.fetch_add(1, Ordering::Relaxed);
        Ok(())
    })
}

/// Drena los paquetes RX pendientes y aplica `callback` a cada uno. Cada
/// bufer se recicla a la cola RX al terminar — el dispositivo tiene siempre
/// receptores listos para la proxima IRQ.
pub fn drenar_rx<F: FnMut(&[u8])>(mut callback: F) {
    let Some(tarjeta) = TARJETA.get() else {
        return;
    };
    interrupts::without_interrupts(|| {
        let mut tarjeta = tarjeta.lock();
        loop {
            if !tarjeta.0.can_recv() {
                break;
            }
            let rx = match tarjeta.0.receive() {
                Ok(r) => r,
                Err(_) => break,
            };
            callback(rx.packet());
            let _ = tarjeta.0.recycle_rx_buffer(rx);
            PAQUETES_RX.fetch_add(1, Ordering::Relaxed);
        }
    });
}

// =============================================================================
//  Composicion de un ARP request — el primer paquete que renaser saluda
// =============================================================================

/// Compone un frame Ethernet con una peticion ARP que pregunta por la MAC del
/// host `objetivo_ip`. El gateway de QEMU lo responde — su replica entra por
/// la cola RX y se registra en COM1 desde la tarea cooperativa de la red.
pub fn componer_arp_request(
    nuestro_mac: Mac,
    nuestro_ip: [u8; 4],
    objetivo_ip: [u8; 4],
) -> [u8; 42] {
    let mut frame = [0u8; 42];
    // Cabecera Ethernet.
    frame[0..6].copy_from_slice(&[0xff; 6]); // destino: broadcast
    frame[6..12].copy_from_slice(&nuestro_mac);
    frame[12..14].copy_from_slice(&ETHER_TYPE_ARP.to_be_bytes());
    // Payload ARP (28 bytes).
    frame[14..16].copy_from_slice(&1u16.to_be_bytes()); // HW type: Ethernet
    frame[16..18].copy_from_slice(&0x0800u16.to_be_bytes()); // proto: IPv4
    frame[18] = 6; // HW len
    frame[19] = 4; // proto len
    frame[20..22].copy_from_slice(&1u16.to_be_bytes()); // opcode: REQUEST
    frame[22..28].copy_from_slice(&nuestro_mac); // sender MAC
    frame[28..32].copy_from_slice(&nuestro_ip); // sender IP
                                                  // bytes 32..38: target MAC, se quedan a cero
    frame[38..42].copy_from_slice(&objetivo_ip); // target IP
    frame
}
