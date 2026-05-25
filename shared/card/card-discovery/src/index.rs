//! Índice en memoria de Cards, con filtros de búsqueda.

use card_core::{Capability, Card, CardKind};
use ulid::Ulid;

/// Colección consultable de Cards.
#[derive(Debug, Clone, Default)]
pub struct CardIndex {
    cards: Vec<Card>,
}

impl CardIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, card: Card) {
        self.cards.push(card);
    }

    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    pub fn all(&self) -> &[Card] {
        &self.cards
    }

    /// Card por id exacto.
    pub fn by_id(&self, id: Ulid) -> Option<&Card> {
        self.cards.iter().find(|c| c.id == id)
    }

    /// Cards cuyo label contiene `needle` (case-insensitive).
    pub fn by_label(&self, needle: &str) -> Vec<&Card> {
        let n = needle.to_lowercase();
        self.cards
            .iter()
            .filter(|c| c.label.to_lowercase().contains(&n))
            .collect()
    }

    /// Cards de un `CardKind` dado.
    pub fn by_kind(&self, kind: CardKind) -> Vec<&Card> {
        self.cards.iter().filter(|c| c.kind == kind).collect()
    }

    /// Cards que proveen la `Capability` dada.
    pub fn providing(&self, cap: &Capability) -> Vec<&Card> {
        self.cards
            .iter()
            .filter(|c| c.provides.contains(cap))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(label: &str) -> Card {
        Card::new(label)
    }

    #[test]
    fn by_label_is_case_insensitive_substring() {
        let mut ix = CardIndex::new();
        ix.insert(card("Broker Demo"));
        ix.insert(card("file explorer"));
        assert_eq!(ix.by_label("broker").len(), 1);
        assert_eq!(ix.by_label("EXPLOR").len(), 1);
        assert_eq!(ix.by_label("zzz").len(), 0);
    }

    #[test]
    fn by_id_finds_exact() {
        let mut ix = CardIndex::new();
        let c = card("x");
        let id = c.id;
        ix.insert(c);
        assert!(ix.by_id(id).is_some());
        assert!(ix.by_id(Ulid::new()).is_none());
    }

    #[test]
    fn providing_filters_by_capability() {
        let mut spawner = card("spawner");
        spawner.provides.insert(Capability::Spawn);
        let mut logger = card("logger");
        logger.provides.insert(Capability::Journal);

        let mut ix = CardIndex::new();
        ix.insert(spawner);
        ix.insert(logger);
        assert_eq!(ix.providing(&Capability::Spawn).len(), 1);
        assert_eq!(ix.providing(&Capability::Journal).len(), 1);
        assert_eq!(ix.providing(&Capability::FilesystemRoot).len(), 0);
    }

    #[test]
    fn by_kind_splits_ente_and_data() {
        let mut ente = card("ente");
        ente.kind = CardKind::Ente;
        let mut data = card("data");
        data.kind = CardKind::Data;
        let mut ix = CardIndex::new();
        ix.insert(ente);
        ix.insert(data);
        assert_eq!(ix.by_kind(CardKind::Ente).len(), 1);
        assert_eq!(ix.by_kind(CardKind::Data).len(), 1);
    }
}
