//! Claves namespaced del DHT compartido.
//!
//! El ecosistema brahman corre UN solo Kademlia (en `brahman-net`). Para
//! que distintos dominios — código indexado (minga), Cards, Personas
//! (ágora) — coexistan sin colisión, cada clave lleva un byte de `kind`
//! como prefijo. La representación en wire es de longitud fija:
//! `[kind_tag] ++ blake3(id)` = 33 bytes.

use serde::{Deserialize, Serialize};

/// Tipo de registro — el namespace de una clave en el DHT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordKind {
    /// Bloque de código indexado (minga).
    Code,
    /// `Card` brahman (módulo, ente, etc.).
    Card,
    /// `Persona` de ágora (identidad humana federada).
    Persona,
    /// Endpoint de servicio.
    Service,
    /// Dominio definido por el consumidor.
    Custom(u8),
}

impl RecordKind {
    /// Byte de etiqueta. `Custom(n)` ocupa `0x80 | n` (top bit) para no
    /// chocar nunca con los kinds estándar (`0x00..`).
    pub fn tag(&self) -> u8 {
        match self {
            RecordKind::Code => 0x01,
            RecordKind::Card => 0x02,
            RecordKind::Persona => 0x03,
            RecordKind::Service => 0x04,
            RecordKind::Custom(n) => 0x80 | (n & 0x7f),
        }
    }
}

/// Longitud fija de la clave en wire: 1 byte de kind + 32 de hash.
pub const DHT_KEY_LEN: usize = 33;

/// Clave de DHT namespaced. Se construye con un `id` legible; la
/// representación en wire hashea el `id` con blake3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DhtKey {
    kind: RecordKind,
    id: String,
}

impl DhtKey {
    pub fn new(kind: RecordKind, id: impl Into<String>) -> Self {
        Self { kind, id: id.into() }
    }

    /// Clave para un bloque de código.
    pub fn code(id: impl Into<String>) -> Self {
        Self::new(RecordKind::Code, id)
    }

    /// Clave para una Card.
    pub fn card(id: impl Into<String>) -> Self {
        Self::new(RecordKind::Card, id)
    }

    /// Clave para una Persona.
    pub fn persona(id: impl Into<String>) -> Self {
        Self::new(RecordKind::Persona, id)
    }

    pub fn kind(&self) -> RecordKind {
        self.kind
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Representación en wire: `[kind_tag] ++ blake3(id)`, 33 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(DHT_KEY_LEN);
        out.push(self.kind.tag());
        out.extend_from_slice(blake3::hash(self.id.as_bytes()).as_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_key_has_fixed_length() {
        assert_eq!(DhtKey::card("modulo-x").to_bytes().len(), DHT_KEY_LEN);
        assert_eq!(DhtKey::code("fn-hash").to_bytes().len(), DHT_KEY_LEN);
    }

    #[test]
    fn same_id_different_kind_does_not_collide() {
        let a = DhtKey::card("foo").to_bytes();
        let b = DhtKey::code("foo").to_bytes();
        let c = DhtKey::persona("foo").to_bytes();
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
        // El hash del id es el mismo; sólo difiere el byte de kind.
        assert_eq!(a[1..], b[1..]);
        assert_ne!(a[0], b[0]);
    }

    #[test]
    fn same_kind_and_id_is_stable() {
        assert_eq!(DhtKey::card("x").to_bytes(), DhtKey::card("x").to_bytes());
    }

    #[test]
    fn custom_kind_never_collides_with_standard() {
        for std in [RecordKind::Code, RecordKind::Card, RecordKind::Persona, RecordKind::Service] {
            for n in 0..=127u8 {
                assert_ne!(std.tag(), RecordKind::Custom(n).tag());
            }
        }
    }
}
