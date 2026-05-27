//! Harness in-memory determinístico para correr dos `SyncSession`s
//! una contra la otra y verificar invariantes del protocolo.

use std::collections::VecDeque;

use crate::message::Message;
use crate::session::SyncSession;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncStats {
    pub challenges: usize,
    pub hellos: usize,
    pub probe_reqs: usize,
    pub probe_ress: usize,
    pub fetches: usize,
    pub delivers: usize,
    pub attest_pushes: usize,
    pub retract_pushes: usize,
    pub root_declarations: usize,
    pub dones: usize,
}

impl SyncStats {
    fn record(&mut self, m: &Message) {
        match m {
            Message::Challenge { .. } => self.challenges += 1,
            Message::Hello { .. } => self.hellos += 1,
            Message::ProbeReq { .. } => self.probe_reqs += 1,
            Message::ProbeRes { .. } => self.probe_ress += 1,
            Message::Fetch { .. } => self.fetches += 1,
            Message::Deliver { .. } => self.delivers += 1,
            Message::AttestPush { .. } => self.attest_pushes += 1,
            Message::RetractPush { .. } => self.retract_pushes += 1,
            Message::RootDeclaration { .. } => self.root_declarations += 1,
            Message::Done => self.dones += 1,
        }
    }

    pub fn total(&self) -> usize {
        self.challenges
            + self.hellos
            + self.probe_reqs
            + self.probe_ress
            + self.fetches
            + self.delivers
            + self.attest_pushes
            + self.retract_pushes
            + self.root_declarations
            + self.dones
    }
}

/// Ejecuta la sincronización entre dos sesiones hasta convergencia.
///
/// Pánico si la conversación termina sin que ambas partes alcancen
/// `is_done()` — eso sería un deadlock del protocolo y una regresión.
pub fn run_sync(a: &mut SyncSession, b: &mut SyncSession) -> SyncStats {
    let mut from_a: VecDeque<Message> = VecDeque::new();
    let mut from_b: VecDeque<Message> = VecDeque::new();
    let mut stats = SyncStats::default();

    from_a.extend(a.start());
    from_b.extend(b.start());

    loop {
        let mut progress = false;

        if let Some(msg) = from_a.pop_front() {
            stats.record(&msg);
            for out in b.handle(msg) {
                from_b.push_back(out);
            }
            progress = true;
        }

        if let Some(msg) = from_b.pop_front() {
            stats.record(&msg);
            for out in a.handle(msg) {
                from_a.push_back(out);
            }
            progress = true;
        }

        if !progress {
            break;
        }
    }

    assert!(
        a.is_done() && b.is_done(),
        "deadlock: sync terminó sin que ambos peers cerraran"
    );

    stats
}
