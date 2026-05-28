//! `NoteStore` — el almacén de notas y su grafo de enlaces.
//!
//! Guarda las notas en un `BTreeMap` para que toda iteración sea
//! determinista (ordenada por id). Los enlaces se resuelven por título
//! sin distinguir mayúsculas: `[[cocina]]` y `[[Cocina]]` apuntan a la
//! misma nota.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::note::{Note, NoteId};

/// El almacén de notas de khipu_app.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoteStore {
    notes: BTreeMap<NoteId, Note>,
    /// Siguiente id a asignar — monótono, nunca reutiliza huecos.
    next_id: NoteId,
}

impl NoteStore {
    pub fn new() -> Self {
        Self { notes: BTreeMap::new(), next_id: 1 }
    }

    /// Crea una nota y devuelve su id. Empieza con `mass = 1.0` y
    /// `last_access = now` — recién nacida, plenamente visible.
    pub fn create(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
        tags: Vec<String>,
        now: u64,
    ) -> NoteId {
        let id = self.next_id;
        self.next_id += 1;
        self.notes.insert(
            id,
            Note {
                id,
                title: title.into(),
                body: body.into(),
                tags,
                created_at: now,
                updated_at: now,
                last_access: now,
                mass: 1.0,
            },
        );
        id
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    pub fn get(&self, id: NoteId) -> Option<&Note> {
        self.notes.get(&id)
    }

    pub fn get_mut(&mut self, id: NoteId) -> Option<&mut Note> {
        self.notes.get_mut(&id)
    }

    /// Itera las notas en orden de id (determinista).
    pub fn iter(&self) -> impl Iterator<Item = &Note> {
        self.notes.values()
    }

    /// Reemplaza el cuerpo de una nota y actualiza su marca de tiempo.
    /// También marca `last_access` — editar cuenta como acceso.
    /// `false` si la nota no existe.
    pub fn update_body(&mut self, id: NoteId, body: impl Into<String>, now: u64) -> bool {
        match self.notes.get_mut(&id) {
            Some(n) => {
                n.body = body.into();
                n.updated_at = now;
                n.last_access = now;
                true
            }
            None => false,
        }
    }

    /// Marca `last_access = now`. La señal que `khipu-gravity` usa para
    /// reforzar la masa. No-op si la nota no existe; `true` si tocó.
    pub fn touch(&mut self, id: NoteId, now: u64) -> bool {
        match self.notes.get_mut(&id) {
            Some(n) => {
                n.last_access = now;
                true
            }
            None => false,
        }
    }

    /// Asigna directamente la masa de una nota. La física vive en
    /// `khipu-gravity`; el store sólo persiste el resultado.
    pub fn set_mass(&mut self, id: NoteId, mass: f32) -> bool {
        match self.notes.get_mut(&id) {
            Some(n) => {
                n.mass = mass;
                true
            }
            None => false,
        }
    }

    /// Elimina una nota y la devuelve.
    pub fn remove(&mut self, id: NoteId) -> Option<Note> {
        self.notes.remove(&id)
    }

    /// Notas que llevan la etiqueta `tag`, en orden de id.
    pub fn by_tag(&self, tag: &str) -> Vec<&Note> {
        self.notes.values().filter(|n| n.has_tag(tag)).collect()
    }

    /// Notas cuyo título o cuerpo contienen `query`, en orden de id.
    pub fn search(&self, query: &str) -> Vec<&Note> {
        self.notes.values().filter(|n| n.matches(query)).collect()
    }

    /// Ids de las notas cuyo título es `title` (sin distinguir
    /// mayúsculas). Pueden ser varias: los títulos no son únicos.
    pub fn resolve_title(&self, title: &str) -> Vec<NoteId> {
        self.notes
            .values()
            .filter(|n| n.title.eq_ignore_ascii_case(title))
            .map(|n| n.id)
            .collect()
    }

    /// Ids de las notas a las que `id` enlaza por `[[...]]`, resueltas y
    /// deduplicadas. Un enlace a un título inexistente simplemente no
    /// aporta ningún id (enlace colgante).
    pub fn forward_links(&self, id: NoteId) -> Vec<NoteId> {
        let Some(note) = self.notes.get(&id) else {
            return Vec::new();
        };
        let mut out: Vec<NoteId> = Vec::new();
        for title in note.outgoing_links() {
            for target in self.resolve_title(&title) {
                if !out.contains(&target) {
                    out.push(target);
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// Ids de las notas que enlazan hacia `id` (backlinks), en orden de id.
    pub fn backlinks(&self, id: NoteId) -> Vec<NoteId> {
        let Some(target) = self.notes.get(&id) else {
            return Vec::new();
        };
        let title = &target.title;
        self.notes
            .values()
            .filter(|n| {
                n.id != id
                    && n.outgoing_links()
                        .iter()
                        .any(|l| l.eq_ignore_ascii_case(title))
            })
            .map(|n| n.id)
            .collect()
    }

    /// Notas sin ningún backlink — las islas del grafo.
    pub fn orphans(&self) -> Vec<&Note> {
        self.notes
            .values()
            .filter(|n| self.backlinks(n.id).is_empty())
            .collect()
    }

    /// Destinos `[[...]]` que ninguna nota satisface — enlaces colgantes.
    pub fn dangling_links(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for note in self.notes.values() {
            for title in note.outgoing_links() {
                if self.resolve_title(&title).is_empty()
                    && !out.iter().any(|t| t.eq_ignore_ascii_case(&title))
                {
                    out.push(title);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Almacén con tres notas enlazadas: Índice → Cocina, Índice → Jardín.
    fn seeded() -> (NoteStore, NoteId, NoteId, NoteId) {
        let mut s = NoteStore::new();
        let indice = s.create("Índice", "ver [[Cocina]] y [[Jardín]]", vec!["meta".into()], 100);
        let cocina = s.create("Cocina", "recetas; vuelve al [[Índice]]", vec!["casa".into()], 100);
        let jardin = s.create("Jardín", "plantas y riego", vec!["casa".into()], 100);
        (s, indice, cocina, jardin)
    }

    #[test]
    fn create_assigns_monotonic_ids() {
        let (s, indice, cocina, jardin) = seeded();
        assert_eq!((indice, cocina, jardin), (1, 2, 3));
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn forward_links_resolve_by_title() {
        let (s, indice, cocina, jardin) = seeded();
        assert_eq!(s.forward_links(indice), vec![cocina, jardin]);
    }

    #[test]
    fn backlinks_find_incoming_references() {
        let (s, indice, cocina, _) = seeded();
        // Cocina enlaza al Índice → el Índice tiene a Cocina como backlink.
        assert_eq!(s.backlinks(indice), vec![cocina]);
    }

    #[test]
    fn link_resolution_is_case_insensitive() {
        let mut s = NoteStore::new();
        let a = s.create("Taller", "trabajo", vec![], 0);
        let b = s.create("Notas", "voy al [[taller]]", vec![], 0);
        assert_eq!(s.forward_links(b), vec![a]);
        assert_eq!(s.backlinks(a), vec![b]);
    }

    #[test]
    fn by_tag_filters() {
        let (s, _, cocina, jardin) = seeded();
        let casa: Vec<_> = s.by_tag("casa").iter().map(|n| n.id).collect();
        assert_eq!(casa, vec![cocina, jardin]);
    }

    #[test]
    fn search_scans_title_and_body() {
        let (s, _, cocina, _) = seeded();
        let hits: Vec<_> = s.search("recetas").iter().map(|n| n.id).collect();
        assert_eq!(hits, vec![cocina]);
    }

    #[test]
    fn update_body_changes_links_and_timestamp() {
        let (mut s, indice, _, jardin) = seeded();
        assert!(s.update_body(indice, "ahora sólo [[Jardín]]", 200));
        assert_eq!(s.forward_links(indice), vec![jardin]);
        assert_eq!(s.get(indice).unwrap().updated_at, 200);
    }

    #[test]
    fn orphans_have_no_backlinks() {
        let (s, _, _, jardin) = seeded();
        // Jardín no recibe enlaces de vuelta... pero el Índice sí lo enlaza.
        // El único huérfano real sería una nota aislada.
        let mut s2 = s;
        let aislada = s2.create("Aislada", "sin conexiones", vec![], 0);
        let orphan_ids: Vec<_> = s2.orphans().iter().map(|n| n.id).collect();
        assert!(orphan_ids.contains(&aislada));
        assert!(!orphan_ids.contains(&jardin));
    }

    #[test]
    fn dangling_links_report_missing_targets() {
        let mut s = NoteStore::new();
        s.create("Nota", "apunta a [[Inexistente]]", vec![], 0);
        assert_eq!(s.dangling_links(), vec!["Inexistente"]);
    }

    #[test]
    fn create_initializes_mass_and_last_access() {
        let mut s = NoteStore::new();
        let id = s.create("x", "y", vec![], 1_700_000_000);
        let n = s.get(id).unwrap();
        assert_eq!(n.mass, 1.0);
        assert_eq!(n.last_access, 1_700_000_000);
    }

    #[test]
    fn touch_refreshes_last_access() {
        let mut s = NoteStore::new();
        let id = s.create("x", "y", vec![], 100);
        assert!(s.touch(id, 500));
        assert_eq!(s.get(id).unwrap().last_access, 500);
        assert!(!s.touch(9_999, 500));
    }

    #[test]
    fn update_body_also_marks_last_access() {
        let mut s = NoteStore::new();
        let id = s.create("x", "y", vec![], 100);
        assert!(s.update_body(id, "z", 700));
        assert_eq!(s.get(id).unwrap().last_access, 700);
    }

    #[test]
    fn set_mass_persists_the_value() {
        let mut s = NoteStore::new();
        let id = s.create("x", "y", vec![], 100);
        assert!(s.set_mass(id, 0.42));
        assert!((s.get(id).unwrap().mass - 0.42).abs() < 1e-6);
    }

    #[test]
    fn remove_drops_the_note() {
        let (mut s, indice, ..) = seeded();
        assert!(s.remove(indice).is_some());
        assert_eq!(s.len(), 2);
        assert!(s.get(indice).is_none());
    }
}
