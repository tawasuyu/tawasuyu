//! History — registro cronológico de URLs visitadas.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    /// Segundos UNIX en que se visitó la URL.
    pub visited_at: u64,
}

/// Lista append-only de entradas. El caller decide cuándo recortar; el
/// modelo no impone política de retención. Más antiguo primero.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct History {
    entries: Vec<HistoryEntry>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn record(&mut self, url: impl Into<String>, title: impl Into<String>, visited_at: u64) {
        self.entries.push(HistoryEntry {
            url: url.into(),
            title: title.into(),
            visited_at,
        });
    }

    /// Las `n` más recientes (cronológicamente descendente).
    pub fn recent(&self, n: usize) -> Vec<&HistoryEntry> {
        self.entries.iter().rev().take(n).collect()
    }

    /// Búsqueda case-insensitive en url+title. Devuelve coincidencias
    /// en orden cronológico descendente.
    pub fn search(&self, query: &str) -> Vec<&HistoryEntry> {
        let q = query.to_lowercase();
        self.entries
            .iter()
            .rev()
            .filter(|e| e.url.to_lowercase().contains(&q) || e.title.to_lowercase().contains(&q))
            .collect()
    }

    /// Recorta el historial a las `max` entradas más recientes.
    pub fn truncate_to(&mut self, max: usize) {
        if self.entries.len() > max {
            let drop = self.entries.len() - max;
            self.entries.drain(..drop);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist_ejemplo() -> History {
        let mut h = History::new();
        h.record("https://tawasuyu.net", "tawasuyu landing", 100);
        h.record("https://docs.rs/serde", "serde docs", 200);
        h.record("https://example.com", "Example Domain", 300);
        h
    }

    #[test]
    fn record_apila_en_orden() {
        let h = hist_ejemplo();
        assert_eq!(h.len(), 3);
        assert_eq!(h.entries()[0].url, "https://tawasuyu.net");
        assert_eq!(h.entries()[2].url, "https://example.com");
    }

    #[test]
    fn recent_devuelve_descendente() {
        let h = hist_ejemplo();
        let r = h.recent(2);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].url, "https://example.com");
        assert_eq!(r[1].url, "https://docs.rs/serde");
    }

    #[test]
    fn search_es_case_insensitive_en_url_y_title() {
        let h = hist_ejemplo();
        let r = h.search("EXAMPLE");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].url, "https://example.com");

        let r = h.search("serde");
        assert_eq!(r.len(), 1);

        let r = h.search("docs");
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn truncate_to_conserva_las_mas_recientes() {
        let mut h = hist_ejemplo();
        h.truncate_to(2);
        assert_eq!(h.len(), 2);
        assert_eq!(h.entries()[0].url, "https://docs.rs/serde");
        assert_eq!(h.entries()[1].url, "https://example.com");
    }
}
