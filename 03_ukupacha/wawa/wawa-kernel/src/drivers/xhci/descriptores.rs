// =============================================================================
//  renaser :: kernel/src/drivers/xhci/descriptores.rs — parser de USB descriptors
// -----------------------------------------------------------------------------
//  Tras leer el Configuration Descriptor con GET_DESCRIPTOR(Config, 0, N), el
//  blob viene como una secuencia plana de descriptors de longitud variable:
//
//    [Length(1B)][Type(1B)][...payload...]
//
//  Cada descriptor empieza con `bLength` y `bDescriptorType`; el siguiente
//  arranca a `&blob[offset + bLength]`. Tipos relevantes:
//
//    * 2 = Configuration  — primer descriptor; bytes 2-3 = wTotalLength.
//    * 4 = Interface      — InterfaceNumber, Class, SubClass, Protocol.
//    * 5 = Endpoint       — EndpointAddress, Attributes, MaxPacketSize.
//
//  Este modulo es vendor-agnostico — parsea el byte array, no habla con el
//  controlador.
// =============================================================================

use alloc::vec::Vec;

/// Tipos de descriptor USB que nos interesan. Hay mas (String=3, Device
/// Qualifier=6, etc) pero no los usamos en X2d.
pub mod tipo {
    pub const CONFIGURATION: u8 = 2;
    pub const INTERFACE: u8 = 4;
    pub const ENDPOINT: u8 = 5;
}

/// Tipos de transferencia (bits 0-1 de bmAttributes en Endpoint Descriptor).
/// `CONTROL`/`ISOCRONO` completan el espacio aunque hoy solo discriminamos BULK
/// (USB-MS) e INTERRUPT (HID).
#[allow(dead_code)]
pub mod transferencia {
    pub const CONTROL: u8 = 0;
    pub const ISOCRONO: u8 = 1;
    pub const BULK: u8 = 2;
    pub const INTERRUPT: u8 = 3;
}

/// Direccion del endpoint (bit 7 de bEndpointAddress).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direccion {
    Out,
    In,
}

/// Resumen plano de un endpoint para los drivers de clase (USB-MS, HID).
#[derive(Clone, Copy, Debug)]
pub struct Endpoint {
    /// Numero de endpoint, 1..=15.
    pub numero: u8,
    /// Direccion IN o OUT (bit 7 de bEndpointAddress).
    pub direccion: Direccion,
    /// Tipo de transferencia (Control/Iso/Bulk/Int).
    pub tipo_transferencia: u8,
    /// Max packet size del endpoint (wMaxPacketSize, low 11 bits son MPS).
    pub max_packet_size: u16,
    /// Polling interval para Interrupt/Iso (bInterval).
    pub intervalo: u8,
}

impl Endpoint {
    /// Device Context Index (DCI): formula del xHCI spec para indexar
    /// endpoints en el Device Context y los doorbells.
    ///   DCI = 2 * numero + (1 si IN else 0). EP0 control = DCI 1.
    pub fn dci(&self) -> u8 {
        let bias = match self.direccion {
            Direccion::In => 1,
            Direccion::Out => 0,
        };
        2 * self.numero + bias
    }
}

/// Resumen de una interface USB. Incluye sus endpoints.
#[derive(Clone, Debug)]
pub struct Interface {
    pub numero: u8,
    pub alt_setting: u8,
    pub clase: u8,
    pub subclase: u8,
    pub protocolo: u8,
    pub endpoints: Vec<Endpoint>,
}

/// Resumen del Configuration Descriptor: configuracion + lista de interfaces.
#[derive(Clone, Debug)]
pub struct Configuracion {
    pub valor: u8, // bConfigurationValue para SET_CONFIGURATION.
    pub interfaces: Vec<Interface>,
}

/// Parsea el blob crudo del Configuration Descriptor. Devuelve `None` si la
/// estructura no concuerda (longitudes invalidas, descriptor truncado).
pub fn parsear(blob: &[u8]) -> Option<Configuracion> {
    if blob.len() < 9 || blob[1] != tipo::CONFIGURATION {
        return None;
    }
    let valor = blob[5];

    let mut interfaces: Vec<Interface> = Vec::new();
    let mut iface_actual: Option<Interface> = None;
    let mut off = blob[0] as usize; // saltar Configuration Descriptor

    while off + 2 <= blob.len() {
        let len = blob[off] as usize;
        let tipo_desc = blob[off + 1];
        if len == 0 || off + len > blob.len() {
            break; // descriptor truncado
        }
        match tipo_desc {
            tipo::INTERFACE if len >= 9 => {
                if let Some(iface) = iface_actual.take() {
                    interfaces.push(iface);
                }
                iface_actual = Some(Interface {
                    numero: blob[off + 2],
                    alt_setting: blob[off + 3],
                    clase: blob[off + 5],
                    subclase: blob[off + 6],
                    protocolo: blob[off + 7],
                    endpoints: Vec::new(),
                });
            }
            tipo::ENDPOINT if len >= 7 => {
                if let Some(iface) = iface_actual.as_mut() {
                    let address = blob[off + 2];
                    let attrs = blob[off + 3];
                    let mps = u16::from_le_bytes([blob[off + 4], blob[off + 5]]);
                    let intervalo = blob[off + 6];
                    iface.endpoints.push(Endpoint {
                        numero: address & 0x0F,
                        direccion: if address & 0x80 != 0 {
                            Direccion::In
                        } else {
                            Direccion::Out
                        },
                        tipo_transferencia: attrs & 0x03,
                        max_packet_size: mps & 0x07FF,
                        intervalo,
                    });
                }
            }
            _ => {} // descriptor irrelevante (HID, Class-Specific, etc).
        }
        off += len;
    }
    if let Some(iface) = iface_actual.take() {
        interfaces.push(iface);
    }

    Some(Configuracion { valor, interfaces })
}

/// Atajo: busca una interface USB-MS Bulk-Only Transport y devuelve sus dos
/// endpoints bulk (IN y OUT) si los tiene. Clase 0x08, subclase 0x06 (SCSI),
/// protocolo 0x50 (BBB).
#[allow(dead_code)]
pub fn buscar_usb_ms(config: &Configuracion) -> Option<(&Interface, Endpoint, Endpoint)> {
    for iface in &config.interfaces {
        if iface.clase != 0x08 || iface.subclase != 0x06 || iface.protocolo != 0x50 {
            continue;
        }
        let mut bulk_in: Option<Endpoint> = None;
        let mut bulk_out: Option<Endpoint> = None;
        for ep in &iface.endpoints {
            if ep.tipo_transferencia != transferencia::BULK {
                continue;
            }
            match ep.direccion {
                Direccion::In => bulk_in = Some(*ep),
                Direccion::Out => bulk_out = Some(*ep),
            }
        }
        if let (Some(i), Some(o)) = (bulk_in, bulk_out) {
            return Some((iface, i, o));
        }
    }
    None
}

/// Atajo: busca una interface HID de tipo RATON (boot protocol) y devuelve su
/// endpoint de interrupcion IN. Clase 0x03 (HID), protocolo de interface 0x02
/// (Mouse). La subclase 0x01 (Boot Interface) es lo habitual; no la exigimos
/// estrictamente porque algunos ratones la dejan en 0 aunque soporten boot
/// protocol — basta con que el protocolo diga «mouse» y haya un interrupt IN.
///
/// Devuelve `(interface, interrupt_in_endpoint)`. El endpoint de interrupcion
/// IN es por donde el raton entrega sus reportes de 3-4 bytes (botones, dx, dy
/// [, rueda]). Es lo unico que necesita el driver `xhci::hid`.
pub fn buscar_raton_hid(config: &Configuracion) -> Option<(&Interface, Endpoint)> {
    for iface in &config.interfaces {
        // Clase 0x03 = HID; protocolo 0x02 = Mouse (tabla USB HID 1.11 §4.3).
        if iface.clase != 0x03 || iface.protocolo != 0x02 {
            continue;
        }
        for ep in &iface.endpoints {
            if ep.tipo_transferencia == transferencia::INTERRUPT
                && ep.direccion == Direccion::In
            {
                return Some((iface, *ep));
            }
        }
    }
    None
}
