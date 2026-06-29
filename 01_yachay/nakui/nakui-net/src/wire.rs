//! El protocolo de cable entre un cliente remoto y el servidor que hospeda
//! el escritor autoritativo.
//!
//! Dos direcciones sobre un único stream libp2p multiplexado:
//!   - cliente → servidor: [`ClientMsg::Submit`] (una intención + `req_id`).
//!   - servidor → cliente: [`ServerMsg::CommitResult`] (la respuesta a un
//!     `req_id`) y [`ServerMsg::Broadcast`] (cada commit autoritativo, para
//!     que toda proyección se mantenga al día).
//!
//! El `req_id` permite que varias intenciones estén en vuelo a la vez: la
//! respuesta se rutea a quien la espera. El `Broadcast` es independiente —
//! aplicar un commit es idempotente por `seq`, así que recibir el propio
//! commit por las dos vías (respuesta + difusión) es inofensivo.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use nakui_sync::{Commit, Intent};

/// Mensaje del cliente hacia el servidor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMsg {
    /// Entrega una intención. `req_id` es único por conexión y monótono;
    /// la respuesta vuelve como [`ServerMsg::CommitResult`] con el mismo id.
    Submit { req_id: u64, intent: Intent },
}

/// Mensaje del servidor hacia el cliente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMsg {
    /// Catch-up inicial: el estado autoritativo completo al momento de
    /// conectar. El servidor lo envía una vez, apenas el cliente se engancha
    /// (atómico con el alta de su suscripción, así no hay hueco ni
    /// duplicado con los `Broadcast` que siguen). `last_seq` es el cursor
    /// del estado capturado: los broadcasts con `seq <= last_seq` ya están
    /// incluidos y se saltean por idempotencia.
    Snapshot {
        records: Vec<(String, Uuid, Value)>,
        last_seq: Option<u64>,
    },
    /// Respuesta a un `Submit`: el commit autoritativo, o el error de
    /// validación (la intención fue rechazada; el estado quedó intacto).
    CommitResult {
        req_id: u64,
        result: Result<Commit, String>,
    },
    /// Difusión de un commit autoritativo a todos los clientes conectados
    /// (incluido el que lo originó). El cliente lo aplica a su proyección.
    Broadcast { commit: Commit },
}
