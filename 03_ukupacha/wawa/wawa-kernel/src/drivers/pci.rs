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
