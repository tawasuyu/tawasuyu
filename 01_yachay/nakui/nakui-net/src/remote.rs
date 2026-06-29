//! [`RemoteBackend`] — un `MetaBackend` que corre contra un servidor
//! remoto.
//!
//! Es el cliente que la UI usa para hablar con un `nakui-server`: implementa
//! el mismo contrato [`MetaBackend`] que el `NakuiBackend` co-locado, así que
//! el widget no cambia. Por dentro mantiene una **proyección local**
//! (`MemoryStore`) que pone al día con el catch-up inicial y con cada commit
//! difundido; las escrituras viajan como [`Intent`] al escritor autoritativo,
//! que es quien valida, ordena y corre los morfismos (el cliente no tiene
//! executors).

use std::sync::mpsc::Receiver;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;
use uuid::Uuid;

use nahual_meta_runtime::{MetaBackend, WriteOutcome};
use nakui_core::store::{MemoryStore, Store};
use nakui_sync::{apply_commit, Commit, Intent, Transport};

use crate::{CardNetTransport, ErrorNet};

/// Plazo para el snapshot de catch-up al conectar.
const CATCHUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Backend remoto: implementa [`MetaBackend`] sobre un [`CardNetTransport`]
/// con una proyección local sincronizada por delta.
pub struct RemoteBackend {
    transport: CardNetTransport,
    proyeccion: Mutex<MemoryStore>,
    rx: Receiver<Commit>,
}

impl RemoteBackend {
    /// Conecta a un `nakui-server` por su multiaddr dialable y trae el estado
    /// actual (catch-up). Bloquea hasta engancharse y recibir el snapshot.
    pub fn connect(server_addr: &str) -> Result<Self, ErrorNet> {
        let transport = CardNetTransport::connect(server_addr)?;
        let rx = transport.subscribe();
        let backend = RemoteBackend {
            transport,
            proyeccion: Mutex::new(MemoryStore::new()),
            rx,
        };
        backend.catchup();
        Ok(backend)
    }

    /// Espera el snapshot inicial (primer commit difundido) y lo aplica, de
    /// modo que `connect` devuelva un backend ya poblado. Un servidor vacío
    /// igual manda un snapshot (con 0 records), así que no cuelga de más.
    fn catchup(&self) {
        if let Ok(commit) = self.rx.recv_timeout(CATCHUP_TIMEOUT) {
            self.aplicar(&commit);
        }
        self.drenar();
    }

    /// Aplica todos los commits difundidos pendientes a la proyección.
    fn drenar(&self) {
        if let Ok(mut store) = self.proyeccion.lock() {
            for commit in self.rx.try_iter() {
                let _ = apply_commit(&mut *store, &commit);
            }
        }
    }

    /// Aplica un commit puntual (idempotente por seq).
    fn aplicar(&self, commit: &Commit) {
        if let Ok(mut store) = self.proyeccion.lock() {
            let _ = apply_commit(&mut *store, commit);
        }
    }

    /// Mapa de un [`Commit`] al `WriteOutcome` que espera la UI.
    fn outcome(commit: Commit) -> WriteOutcome {
        WriteOutcome {
            id: commit.primary_id,
            changed: commit.changed,
            post_status: commit.post_status,
        }
    }
}

impl MetaBackend for RemoteBackend {
    fn list_records(&self, entity: &str) -> Vec<(Uuid, Value)> {
        self.drenar();
        let store = match self.proyeccion.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        let it = match store.iter() {
            Ok(i) => i,
            Err(_) => return Vec::new(),
        };
        let mut out: Vec<(Uuid, Value)> = it
            .filter(|(e, _, _)| e == entity)
            .map(|(_, id, v)| (id, v))
            .collect();
        out.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        out
    }

    fn load_record(&self, entity: &str, id: Uuid) -> Option<Value> {
        self.drenar();
        self.proyeccion.lock().ok()?.load(entity, id)
    }

    fn seed(
        &mut self,
        entity: &str,
        data: serde_json::Map<String, Value>,
    ) -> Result<WriteOutcome, String> {
        let commit = self.transport.submit(Intent::Seed {
            entity: entity.to_string(),
            data,
        })?;
        self.aplicar(&commit);
        Ok(Self::outcome(commit))
    }

    fn update(
        &mut self,
        entity: &str,
        id: Uuid,
        set: serde_json::Map<String, Value>,
        clear: Vec<String>,
    ) -> Result<WriteOutcome, String> {
        let commit = self.transport.submit(Intent::Update {
            entity: entity.to_string(),
            id,
            set,
            clear,
        })?;
        self.aplicar(&commit);
        Ok(Self::outcome(commit))
    }

    fn delete(&mut self, entity: &str, id: Uuid) -> Result<WriteOutcome, String> {
        let commit = self.transport.submit(Intent::Delete {
            entity: entity.to_string(),
            id,
        })?;
        self.aplicar(&commit);
        Ok(Self::outcome(commit))
    }

    fn morphism_n(
        &mut self,
        module_id: &str,
        name: &str,
        inputs: Vec<(String, Uuid)>,
        params: Value,
    ) -> Result<WriteOutcome, String> {
        let commit = self.transport.submit(Intent::Morphism {
            module_id: module_id.to_string(),
            name: name.to_string(),
            inputs,
            params,
        })?;
        self.aplicar(&commit);
        Ok(Self::outcome(commit))
    }
}
