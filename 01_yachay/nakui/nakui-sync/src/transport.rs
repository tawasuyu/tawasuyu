//! El transporte: cómo un cliente entrega una [`Intent`] al escritor
//! autoritativo y recibe los [`Commit`]s resultantes.
//!
//! Hoy hay un solo impl, [`LocalTransport`] (in-process). El path en red
//! (card-net) será otro impl del mismo trait — los clientes no cambian.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use crate::intent::{Commit, Intent};
use crate::writer::Writer;

/// Frontera cliente ↔ escritor. Un cliente `submit`-ea intenciones y se
/// `subscribe`-be a los commits de TODOS los clientes para mantener su
/// proyección al día.
pub trait Transport {
    /// Entrega una intención al escritor autoritativo. Bloqueante: cuando
    /// vuelve `Ok`, el commit ya es durable en el log. Un `Err` es un
    /// rechazo de validación (el estado quedó intacto).
    fn submit(&self, intent: Intent) -> Result<Commit, String>;

    /// Suscribe a todos los commits autoritativos posteriores. El cliente
    /// drena el receiver y aplica cada commit con [`crate::apply_commit`].
    fn subscribe(&self) -> mpsc::Receiver<Commit>;
}

/// Transporte in-process: el escritor vive en el mismo proceso, detrás de
/// un `Mutex` que serializa los commits (la garantía de escritor único).
///
/// Clonar el `LocalTransport` da otro cliente contra el MISMO escritor —
/// así se modela el multi-usuario concurrente sin red: cada clon es un
/// "asiento", todos compiten por el lock del escritor, que les impone
/// orden total.
#[derive(Clone)]
pub struct LocalTransport {
    writer: Arc<Mutex<Writer>>,
    subscribers: Arc<Mutex<Vec<mpsc::Sender<Commit>>>>,
}

impl LocalTransport {
    pub fn new(writer: Writer) -> Self {
        Self {
            writer: Arc::new(Mutex::new(writer)),
            subscribers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Handle al escritor para setup co-locado (handle del store para
    /// reads, lookup de executors). NO es el path de escritura — eso es
    /// `submit`.
    pub fn writer(&self) -> Arc<Mutex<Writer>> {
        self.writer.clone()
    }

    /// Difunde un commit a todos los subscribers. Subscribers caídos
    /// (receiver dropeado) se purgan. No difunde no-ops (sin entries).
    fn broadcast(&self, commit: &Commit) {
        if commit.entries.is_empty() {
            return;
        }
        let Ok(mut subs) = self.subscribers.lock() else {
            return;
        };
        subs.retain(|tx| tx.send(commit.clone()).is_ok());
    }
}

impl Transport for LocalTransport {
    fn submit(&self, intent: Intent) -> Result<Commit, String> {
        let commit = {
            let mut w = self.writer.lock().map_err(|_| "writer mutex envenenado".to_string())?;
            w.commit(intent)?
        };
        self.broadcast(&commit);
        Ok(commit)
    }

    fn subscribe(&self) -> mpsc::Receiver<Commit> {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut subs) = self.subscribers.lock() {
            subs.push(tx);
        }
        rx
    }
}
