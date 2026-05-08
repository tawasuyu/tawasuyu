use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldPath {
    pub entity: String,
    pub id: Uuid,
    pub field: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FieldOp {
    Set {
        path: FieldPath,
        value: Value,
    },
    Create {
        entity: String,
        id: Uuid,
        data: Value,
    },
    Delete {
        entity: String,
        id: Uuid,
    },
}

impl FieldOp {
    /// Token a manifest's `writes` list matches against.
    /// "Caja.saldo" for field updates, "Movimiento" for whole-record ops.
    pub fn capability_token(&self) -> String {
        match self {
            FieldOp::Set { path, .. } => format!("{}.{}", path.entity, path.field),
            FieldOp::Create { entity, .. } => entity.clone(),
            FieldOp::Delete { entity, .. } => entity.clone(),
        }
    }
}

/// Apply only the ops that target `(entity, id)` to `state` and return the
/// new value. Returns `None` if a Delete op removes the target — callers
/// should skip post-checks against a deleted entity rather than running
/// them against the stale prior state.
pub fn simulate_on(state: &Value, entity: &str, id: Uuid, ops: &[FieldOp]) -> Option<Value> {
    let mut s: Option<Value> = Some(state.clone());
    for op in ops {
        match op {
            FieldOp::Set { path, value } if path.entity == entity && path.id == id => {
                if let Some(Value::Object(map)) = s.as_mut() {
                    map.insert(path.field.clone(), value.clone());
                }
            }
            FieldOp::Create {
                entity: e,
                id: i,
                data,
            } if e == entity && *i == id => {
                s = Some(data.clone());
            }
            FieldOp::Delete {
                entity: e,
                id: i,
            } if e == entity && *i == id => {
                s = None;
            }
            _ => {}
        }
    }
    s
}
