//! Bookmark — URL guardada, opcionalmente en una carpeta.

use serde::{Deserialize, Serialize};
use ulid::Ulid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BookmarkId(pub Ulid);

impl BookmarkId {
    pub fn nuevo() -> Self {
        Self(Ulid::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: BookmarkId,
    pub url: String,
    pub title: String,
    /// `None` = raíz (sin carpeta). Carpetas son strings simples — no
    /// hay sub-jerarquía formal; basta para Fase 1.
    pub folder: Option<String>,
    pub created_at: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BookmarkStore {
    items: Vec<Bookmark>,
}

impl BookmarkStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn items(&self) -> &[Bookmark] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn add(
        &mut self,
        url: impl Into<String>,
        title: impl Into<String>,
        folder: Option<String>,
        created_at: u64,
    ) -> BookmarkId {
        let bm = Bookmark {
            id: BookmarkId::nuevo(),
            url: url.into(),
            title: title.into(),
            folder,
            created_at,
        };
        let id = bm.id;
        self.items.push(bm);
        id
    }

    /// Devuelve `true` si se eliminó, `false` si el id no existía.
    pub fn remove(&mut self, id: BookmarkId) -> bool {
        let len_antes = self.items.len();
        self.items.retain(|b| b.id != id);
        self.items.len() != len_antes
    }

    pub fn get(&self, id: BookmarkId) -> Option<&Bookmark> {
        self.items.iter().find(|b| b.id == id)
    }

    /// Carpetas distintas presentes, ordenadas alfabéticamente.
    /// La raíz (`None`) no aparece en el listado.
    pub fn folders(&self) -> Vec<String> {
        let mut f: Vec<String> = self
            .items
            .iter()
            .filter_map(|b| b.folder.clone())
            .collect();
        f.sort();
        f.dedup();
        f
    }

    /// Items en una carpeta dada. `None` lista los de la raíz.
    pub fn in_folder(&self, folder: Option<&str>) -> Vec<&Bookmark> {
        self.items
            .iter()
            .filter(|b| b.folder.as_deref() == folder)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_ejemplo() -> BookmarkStore {
        let mut s = BookmarkStore::new();
        s.add("https://a.test", "A", None, 1);
        s.add("https://b.test", "B", Some("trabajo".into()), 2);
        s.add("https://c.test", "C", Some("trabajo".into()), 3);
        s.add("https://d.test", "D", Some("ocio".into()), 4);
        s
    }

    #[test]
    fn add_y_remove_ajustan_len() {
        let mut s = store_ejemplo();
        assert_eq!(s.len(), 4);
        let id = s.items()[0].id;
        assert!(s.remove(id));
        assert_eq!(s.len(), 3);
        assert!(!s.remove(id));
    }

    #[test]
    fn folders_lista_unicas_ordenadas() {
        let s = store_ejemplo();
        assert_eq!(s.folders(), vec!["ocio".to_string(), "trabajo".into()]);
    }

    #[test]
    fn in_folder_filtra_correctamente() {
        let s = store_ejemplo();
        assert_eq!(s.in_folder(None).len(), 1);
        assert_eq!(s.in_folder(Some("trabajo")).len(), 2);
        assert_eq!(s.in_folder(Some("ocio")).len(), 1);
        assert_eq!(s.in_folder(Some("inexistente")).len(), 0);
    }
}
