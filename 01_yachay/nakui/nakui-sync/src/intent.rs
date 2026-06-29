//! El protocolo de escritura entre un cliente y el escritor autoritativo.
//!
//! Un cliente nunca muta estado directamente: emite una [`Intent`] que el
//! escritor valida, ordena (le asigna un `seq` monótono) y materializa. El
//! resultado es un [`Commit`] cuyas `entries` son exactamente las entradas
//! que se anexaron al log — el DELTA que se difunde a todos los clientes.
//!
//! La clave del diseño: como nakui es event-sourced, una entrada de log
//! ([`LogEntry`]) ya *es* el delta. Aplicar las `entries` de un commit a
//! cualquier proyección la pone al día, exactamente como hace `replay` por
//! entrada. No hace falta un formato de diff aparte.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use nakui_core::event_log::LogEntry;

/// Una intención de escritura emitida por un cliente hacia el escritor
/// autoritativo.
///
/// `Seed` no lleva `id`: lo asigna el escritor, así todos los clientes
/// convergen al mismo identificador (no pueden inventar ids que colisionen).
/// Las cuatro variantes espejan el contrato `MetaBackend` (seed/update/
/// delete/morphism) pero como *datos serializables* — listas para viajar
/// por un socket en el path en red.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum Intent {
    /// Alta de un record en `entity`. El escritor asigna el `id`.
    Seed {
        entity: String,
        data: Map<String, Value>,
    },
    /// Edición de campos de un record existente: `set` sobreescribe,
    /// `clear` elimina claves. Ambos vacíos = no-op.
    Update {
        entity: String,
        id: Uuid,
        #[serde(default)]
        set: Map<String, Value>,
        #[serde(default)]
        clear: Vec<String>,
    },
    /// Baja de un record.
    Delete { entity: String, id: Uuid },
    /// Ejecución de un morfismo declarado por un módulo. `inputs` es una
    /// lista ORDENADA `(rol, id)` que admite el mismo rol repetido (inputs
    /// variádicos, p.ej. un asiento de N patas).
    Morphism {
        module_id: String,
        name: String,
        #[serde(default)]
        inputs: Vec<(String, Uuid)>,
        #[serde(default)]
        params: Value,
    },
}

/// Resultado de un commit autoritativo.
///
/// `entries` es el delta a difundir: aplicarlas a una proyección la pone
/// al día. `primary_id`/`changed`/`post_status` son metadatos para la UI
/// del cliente que emitió la intención (componen el toast).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Commit {
    /// Entradas anexadas al log por este commit (el delta). Vacío en un
    /// no-op (update sin cambios) o en modo in-memory sin log.
    pub entries: Vec<LogEntry>,
    /// Id del record primario afectado (seed/update/delete). `None` para
    /// morphism (puede tocar varios records).
    pub primary_id: Option<Uuid>,
    /// Cantidad de cambios efectivos. `0` = no-op.
    pub changed: usize,
    /// Status emitido por hooks internos del escritor (ej. auto-compact),
    /// para concatenar al toast.
    pub post_status: Option<String>,
}

impl Commit {
    /// Commit de un no-op (edit que no cambió nada): sin entradas, sin
    /// status, `changed = 0`.
    pub fn no_op(id: Uuid) -> Self {
        Self {
            entries: Vec::new(),
            primary_id: Some(id),
            changed: 0,
            post_status: None,
        }
    }

    /// El `seq` más alto anexado por este commit, si anexó algo. Un
    /// cliente lo usa para saber hasta dónde quedó al día.
    pub fn last_seq(&self) -> Option<u64> {
        self.entries.last().map(|e| e.seq())
    }
}
