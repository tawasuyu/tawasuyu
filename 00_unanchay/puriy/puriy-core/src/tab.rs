//! Tab — pestaña con URL y título.

use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Identificador estable de una pestaña dentro de su [`Session`](crate::Session).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TabId(pub Ulid);

impl TabId {
    pub fn nuevo() -> Self {
        Self(Ulid::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tab {
    pub id: TabId,
    pub url: String,
    pub title: String,
    /// Segundos UNIX en que se abrió.
    pub created_at: u64,
}

impl Tab {
    pub fn nueva(url: impl Into<String>, created_at: u64) -> Self {
        Self {
            id: TabId::nuevo(),
            url: url.into(),
            title: String::new(),
            created_at,
        }
    }
}
