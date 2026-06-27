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
    wayland_floor,
    WAYLAND_FLOOR_INTERFACE,
    CgroupSpec,
    DepContract,
    DeviceClass,
    FsPolicy,
    InterfaceId,
    LegacyFacade,
    NamespaceSet,
    NetlinkFamily,
    NetworkingPolicy,
    Payload,
    Permissions,
    RunAs,
    Priority,
    ResourceLimits,
    SomaSpec,
    Supervision,
    UnmetContract,
    WireCard,
    CARD_SCHEMA_VERSION,
};
