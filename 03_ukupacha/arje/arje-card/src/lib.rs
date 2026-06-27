//! `ente-card` — alias histórico de [`card_core`].
//!
//! Mantenido como compatibilidad para los crates `ente-*` del Init que
//! importan `EntityCard`, `Capability`, `Payload`, etc. La fuente de verdad
//! del schema vive en [`card_core`]; este crate sólo re-exporta los tipos
//! bajo sus nombres legacy:
//!
//! - `EntityCard` ≡ [`card_core::Card`]
//! - El resto de tipos conservan el mismo nombre.
//!
//! Toda lógica nueva debe consumir directamente `card_core`.

#![forbid(unsafe_code)]

pub use card_core::{
    AttestPolicy,
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
    Priority,
    ResourceLimits,
    SomaSpec,
    Supervision,
    WireCard,
    CARD_SCHEMA_VERSION,
};
