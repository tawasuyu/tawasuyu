//! Claves namespaced del DHT compartido — la **primitiva unificadora** de
//! Brahman (Capa 1).
//!
//! El ecosistema brahman corre UN solo Kademlia (en este crate, `card-net`).
//! Para que distintos dominios — código indexado (minga), Cards
//! (card-discovery), Personas (ágora), servicios — coexistan sin colisión,
//! cada clave lleva un byte de `kind` como prefijo. La representación en wire
//! es de longitud fija: `[kind_tag] ++ blake3(id)` = 33 bytes.
//!
//! Vive en `card-net` (y no en un dominio concreto) precisamente porque es el
//! namespace COMÚN: minga, agora y card-discovery la comparten. `minga-dht`
//! la re-exporta por compatibilidad histórica.

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

/// Clave de DHT namespaced. Dos formas de construcción:
///
/// - Con un `id` legible (`new`, `code`, `card`, `persona`) — el wire
///   hashea el id con blake3. Útil cuando el consumidor publica
///   identidades simbólicas (nombres de módulos, slugs de personas).
/// - Con `for_hash` — el wire usa los 32 bytes del hash directamente.
///   Útil cuando el id YA es un blake3 (como en minga, que indexa
///   contenido por su α-hash, o ágora, cuyo `IdentityId` ya es
///   `blake3(pubkey)`) — evita una segunda pasada de blake3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DhtKey {
    kind: RecordKind,
    /// Representación canónica de los 32 bytes que forman la "id" en wire.
    /// Si se construyó por nombre simbólico, son `blake3(id_string)`.
    /// Si se construyó por hash directo, son el hash tal cual.
    body: [u8; 32],
    /// `id` legible — para `Display`. `None` si se construyó por hash.
    label: Option<String>,
}

impl DhtKey {
    pub fn new(kind: RecordKind, id: impl Into<String>) -> Self {
        let id = id.into();
        let body = *blake3::hash(id.as_bytes()).as_bytes();
        Self {
            kind,
            body,
            label: Some(id),
        }
    }

    /// Clave para un bloque de código (id legible).
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

    /// Clave a partir de un hash ya computado (32 bytes). El wire usa
    /// esos bytes directamente, **sin re-hashear**.
    pub fn for_hash(kind: RecordKind, hash: [u8; 32]) -> Self {
        Self {
            kind,
            body: hash,
            label: None,
        }
    }

    pub fn kind(&self) -> RecordKind {
        self.kind
    }

    pub fn id(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Representación en wire: `[kind_tag] ++ body`, 33 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(DHT_KEY_LEN);
        out.push(self.kind.tag());
        out.extend_from_slice(&self.body);
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
    fn for_hash_no_rehashea() {
        // for_hash debe usar los bytes tal cual: body = hash, no blake3(hash).
        let h = [7u8; 32];
        let k = DhtKey::for_hash(RecordKind::Persona, h);
        let wire = k.to_bytes();
        assert_eq!(wire[0], RecordKind::Persona.tag());
        assert_eq!(&wire[1..], &h);
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
