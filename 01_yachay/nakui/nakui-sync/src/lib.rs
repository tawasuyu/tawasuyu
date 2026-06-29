//! `nakui-sync` — núcleo de sincronización multi-cliente de Nakui.
//!
//! Extiende nakui a **múltiples usuarios concurrentes** sin abandonar su
//! arquitectura event-sourced. La pieza central es el [`Writer`]: el
//! escritor autoritativo, dueño único del `EventLog`. Toda mutación entra
//! como [`Intent`], se ordena (un `seq` monótono) y sale como [`Commit`]
//! cuyas entradas se difunden a todos los clientes.
//!
//! Capas:
//! - [`Intent`] / [`Commit`] — el protocolo (serializable, listo para red).
//! - [`Writer`] — el escritor autoritativo (validación + log + store).
//! - [`Transport`] / [`LocalTransport`] — la frontera cliente↔escritor.
//!   Hoy in-process; mañana card-net detrás del mismo trait.
//! - [`apply_commit`] — cómo un cliente pone su proyección al día.
//!
//! Por qué un solo escritor y no multi-master/CRDT: un ERP exige
//! serializabilidad estricta (la partida doble se rompería bajo
//! consistencia eventual). El escritor único da orden total gratis; el
//! tráfico de lectura escala aparte, contra proyecciones locales.

mod intent;
mod transport;
mod writer;

pub use intent::{Commit, Intent};
pub use transport::{LocalTransport, Transport};
pub use writer::{maybe_compact_log, snapshot_path_for, OpenStatus, Writer};

use nakui_core::event_log::LogEntry;
use nakui_core::store::{Store, StoreError};

/// Aplica un [`Commit`] a una proyección local. Idempotente por `seq`:
/// entradas con `seq <= last_applied` se ignoran, así re-entregar el mismo
/// commit (o uno solapado) es inofensivo.
///
/// Es lo que corre un cliente remoto al recibir cada commit del escritor:
/// `Seed` siembra el record, `Morphism` aplica sus ops — exactamente la
/// misma lógica que `replay`, sólo que sobre el tail en vivo en vez del
/// log en disco.
pub fn apply_commit<S: Store>(store: &mut S, commit: &Commit) -> Result<(), StoreError> {
    let last_applied = store.last_applied_seq()?;
    for entry in &commit.entries {
        if let Some(last) = last_applied {
            if entry.seq() <= last {
                continue;
            }
        }
        match entry {
            LogEntry::Seed { entity, id, data, .. } => store.seed(entity, *id, data.clone()),
            LogEntry::Morphism { ops, .. } => store.apply(ops)?,
        }
        // Best-effort: el marcador sólo ahorra reaplicaciones; un fallo acá
        // no afecta corrección (se vuelve a evaluar en el próximo commit).
        let _ = store.set_last_applied_seq(entry.seq());
    }
    Ok(())
}
