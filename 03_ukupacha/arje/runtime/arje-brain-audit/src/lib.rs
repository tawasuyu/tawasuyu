//! arje-brain-audit — accountability del brain.
//!
//! Audit log con cadena de hashes anclada al content-addressed storage
//! (`arje-cas`). Permite verificar la integridad de la historia de
//! decisiones del brain y reconstruir el estado vía replay.

pub mod audit;

pub use audit::{
    AuditAction, AuditEntry, AuditHeadPointer, AuditLog, ReplayReport,
    VerificationReport, collect_chain_from_cas, reachable_from_head,
    replay_chain, verify_chain_from_cas,
};
