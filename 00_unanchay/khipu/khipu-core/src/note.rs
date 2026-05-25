//! El modelo `Note` — la unidad de badu.

use serde::{Deserialize, Serialize};

use crate::links::parse_links;

/// Identificador de una nota. Lo asigna el almacén, monótono y estable.
pub type NoteId = u64;

/// Una nota: título, cuerpo, etiquetas y marcas de tiempo. Los enlaces
/// no se guardan aparte — se derivan del cuerpo bajo demanda.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: NoteId,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    /// Segundo Unix de creación.
    pub created_at: u64,
    /// Segundo Unix de la última edición.
    pub updated_at: u64,
}

impl Note {
    /// Destinos `[[...]]` que el cuerpo de la nota referencia.
    pub fn outgoing_links(&self) -> Vec<String> {
        parse_links(&self.body)
    }

    /// `true` si la nota lleva la etiqueta `tag` (sin distinguir mayúsculas).
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t.eq_ignore_ascii_case(tag))
    }

    /// `true` si `query` aparece en el título o el cuerpo (sin distinguir
    /// mayúsculas).
    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        self.title.to_lowercase().contains(&q) || self.body.to_lowercase().contains(&q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(title: &str, body: &str) -> Note {
        Note {
            id: 1,
            title: title.into(),
            body: body.into(),
            tags: vec!["casa".into()],
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn outgoing_links_reads_the_body() {
        let n = note("Cocina", "preparar con [[Horno]] y [[Cuchillos]]");
        assert_eq!(n.outgoing_links(), vec!["Horno", "Cuchillos"]);
    }

    #[test]
    fn has_tag_is_case_insensitive() {
        let n = note("x", "y");
        assert!(n.has_tag("CASA"));
        assert!(!n.has_tag("trabajo"));
    }

    #[test]
    fn matches_searches_title_and_body() {
        let n = note("Lista de mercado", "comprar pan");
        assert!(n.matches("MERCADO"));
        assert!(n.matches("pan"));
        assert!(!n.matches("ausente"));
    }
}
