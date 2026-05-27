//! `CardDiscovery` — une el índice local de Cards con el DHT.

use crate::index::CardIndex;
use minga_dht::{Dht, DhtKey};
use libp2p::PeerId;

/// Búsqueda de Cards: siempre local, opcionalmente sobre la malla P2P.
pub struct CardDiscovery {
    /// Índice local consultable.
    pub index: CardIndex,
    dht: Option<Dht>,
}

impl CardDiscovery {
    /// Discovery sólo-local (sin malla P2P).
    pub fn local(index: CardIndex) -> Self {
        Self { index, dht: None }
    }

    /// Discovery local + DHT.
    pub fn with_dht(index: CardIndex, dht: Dht) -> Self {
        Self { index, dht: Some(dht) }
    }

    /// `true` si hay malla P2P conectada.
    pub fn has_dht(&self) -> bool {
        self.dht.is_some()
    }

    /// Anuncia al DHT cada Card local (clave = `DhtKey::card(id)`).
    /// No-op si no hay DHT.
    pub fn announce_all(&self) {
        if let Some(dht) = &self.dht {
            for card in self.index.all() {
                dht.announce(&DhtKey::card(card.id.to_string()));
            }
        }
    }

    /// Busca proveedores remotos de una Card por id. Vacío si no hay DHT.
    pub async fn find_remote(&self, card_id: &str) -> Vec<PeerId> {
        match &self.dht {
            Some(dht) => dht.find(&DhtKey::card(card_id)).await,
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use card_core::Card;

    #[tokio::test]
    async fn local_only_discovery_has_no_dht() {
        let mut ix = CardIndex::new();
        ix.insert(Card::new("local-1"));
        let disc = CardDiscovery::local(ix);
        assert!(!disc.has_dht());
        disc.announce_all(); // no-op, no debe panickear
        assert!(disc.find_remote("cualquiera").await.is_empty());
        assert_eq!(disc.index.len(), 1);
    }
}
