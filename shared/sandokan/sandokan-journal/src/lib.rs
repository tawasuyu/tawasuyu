//! sandokan-journal — el registro durable del plano de control.
//!
//! sandokan sabe qué unidades corren, pero ese saber era **efímero**: vivía en
//! memoria y se perdía al reiniciar el orquestador. Este crate le da un
//! **journal append-only** con `seq` monótono, snapshot + compactación y
//! **replay-para-reconstruir** — el mismo patrón que `nakui_core::event_log`,
//! y hermano de `arje-brain-audit` (que ancla la cadena de decisiones del init
//! al CAS). No copia el crate de nakui: sus `LogEntry` (`Seed`/`Morphism` +
//! `FieldOp` + Rhai) son un modelo documental que no calza con los eventos
//! **tipados** de sandokan (`LifecycleEvent`). Se reusa la *arquitectura*, no el
//! acoplamiento.
//!
//! Lo que el journal registra es el **stream de eventos de ciclo de vida** que el
//! `Engine` ya emite (más un registro de *procedencia* de qué se pidió correr).
//! Replayándolo se reconstruye el [`ControlPlaneState`] vivo: qué unidades
//! existen, en qué estado, con qué PID y cuántos restarts acumulados. Un
//! `Journal::open` tras un crash devuelve exactamente el estado previo.
//!
//! No es un motor de replicación ni de consenso: registra un stream local. La
//! convergencia entre varios sandokan es una capa CRDT aparte (ver
//! `shared/PLAN-CRUCES.md`, F-A).

mod backend;

pub use backend::FileBackend;

use sandokan_core::LifecycleEvent;
use sandokan_lifecycle::LifecycleState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::SystemTime;
use thiserror::Error;
use ulid::Ulid;

/// Errores del journal.
#[derive(Debug, Error)]
pub enum JournalError {
    /// I/O del backend (archivo, etc.).
    #[error("journal io: {0}")]
    Io(String),
    /// (De)serialización de una entrada o snapshot.
    #[error("journal serde: {0}")]
    Serde(String),
}

/// Lo que se anexa al journal. Dos formas: el evento de ciclo de vida que el
/// `Engine` emite, y un registro de **procedencia** (qué Card se pidió correr y
/// con qué rótulo) para que el journal sea auditable — `Spawned` por sí solo
/// pierde el "quién".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JournalRecord {
    /// Se pidió orquestar una unidad. Ancla el `card_id` a un rótulo legible
    /// antes (o junto) de su `Spawned`.
    Intent {
        card_id: Ulid,
        /// Rótulo legible de la Card (nombre de la unidad).
        label: String,
    },
    /// Un evento de ciclo de vida emitido por el `Engine`.
    Lifecycle(LifecycleEvent),
}

impl JournalRecord {
    /// El `card_id` al que refiere el registro.
    pub fn card_id(&self) -> Ulid {
        match self {
            JournalRecord::Intent { card_id, .. } => *card_id,
            JournalRecord::Lifecycle(ev) => ev.card_id(),
        }
    }
}

/// Una entrada del journal: un registro con su `seq` monótono y el instante en
/// que se anexó.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Secuencia monótona (1-based), asignada por el [`Journal`].
    pub seq: u64,
    /// Cuándo se anexó.
    pub at: SystemTime,
    /// El contenido.
    pub record: JournalRecord,
}

/// Estado vivo de una unidad, reconstruido replayando el journal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitState {
    /// Estado de ciclo de vida actual.
    pub state: LifecycleState,
    /// PID si aplica (`None` para Wasm/virtual o antes del primer `Spawned`).
    pub pid: Option<i32>,
    /// Rótulo legible, si un `Intent` lo ancló.
    pub label: Option<String>,
    /// Cuántas veces se encarnó (`Spawned`). El primer arranque es 1.
    pub spawns: u32,
    /// `seq` de la primera entrada que tocó esta unidad.
    pub first_seq: u64,
    /// `seq` de la última entrada que la tocó.
    pub last_seq: u64,
}

impl UnitState {
    fn new(seq: u64) -> Self {
        Self {
            state: LifecycleState::Pending,
            pid: None,
            label: None,
            spawns: 0,
            first_seq: seq,
            last_seq: seq,
        }
    }

    /// Restarts acumulados: un `Spawned` más allá del primero es un restart.
    pub fn restarts(&self) -> u32 {
        self.spawns.saturating_sub(1)
    }
}

/// El estado del plano de control: el conjunto de unidades conocidas y su
/// estado, materializado desde el journal.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ControlPlaneState {
    /// Unidades por `card_id`.
    pub units: BTreeMap<Ulid, UnitState>,
}

impl ControlPlaneState {
    /// Unidades que no están en estado terminal (las que el orquestador cree
    /// vivas).
    pub fn active(&self) -> impl Iterator<Item = (&Ulid, &UnitState)> {
        self.units.iter().filter(|(_, u)| !u.state.is_terminal())
    }

    /// Estado de una unidad puntual.
    pub fn get(&self, card_id: &Ulid) -> Option<&UnitState> {
        self.units.get(card_id)
    }

    /// Aplica una entrada, mutando el estado. Es la función de replay: aplicar
    /// las entradas en orden reconstruye el estado exacto.
    fn apply(&mut self, entry: &JournalEntry) {
        let card_id = entry.record.card_id();
        let unit = self
            .units
            .entry(card_id)
            .or_insert_with(|| UnitState::new(entry.seq));
        match &entry.record {
            JournalRecord::Intent { label, .. } => {
                unit.label = Some(label.clone());
            }
            JournalRecord::Lifecycle(ev) => match ev {
                LifecycleEvent::Spawned { pid, .. } => {
                    unit.spawns += 1;
                    unit.pid = *pid;
                    // (re)arranque: vuelve a Pending hasta el próximo StateChanged.
                    unit.state = LifecycleState::Pending;
                }
                LifecycleEvent::StateChanged { state, .. } => {
                    unit.state = state.clone();
                }
                LifecycleEvent::Exited { state, .. } => {
                    unit.state = state.clone();
                }
            },
        }
        unit.last_seq = entry.seq;
    }
}

/// Un snapshot del estado del plano de control hasta un `seq` dado. Permite
/// compactar el journal sin perder el estado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Todas las entradas con `seq <= through_seq` están plegadas en `state`.
    pub through_seq: u64,
    /// El estado reconstruido hasta `through_seq`.
    pub state: ControlPlaneState,
}

/// Backend de almacenamiento del journal. El motor es agnóstico del transporte:
/// memoria para tests, archivo jsonl para producción, y en el futuro un backend
/// direccionado por contenido (CAS) para el espejo con arje/wawa.
pub trait JournalBackend {
    /// Anexa una entrada (durable en backends persistentes).
    fn append(&mut self, entry: &JournalEntry) -> Result<(), JournalError>;
    /// Todas las entradas, en orden de `seq`.
    fn entries(&self) -> Result<Vec<JournalEntry>, JournalError>;
    /// Reescribe el log completo (usado por la compactación).
    fn replace_all(&mut self, entries: &[JournalEntry]) -> Result<(), JournalError>;
    /// Persiste un snapshot. Default no-op: los backends transitorios lo
    /// mantienen sólo en memoria.
    fn store_snapshot(&mut self, _snap: &Snapshot) -> Result<(), JournalError> {
        Ok(())
    }
    /// Carga el snapshot persistido, si hay. Default `None`.
    fn load_snapshot(&self) -> Result<Option<Snapshot>, JournalError> {
        Ok(None)
    }
}

/// Backend en memoria (tests, engines transitorios).
#[derive(Debug, Default)]
pub struct MemoryBackend {
    entries: Vec<JournalEntry>,
    snapshot: Option<Snapshot>,
}

impl MemoryBackend {
    /// Backend vacío.
    pub fn new() -> Self {
        Self::default()
    }

    /// Siembra un backend con entradas ya conocidas (para simular una reapertura
    /// tras crash desde un log persistido).
    pub fn from_entries(entries: Vec<JournalEntry>) -> Self {
        Self {
            entries,
            snapshot: None,
        }
    }
}

impl JournalBackend for MemoryBackend {
    fn append(&mut self, entry: &JournalEntry) -> Result<(), JournalError> {
        self.entries.push(entry.clone());
        Ok(())
    }

    fn entries(&self) -> Result<Vec<JournalEntry>, JournalError> {
        Ok(self.entries.clone())
    }

    fn replace_all(&mut self, entries: &[JournalEntry]) -> Result<(), JournalError> {
        self.entries = entries.to_vec();
        Ok(())
    }

    fn store_snapshot(&mut self, snap: &Snapshot) -> Result<(), JournalError> {
        self.snapshot = Some(snap.clone());
        Ok(())
    }

    fn load_snapshot(&self) -> Result<Option<Snapshot>, JournalError> {
        Ok(self.snapshot.clone())
    }
}

/// El journal: envuelve un backend, asigna `seq` monótonos y mantiene el estado
/// vivo materializado. `open` lo reconstruye desde lo persistido (snapshot +
/// cola de entradas).
#[derive(Debug)]
pub struct Journal<B: JournalBackend> {
    backend: B,
    next_seq: u64,
    state: ControlPlaneState,
}

impl<B: JournalBackend> Journal<B> {
    /// Abre el journal sobre un backend, reconstruyendo el estado: parte del
    /// snapshot (si hay) y replaya las entradas posteriores. Tras un crash,
    /// devuelve exactamente el estado previo.
    pub fn open(backend: B) -> Result<Self, JournalError> {
        let snap = backend.load_snapshot()?;
        let base_seq = snap.as_ref().map(|s| s.through_seq).unwrap_or(0);
        let mut state = snap.map(|s| s.state).unwrap_or_default();
        let mut next_seq = base_seq + 1;
        for entry in backend.entries()? {
            if entry.seq <= base_seq {
                continue; // ya plegada en el snapshot
            }
            state.apply(&entry);
            if entry.seq >= next_seq {
                next_seq = entry.seq + 1;
            }
        }
        Ok(Self {
            backend,
            next_seq: next_seq.max(1),
            state,
        })
    }

    /// Anexa un registro con el instante actual. Devuelve el `seq` asignado.
    pub fn record(&mut self, record: JournalRecord) -> Result<u64, JournalError> {
        self.record_at(record, SystemTime::now())
    }

    /// Anexa un registro con un instante explícito (tests deterministas).
    pub fn record_at(
        &mut self,
        record: JournalRecord,
        at: SystemTime,
    ) -> Result<u64, JournalError> {
        let seq = self.next_seq;
        let entry = JournalEntry { seq, at, record };
        self.backend.append(&entry)?;
        self.state.apply(&entry);
        self.next_seq += 1;
        Ok(seq)
    }

    /// Azúcar: anexa un evento de ciclo de vida.
    pub fn record_lifecycle(&mut self, event: LifecycleEvent) -> Result<u64, JournalError> {
        self.record(JournalRecord::Lifecycle(event))
    }

    /// Azúcar: ancla la procedencia de una unidad (qué se pidió correr).
    pub fn record_intent(
        &mut self,
        card_id: Ulid,
        label: impl Into<String>,
    ) -> Result<u64, JournalError> {
        self.record(JournalRecord::Intent {
            card_id,
            label: label.into(),
        })
    }

    /// El estado del plano de control vivo.
    pub fn state(&self) -> &ControlPlaneState {
        &self.state
    }

    /// El próximo `seq` que se asignaría.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Un snapshot del estado hasta la última entrada anexada.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            through_seq: self.next_seq - 1,
            state: self.state.clone(),
        }
    }

    /// Compacta: persiste un snapshot y descarta las entradas ya plegadas. Un
    /// `open` posterior reconstruye el mismo estado y el mismo `next_seq`.
    pub fn compact(&mut self) -> Result<(), JournalError> {
        let snap = self.snapshot();
        self.backend.store_snapshot(&snap)?;
        self.backend.replace_all(&[])?;
        Ok(())
    }
}

impl Journal<MemoryBackend> {
    /// Un journal en memoria, vacío.
    pub fn in_memory() -> Self {
        Journal::open(MemoryBackend::new()).expect("MemoryBackend open no falla")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawned(id: Ulid, pid: i32) -> JournalRecord {
        JournalRecord::Lifecycle(LifecycleEvent::Spawned {
            card_id: id,
            pid: Some(pid),
        })
    }
    fn changed(id: Ulid, state: LifecycleState) -> JournalRecord {
        JournalRecord::Lifecycle(LifecycleEvent::StateChanged {
            card_id: id,
            state,
        })
    }
    fn exited(id: Ulid, code: i32) -> JournalRecord {
        JournalRecord::Lifecycle(LifecycleEvent::Exited {
            card_id: id,
            state: LifecycleState::Exited { code },
        })
    }

    #[test]
    fn seq_es_monotono_desde_uno() {
        let mut j = Journal::in_memory();
        let id = Ulid::new();
        assert_eq!(j.record_intent(id, "web").unwrap(), 1);
        assert_eq!(j.record(spawned(id, 42)).unwrap(), 2);
        assert_eq!(j.record(changed(id, LifecycleState::Running)).unwrap(), 3);
        assert_eq!(j.next_seq(), 4);
    }

    #[test]
    fn replay_reconstruye_el_estado_vivo() {
        let mut j = Journal::in_memory();
        let id = Ulid::new();
        j.record_intent(id, "web").unwrap();
        j.record(spawned(id, 42)).unwrap();
        j.record(changed(id, LifecycleState::Running)).unwrap();

        let u = j.state().get(&id).unwrap();
        assert_eq!(u.state, LifecycleState::Running);
        assert_eq!(u.pid, Some(42));
        assert_eq!(u.label.as_deref(), Some("web"));
        assert_eq!(u.restarts(), 0);
        assert_eq!(j.state().active().count(), 1);
    }

    #[test]
    fn recuperacion_tras_crash_devuelve_el_mismo_estado() {
        // Un journal persiste sus entradas...
        let mut j = Journal::open(MemoryBackend::new()).unwrap();
        let id = Ulid::new();
        j.record_intent(id, "db").unwrap();
        j.record(spawned(id, 7)).unwrap();
        j.record(changed(id, LifecycleState::Running)).unwrap();
        let antes = j.state().clone();
        let next_antes = j.next_seq();

        // ...el orquestador "crashea": el estado en memoria se pierde, pero el
        // log persistido sobrevive. Reabrimos desde esas mismas entradas.
        let persistidas = j.backend.entries().unwrap();
        let j2 = Journal::open(MemoryBackend::from_entries(persistidas)).unwrap();

        assert_eq!(j2.state(), &antes, "replay reconstruye el estado exacto");
        assert_eq!(j2.next_seq(), next_antes, "el seq no retrocede");
    }

    #[test]
    fn un_respawn_cuenta_como_restart() {
        let mut j = Journal::in_memory();
        let id = Ulid::new();
        j.record(spawned(id, 1)).unwrap();
        j.record(changed(id, LifecycleState::Running)).unwrap();
        j.record(exited(id, 1)).unwrap(); // crasheó
        j.record(spawned(id, 2)).unwrap(); // el supervisor lo reencarnó
        j.record(changed(id, LifecycleState::Running)).unwrap();

        let u = j.state().get(&id).unwrap();
        assert_eq!(u.spawns, 2);
        assert_eq!(u.restarts(), 1, "el segundo Spawned es un restart");
        assert_eq!(u.state, LifecycleState::Running);
        assert_eq!(u.pid, Some(2));
    }

    #[test]
    fn una_unidad_terminada_no_esta_activa() {
        let mut j = Journal::in_memory();
        let id = Ulid::new();
        j.record(spawned(id, 1)).unwrap();
        j.record(exited(id, 0)).unwrap();
        assert_eq!(j.state().active().count(), 0);
        assert!(j.state().get(&id).unwrap().state.is_terminal());
    }

    #[test]
    fn compactar_preserva_estado_y_seq() {
        let mut j = Journal::open(MemoryBackend::new()).unwrap();
        let id = Ulid::new();
        j.record_intent(id, "cache").unwrap();
        j.record(spawned(id, 9)).unwrap();
        j.record(changed(id, LifecycleState::Running)).unwrap();
        let antes = j.state().clone();
        let next_antes = j.next_seq();

        j.compact().unwrap();
        // Tras compactar, el log de entradas quedó vacío pero el snapshot tiene
        // el estado. Reabrir reconstruye idéntico.
        assert!(j.backend.entries().unwrap().is_empty());
        let j2 = Journal::open(j.backend).unwrap();
        assert_eq!(j2.state(), &antes, "el snapshot preserva el estado");
        assert_eq!(j2.next_seq(), next_antes, "el seq sobrevive a la compactación");

        // Y se puede seguir anexando sin colisión de seq.
        let mut j2 = j2;
        assert_eq!(j2.record(exited(id, 0)).unwrap(), next_antes);
    }

    #[test]
    fn compactar_y_luego_anexar_replaya_bien() {
        let mut j = Journal::open(MemoryBackend::new()).unwrap();
        let id = Ulid::new();
        j.record(spawned(id, 1)).unwrap();
        j.record(changed(id, LifecycleState::Running)).unwrap();
        j.compact().unwrap();
        j.record(exited(id, 0)).unwrap(); // entrada nueva tras el snapshot

        // Reabrir: snapshot (Running) + cola (Exited) → Exited.
        let j2 = Journal::open(j.backend).unwrap();
        assert_eq!(
            j2.state().get(&id).unwrap().state,
            LifecycleState::Exited { code: 0 }
        );
        assert_eq!(j2.state().active().count(), 0);
    }
}
