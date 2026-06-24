//! Índice semántico **persistido** — embeddings en disco (formato nativo
//! postcard), al estilo de `paloma-semantic` pero genérico sobre claves
//! `String` (líneas de comando del historial, rutas de archivo del explorer…).
//!
//! El objetivo es no re-embeber el corpus entero en cada `:buscar`: el índice
//! guarda `clave → vector`, sólo embebe las claves nuevas (`ensure`), poda las
//! que desaparecieron (`retain`) y rankea por coseno (`search`). Se guarda
//! etiquetado con el `ModelId` del provider; si cambia el backend/dimensión de
//! embeddings, el índice se descarta y se reconstruye solo.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rimay_verbo::{EmbeddingVector, ModelId, Provider};
use serde::{Deserialize, Serialize};

/// Forma serializada en disco: el modelo con que se embebió + los vectores.
#[derive(Serialize, Deserialize, Default)]
struct Stored {
    model_name: String,
    model_dim: usize,
    vectors: HashMap<String, Vec<f32>>,
}

/// Un índice semántico vivo, respaldado por un archivo postcard.
pub(crate) struct SemanticIndex {
    path: PathBuf,
    model: ModelId,
    vectors: HashMap<String, Vec<f32>>,
    dirty: bool,
}

impl SemanticIndex {
    /// La ruta canónica de un índice por `scope` (p.ej. `"history"`, `"files"`):
    /// `$XDG_DATA_HOME/shuma/semantic/<scope>.idx` (o `~/.local/share/...`).
    pub fn path_for(scope: &str) -> PathBuf {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("shuma").join("semantic").join(format!("{scope}.idx"))
    }

    /// Carga el índice de `path` para `model`. Si el archivo no existe, está
    /// corrupto, o fue hecho con otro modelo (cambió backend/dimensión de
    /// embeddings), arranca vacío — así se reconstruye sin intervención.
    pub fn load(path: PathBuf, model: ModelId) -> Self {
        let vectors = std::fs::read(&path)
            .ok()
            .and_then(|b| postcard::from_bytes::<Stored>(&b).ok())
            .filter(|s| s.model_name == model.name && s.model_dim == model.dimension)
            .map(|s| s.vectors)
            .unwrap_or_default();
        Self { path, model, vectors, dirty: false }
    }

    /// Embebe con `provider` las entradas de `corpus` cuya **clave** todavía no
    /// esté en el índice y las ingiere. Cada entrada es `(clave, texto)`: la
    /// clave identifica/indexa, el texto es lo que se embebe (puede traer más
    /// contexto). Sólo toca lo que falta — el resto ya está en disco.
    pub async fn ensure(&mut self, provider: &dyn Provider, corpus: &[(String, String)]) -> Result<(), String> {
        let (keys, texts): (Vec<String>, Vec<String>) = corpus
            .iter()
            .filter(|(k, _)| !self.vectors.contains_key(k))
            .cloned()
            .unzip();
        if texts.is_empty() {
            return Ok(());
        }
        let vecs = provider
            .embed_batch(&texts)
            .await
            .map_err(|e| format!("embeddings: {e}"))?;
        for (k, v) in keys.into_iter().zip(vecs) {
            self.vectors.insert(k, v.values);
        }
        self.dirty = true;
        Ok(())
    }

    /// Descarta del índice las claves que no estén en `keep` (poda lo que ya no
    /// existe en el corpus, p.ej. comandos caídos fuera de la ventana).
    pub fn retain(&mut self, keep: &[String]) {
        let set: HashSet<&str> = keep.iter().map(String::as_str).collect();
        let before = self.vectors.len();
        self.vectors.retain(|k, _| set.contains(k.as_str()));
        if self.vectors.len() != before {
            self.dirty = true;
        }
    }

    /// Rankea `query` (ya embebido) contra el corpus indexado: top-k con score
    /// ≥ `min_score`, ordenado de mayor a menor parecido (coseno).
    pub fn search(&self, query: &EmbeddingVector, top_k: usize, min_score: f32) -> Vec<(String, f32)> {
        let mut scored: Vec<(String, f32)> = self
            .vectors
            .iter()
            .filter_map(|(k, v)| cosine(&query.values, v).map(|s| (k.clone(), s)))
            .filter(|(_, s)| *s >= min_score)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }

    /// Persiste a disco si cambió (escritura atómica vía temporal + rename).
    pub fn save(&self) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let stored = Stored {
            model_name: self.model.name.clone(),
            model_dim: self.model.dimension,
            vectors: self.vectors.clone(),
        };
        let bytes = postcard::to_allocvec(&stored)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("idx.tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &self.path)
    }
}

/// Similitud coseno entre dos vectores crudos. `None` si difieren en largo, hay
/// alguno vacío, o alguno es el vector nulo (norma 0).
fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return None;
    }
    Some(dot / (na.sqrt() * nb.sqrt()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(model: &ModelId, vals: Vec<f32>) -> EmbeddingVector {
        EmbeddingVector::new(model.clone(), vals).unwrap()
    }

    #[test]
    fn search_rankea_por_coseno_y_respeta_min_score() {
        let model = ModelId::new("test", 3);
        let mut idx = SemanticIndex {
            path: PathBuf::from("/dev/null"),
            model: model.clone(),
            vectors: HashMap::new(),
            dirty: false,
        };
        idx.vectors.insert("igual".into(), vec![1.0, 0.0, 0.0]);
        idx.vectors.insert("ortogonal".into(), vec![0.0, 1.0, 0.0]);
        idx.vectors.insert("opuesto".into(), vec![-1.0, 0.0, 0.0]);
        let q = ev(&model, vec![1.0, 0.0, 0.0]);
        let hits = idx.search(&q, 10, 0.5);
        // Sólo "igual" supera 0.5; "ortogonal" (0.0) y "opuesto" (-1.0) caen.
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "igual");
        assert!((hits[0].1 - 1.0).abs() < 1e-5);
    }

    #[test]
    fn retain_poda_lo_que_no_esta_en_keep() {
        let model = ModelId::new("test", 2);
        let mut idx = SemanticIndex {
            path: PathBuf::from("/dev/null"),
            model,
            vectors: HashMap::new(),
            dirty: false,
        };
        idx.vectors.insert("a".into(), vec![1.0, 0.0]);
        idx.vectors.insert("b".into(), vec![0.0, 1.0]);
        idx.retain(&["a".to_string()]);
        assert!(idx.vectors.contains_key("a"));
        assert!(!idx.vectors.contains_key("b"));
        assert!(idx.dirty);
    }
}
