//! `ente-card` — alias histórico de [`brahman_card`].
//!
//! Mantenido como compatibilidad para los crates `ente-*` del Init que
//! importan `EntityCard`, `Capability`, `Payload`, etc. La fuente de verdad
//! del schema vive en [`brahman_card`]; este crate sólo re-exporta los tipos
//! bajo sus nombres legacy:
//!
//! - `EntityCard` ≡ [`brahman_card::Card`]
//! - El resto de tipos conservan el mismo nombre.
//!
//! Toda lógica nueva debe consumir directamente `brahman_card`.

#![forbid(unsafe_code)]

pub use brahman_card::{
    Capability,
    CardError,
    Card as EntityCard,
    CgroupSpec,
    DeviceClass,
    InterfaceId,
    LegacyFacade,
    NamespaceSet,
    NetlinkFamily,
    Payload,
    ResourceLimits,
    SomaSpec,
    Supervision,
    CARD_SCHEMA_VERSION,
};
