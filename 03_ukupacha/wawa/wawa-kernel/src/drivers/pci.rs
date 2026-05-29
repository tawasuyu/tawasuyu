// =============================================================================
//  renaser :: kernel/src/drivers/pci.rs — Fase 6.1 :: el acceso al bus PCI
// -----------------------------------------------------------------------------
//  El cargador `bootloader` entrega el mapa de memoria y el framebuffer, pero
//  NO un censo de perifericos: descubrir el disco es tarea del kernel. Aqui
//  renaser habla con el espacio de configuracion del bus PCI a traves del
//  mecanismo #1 —los puertos 0xCF8 (direccion) y 0xCFC (datos)—.
//
//  Este modulo provee `CamPuertos`, una implementacion del rasgo
//  `ConfigurationAccess` de `virtio-drivers`: la crate enumera el bus y mapea
//  los BARs del dispositivo apoyandose en estas dos funciones de acceso.
// =============================================================================

use alloc::vec::Vec;

use virtio_drivers::transport::pci::bus::{ConfigurationAccess, DeviceFunction};
use x86_64::instructions::port::Port;

/// Puerto de DIRECCION del mecanismo de configuracion PCI #1.
const CONFIG_ADDRESS: u16 = 0xCF8;
/// Puerto de DATOS del mecanismo de configuracion PCI #1.
const CONFIG_DATA: u16 = 0xCFC;

/// Compone la palabra de direccion del mecanismo #1: bit 31 de habilitacion,
/// bus, dispositivo, funcion y offset de registro alineado a dword.
fn direccion(device_function: DeviceFunction, offset: u8) -> u32 {
    0x8000_0000
        | ((device_function.bus as u32) << 16)
        | ((device_function.device as u32) << 11)
        | ((device_function.function as u32) << 8)
        | ((offset as u32) & 0xFC)
}

/// Lee un registro de 32 bits del espacio de configuracion PCI.
fn leer(device_function: DeviceFunction, offset: u8) -> u32 {
    // SEGURIDAD: 0xCF8/0xCFC son los puertos del mecanismo de configuracion
    // PCI #1, fijos en la arquitectura PC. La direccion lleva su bit de
    // habilitacion y el offset alineado a dword, como exige el protocolo.
    unsafe {
        Port::<u32>::new(CONFIG_ADDRESS).write(direccion(device_function, offset));
        Port::<u32>::new(CONFIG_DATA).read()
    }
}

/// Escribe un registro de 32 bits en el espacio de configuracion PCI.
fn escribir(device_function: DeviceFunction, offset: u8, dato: u32) {
    // SEGURIDAD: vease `leer` — mismos puertos, mismo protocolo del 8259/PCI.
    unsafe {
        Port::<u32>::new(CONFIG_ADDRESS).write(direccion(device_function, offset));
        Port::<u32>::new(CONFIG_DATA).write(dato);
    }
}

/// Lee el registro «Interrupt Line» (byte bajo del offset 0x3C): la linea de
/// IRQ del PIC que el firmware UEFI enruto y asigno a este dispositivo. Es el
/// puente entre el descubrimiento PCI y el manejo de interrupciones (Fase 6.2).
pub fn linea_irq(device_function: DeviceFunction) -> u8 {
    (leer(device_function, 0x3C) & 0xFF) as u8
}

// =============================================================================
//  ENUMERACION POR CLASE :: descubrir dispositivos sin saber vendor/device
// -----------------------------------------------------------------------------
//  virtio-drivers ya cubre virtio-blk/net/console por vendor 0x1AF4, pero el
//  resto del mundo PCI se identifica por la TRIPLA `(class, subclass, prog_if)`
//  del registro 0x08 del header. Esa tripla es vendor-agnostica: cualquier
//  controlador XHCI legitimo dice `(0x0C, 0x03, 0x30)`, cualquier NVMe dice
//  `(0x01, 0x08, 0x02)`. Este modulo expone esa enumeracion para que los
//  drivers nuevos no tengan que tocar puertos manualmente.
//
//  La enumeracion es lineal sobre el espacio canonico (256 buses × 32 dev ×
//  hasta 8 funciones). Cara: ~65 000 lecturas en el peor caso, pero solo se
//  recorre al boot. Respeta el bit «multifunction» del header type para no
//  leer funciones inexistentes.
// =============================================================================

/// Informacion de identificacion de un dispositivo PCI. La devuelve
/// `leer_info` cuando el slot tiene un dispositivo presente.
///
/// `#[allow(dead_code)]` — API consumida por los drivers de X2 (XHCI) y
/// X3 (USB-MS) cuando entren. Hasta ese punto el campo se compila pero
/// no se usa.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct InfoPci {
    pub bus: u8,
    pub dispositivo: u8,
    pub funcion: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
    pub header_type: u8,
}

impl InfoPci {
    /// Convierte la info en el `DeviceFunction` que las API de `virtio-drivers`
    /// y nuestros `leer`/`escribir`/`linea_irq` consumen.
    ///
    /// `#[allow(dead_code)]` hasta X2.
    #[allow(dead_code)]
    pub fn device_function(&self) -> DeviceFunction {
        DeviceFunction {
            bus: self.bus,
            device: self.dispositivo,
            function: self.funcion,
        }
    }
}

/// Lee la identidad de un slot `(bus, dispositivo, funcion)`. Devuelve `None`
/// si el slot esta vacio (vendor_id = 0xFFFF, valor convencional del bus PCI
/// para «aqui no hay nadie»).
///
/// `#[allow(dead_code)]` hasta X2.
#[allow(dead_code)]
pub fn leer_info(bus: u8, dispositivo: u8, funcion: u8) -> Option<InfoPci> {
    let device_function = DeviceFunction {
        bus,
        device: dispositivo,
        function: funcion,
    };
    let id = leer(device_function, 0x00);
    let vendor_id = (id & 0xFFFF) as u16;
    if vendor_id == 0xFFFF {
        return None;
    }
    let device_id = ((id >> 16) & 0xFFFF) as u16;
    let clase_reg = leer(device_function, 0x08);
    let revision = (clase_reg & 0xFF) as u8;
    let prog_if = ((clase_reg >> 8) & 0xFF) as u8;
    let subclass = ((clase_reg >> 16) & 0xFF) as u8;
    let class_code = ((clase_reg >> 24) & 0xFF) as u8;
    let header_reg = leer(device_function, 0x0C);
    let header_type = ((header_reg >> 16) & 0xFF) as u8;
    Some(InfoPci {
        bus,
        dispositivo,
        funcion,
        vendor_id,
        device_id,
        class_code,
        subclass,
        prog_if,
        revision,
        header_type,
    })
}

/// Recorre el bus PCI entero invocando `visita` por cada dispositivo
/// presente. Respeta el bit de multifuncion del `header_type` (bit 7): si
/// la funcion 0 no lo trae, las funciones 1-7 se omiten — son inexistentes.
///
/// La crate `virtio-drivers` tiene su propia `enumerate_bus(bus)` que
/// resuelve lo mismo para vendor 0x1AF4, pero quedaria atada al ecosistema
/// virtio. Esta enumeracion es vendor-agnostica.
///
/// `#[allow(dead_code)]` hasta X2.
#[allow(dead_code)]
pub fn enumerar(mut visita: impl FnMut(&InfoPci)) {
    for bus in 0..=255u8 {
        for dispositivo in 0..32u8 {
            let Some(info0) = leer_info(bus, dispositivo, 0) else {
                continue;
            };
            visita(&info0);
            if info0.header_type & 0x80 != 0 {
                for funcion in 1..8u8 {
                    if let Some(info) = leer_info(bus, dispositivo, funcion) {
                        visita(&info);
                    }
                }
            }
        }
    }
}

/// Lista todos los dispositivos PCI cuya tripla `(class, subclass, prog_if)`
/// coincide con `triple`. Atajo sobre `enumerar` — los drivers nuevos
/// (XHCI, NVMe, NIC) lo invocan al fundarse para descubrir su hardware.
///
/// `#[allow(dead_code)]` hasta X2.
#[allow(dead_code)]
pub fn enumerar_por_clase(triple: (u8, u8, u8)) -> Vec<InfoPci> {
    let (clase, subclase, prog_if) = triple;
    let mut hallazgos = Vec::new();
    enumerar(|info| {
        if info.class_code == clase && info.subclass == subclase && info.prog_if == prog_if {
            hallazgos.push(*info);
        }
    });
    hallazgos
}

/// Triplas `(class_code, subclass, prog_if)` de las clases PCI que el kernel
/// necesita descubrir. Vienen de la Intel PCI Class Code list (capitulo
/// «Class Codes» de la PCI Local Bus Specification).
///
/// `#[allow(dead_code)]` hasta que cada driver consuma su tripla. USB_XHCI
/// entra en X2; NVMe + NIC son hitos posteriores.
#[allow(dead_code)]
pub mod clases {
    /// USB xHCI 1.0+ — el controlador host de USB 3.x. El driver de almacenamiento
    /// USB y el de HID se apoyan sobre este.
    pub const USB_XHCI: (u8, u8, u8) = (0x0C, 0x03, 0x30);

    /// USB EHCI 1.0 — controlador host USB 2.0. Algunas placas viejas solo
    /// exponen EHCI; XHCI las soporta nativamente en modo 2.0, pero EHCI
    /// puede ser util como camino de bajo riesgo en hardware muy legacy.
    pub const USB_EHCI: (u8, u8, u8) = (0x0C, 0x03, 0x20);

    /// USB OHCI 1.0 — controlador USB 1.1 antiguo (no-Intel).
    pub const USB_OHCI: (u8, u8, u8) = (0x0C, 0x03, 0x10);

    /// USB UHCI — controlador USB 1.1 de Intel.
    pub const USB_UHCI: (u8, u8, u8) = (0x0C, 0x03, 0x00);

    /// NVMe SSD sobre PCIe. Storage moderno; nos interesa cuando wawa pueda
    /// arrancar desde discos internos (mas alla del USB del MVP).
    pub const NVME: (u8, u8, u8) = (0x01, 0x08, 0x02);

    /// AHCI SATA — controlador SATA estandar para discos rotativos / SSDs SATA.
    pub const AHCI_SATA: (u8, u8, u8) = (0x01, 0x06, 0x01);

    /// Controlador Ethernet generico. Las NICs reales (Realtek, Intel,
    /// Broadcom) se identifican por vendor/device; clase solo dice «es una
    /// NIC». Util para descubrir QUE hay antes de elegir driver especifico.
    pub const NIC_ETHERNET: (u8, u8, u8) = (0x02, 0x00, 0x00);
}

/// Acceso al espacio de configuracion PCI por puertos de E/S — el mecanismo #1.
/// Es un tipo sin estado: toda la informacion viaja en cada llamada.
pub struct CamPuertos;

impl ConfigurationAccess for CamPuertos {
    fn read_word(&self, device_function: DeviceFunction, register_offset: u8) -> u32 {
        leer(device_function, register_offset)
    }

    fn write_word(&mut self, device_function: DeviceFunction, register_offset: u8, data: u32) {
        escribir(device_function, register_offset, data);
    }

    unsafe fn unsafe_clone(&self) -> Self {
        // `CamPuertos` no tiene estado: clonarlo es trivial y sin riesgo.
        CamPuertos
    }
}
