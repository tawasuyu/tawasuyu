//! Constantes públicas del Ente echo. Lib aparte del bin para que `busctl`
//! y otros consumidores puedan importar el InterfaceId sin enlazar el binario.

use arje_card::{Capability, InterfaceId};

/// UUID estable del interface "echo". Genera nuevo por sed si forkeas.
pub const ECHO_IFACE: InterfaceId = InterfaceId([
    0xec, 0x40, 0xa1, 0x00, 0x00, 0x00, 0x00, 0x01,
    0x80, 0x00, 0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe,
]);
pub const ECHO_VERSION: u16 = 1;

pub fn echo_capability() -> Capability {
    Capability::Endpoint {
        interface: ECHO_IFACE,
        version: ECHO_VERSION,
    }
}
