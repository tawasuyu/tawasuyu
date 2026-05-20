//! Pseudo-embeddings de archivos: vectores deterministas derivados de
//! metadatos (sin LLM).
//!
//! Implementan el "imán semántico" matemático que el diseño de Kairos
//! pide: cada archivo tiene un vector, cada Mónada tiene un centroide,
//! y un archivo nuevo se "pega" a la Mónada cuyo centroide está más
//! cerca (cosine similarity).
//!
//! No reemplaza embeddings reales (text-embedding de un LLM); sirve para:
//! - Bootstrapping sin Nous corriendo.
//! - Mock determinístico en `BRAHMAN_BROKER_CONTEXT=test`.
//! - Cohesión visual por path/extension (dos `.rs` en `src/` quedan
//!   muy juntos en el espacio vectorial).
//!
//! ## Forma del vector ([`EMBED_DIM`]=32, normalizado)
//!
//! - dims  0..8:  `blake3(extension)`     → identidad de tipo
//! - dims  8..16: `blake3(parent_dir)`    → identidad de contenedor
//! - dims 16..24: `blake3(file_stem)`     → identidad léxica del archivo
//! - dims 24..28: tamaño (log scale + flags binarios)
//! - dims 28..32: mtime (escala día + features cíclicas)
//!
//! ## Propiedades empíricas
//!
//! - Mismo dir + misma ext       → similitud > 0.7 (alta cohesión).
//! - Mismo dir + ext distinta    → similitud ~ 0.5.
//! - Dirs distintos + misma ext  → similitud ~ 0.5.
//! - Sin parecido                → similitud < 0.3.

use chasqui_card::{FileEntry, MonadId, MonadManifest};

/// Dimensión del vector embedding.
pub const EMBED_DIM: usize = 32;

/// Identificador del modelo que produce este embedding. Se usa para
/// taggear `MonadManifest.centroid_model`: los consumidores comparan
/// este string contra el suyo antes de hacer cosine similarity.
/// Mezclar centroides de distinto MODEL_ID corrompe scores
/// silenciosamente (dimensiones distintas, semántica distinta).
pub const MODEL_ID: &str = "chasqui-pseudo-32d";

/// Computa el embedding de un archivo. Determinístico: misma input
/// → mismo vector. El vector queda L2-normalizado.
pub fn embed(file: &FileEntry) -> [f32; EMBED_DIM] {
    let mut v = [0.0f32; EMBED_DIM];

    // dims 0..8: extension hash
    fill_from_hash(&mut v[0..8], file.extension.as_deref().unwrap_or(""));

    // dims 8..16: parent dir name hash
    let parent = file
        .path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("");
    fill_from_hash(&mut v[8..16], parent);

    // dims 16..24: file stem hash (sin extensión)
    let stem = file
        .path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    fill_from_hash(&mut v[16..24], stem);

    // dims 24..28: tamaño (centrado en 0 para que dot products entre
    // archivos de tamaño diferente sumen 0 en expectativa).
    let log_size = (file.size.max(1) as f32).log10();
    v[24] = ((log_size / 15.0).clamp(0.0, 1.0) - 0.5) * 2.0; // [-1, 1]
    v[25] = (log_size.fract() - 0.5) * 2.0;
    v[26] = if file.size >= 1_048_576 { 1.0 } else { -1.0 }; // ≥1MiB flag
    v[27] = if file.size <= 256 { 1.0 } else { -1.0 }; // ≤256B flag

    // dims 28..32: mtime — escala día + cíclicas (centradas).
    let day = file.mtime_ms / (86_400 * 1000);
    v[28] = (((day as f32) / 30_000.0).clamp(0.0, 1.0) - 0.5) * 2.0;
    v[29] = ((day % 365) as f32 / 365.0 - 0.5) * 2.0;
    v[30] = ((day % 30) as f32 / 30.0 - 0.5) * 2.0;
    v[31] = ((day % 7) as f32 / 7.0 - 0.5) * 2.0;

    normalize(&mut v);
    v
}

/// Fill `out` con bytes del hash blake3 de `input`, centrados en [-1, 1].
/// El centrado es crítico: bytes uniformes en [0,1] tienen media 0.5,
/// así dos vectores hash distintos (de strings no relacionados) tendrían
/// expected cosine similarity ≈ 0.75 (espuriamente alto). Centrarlos en
/// [-1, 1] hace que la expectativa sea ≈ 0 — propiedad necesaria para
/// que cosine similarity sea una métrica útil de afinidad.
fn fill_from_hash(out: &mut [f32], input: &str) {
    let h = blake3::hash(input.as_bytes());
    let bytes = h.as_bytes();
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = (bytes[i] as f32 - 127.5) / 127.5;
    }
}

/// L2-normaliza un vector in-place. Vectores con norma 0 quedan en 0.
fn normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Cosine similarity entre dos vectores. Asume ambos L2-normalizados
/// (en cuyo caso `dot product == cosine similarity`). Si las longitudes
/// no coinciden, devuelve 0.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Centroide de un set de vectores. Promedio dim-por-dim seguido de
/// L2-normalización. El resultado es un vector unidad apto para
/// comparar con miembros nuevos vía cosine similarity.
pub fn centroid(vectors: &[Vec<f32>]) -> Vec<f32> {
    if vectors.is_empty() {
        return Vec::new();
    }
    let dim = vectors[0].len();
    let mut c = vec![0.0f32; dim];
    for v in vectors {
        if v.len() != dim {
            continue;
        }
        for (i, x) in v.iter().enumerate() {
            c[i] += x;
        }
    }
    let n = vectors.len() as f32;
    for x in c.iter_mut() {
        *x /= n;
    }
    normalize(&mut c);
    c
}

/// Cohesión interna: media de cosine similarity de cada miembro contra
/// el centroide. Alta cohesión = Mónada compacta. Baja = bifurcable.
pub fn cohesion(centroid: &[f32], member_vectors: &[Vec<f32>]) -> f32 {
    if member_vectors.is_empty() || centroid.is_empty() {
        return 0.0;
    }
    let sum: f32 = member_vectors
        .iter()
        .map(|v| cosine_similarity(centroid, v))
        .sum();
    sum / member_vectors.len() as f32
}

/// Score de atracción de un archivo nuevo a una Mónada existente:
/// cosine similarity de su embedding contra el centroide de la Mónada.
/// Mayor score = mayor afinidad.
pub fn attraction_score(file_vec: &[f32], monad: &MonadManifest) -> f32 {
    if monad.centroid.is_empty() {
        return 0.0;
    }
    cosine_similarity(file_vec, &monad.centroid)
}

/// Encuentra la Mónada con mayor afinidad a un archivo. Devuelve
/// `(MonadId, score)` o `None` si ninguna tiene centroide.
pub fn best_attraction<'a, I>(file_vec: &[f32], monads: I) -> Option<(MonadId, f32)>
where
    I: IntoIterator<Item = &'a MonadManifest>,
{
    monads
        .into_iter()
        .filter(|m| !m.centroid.is_empty())
        .map(|m| (m.id, attraction_score(file_vec, m)))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
}

/// Umbral por defecto para "se pega": si el score es ≥ esto, el
/// archivo se asigna automáticamente. Ajustable por el caller.
pub const DEFAULT_ATTRACTION_THRESHOLD: f32 = 0.7;

#[cfg(test)]
mod tests {
    use super::*;
    use chasqui_card::FileId;
    use std::path::PathBuf;
    use ulid::Ulid;

    fn mk(path: &str, ext: Option<&str>, size: u64) -> FileEntry {
        FileEntry {
            id: FileId::from(Ulid::new()),
            path: PathBuf::from(path),
            content_hash: None,
            size,
            mtime_ms: 1_700_000_000_000, // fixed para que mtime no domine
            extension: ext.map(String::from),
        }
    }

    #[test]
    fn embed_is_deterministic() {
        let a = mk("/x/foo.rs", Some("rs"), 1024);
        let b = mk("/x/foo.rs", Some("rs"), 1024);
        let va = embed(&a);
        let vb = embed(&b);
        // Mismos metadatos → mismo vector (los IDs no entran al embedding).
        assert_eq!(va, vb);
    }

    #[test]
    fn embed_is_unit_normalized() {
        let f = mk("/x/foo.rs", Some("rs"), 1024);
        let v = embed(&f);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm={norm}");
    }

    #[test]
    fn same_dir_same_ext_high_similarity() {
        let a = embed(&mk("/proj/src/a.rs", Some("rs"), 1000));
        let b = embed(&mk("/proj/src/b.rs", Some("rs"), 1100));
        let sim = cosine_similarity(&a, &b);
        assert!(sim > 0.7, "esperaba sim > 0.7, fue {sim}");
    }

    #[test]
    fn unrelated_files_low_similarity() {
        let a = embed(&mk("/proj/src/main.rs", Some("rs"), 1000));
        let b = embed(&mk("/photos/2024/sunset.jpg", Some("jpg"), 5_000_000));
        let sim = cosine_similarity(&a, &b);
        assert!(sim < 0.5, "esperaba sim < 0.5, fue {sim}");
    }

    #[test]
    fn centroid_is_unit_and_close_to_members() {
        let v1 = embed(&mk("/x/a.rs", Some("rs"), 1000));
        let v2 = embed(&mk("/x/b.rs", Some("rs"), 1100));
        let v3 = embed(&mk("/x/c.rs", Some("rs"), 1200));
        let c = centroid(&[v1.to_vec(), v2.to_vec(), v3.to_vec()]);

        // Norma unitaria.
        let norm: f32 = c.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm={norm}");

        // Cohesión alta porque los miembros son similares.
        let cohesion = cohesion(&c, &[v1.to_vec(), v2.to_vec(), v3.to_vec()]);
        assert!(cohesion > 0.9, "cohesion={cohesion}");
    }

    #[test]
    fn attraction_picks_correct_monad() {
        // Construimos dos Mónadas: una de Rust, otra de imágenes.
        let rust_files = vec![
            embed(&mk("/proj/src/a.rs", Some("rs"), 1000)).to_vec(),
            embed(&mk("/proj/src/b.rs", Some("rs"), 1100)).to_vec(),
        ];
        let img_files = vec![
            embed(&mk("/photos/p1.jpg", Some("jpg"), 5_000_000)).to_vec(),
            embed(&mk("/photos/p2.jpg", Some("jpg"), 4_000_000)).to_vec(),
        ];

        let mut rust_monad = MonadManifest::new("rust");
        rust_monad.members.insert(FileId::from(Ulid::new()));
        rust_monad.touch();
        rust_monad.centroid = centroid(&rust_files);

        let mut img_monad = MonadManifest::new("photos");
        img_monad.members.insert(FileId::from(Ulid::new()));
        img_monad.touch();
        img_monad.centroid = centroid(&img_files);

        // Un archivo .rs nuevo en /proj/src debe atraerse a la Mónada Rust.
        let new_rs = embed(&mk("/proj/src/new.rs", Some("rs"), 1500));
        let (best_id, _score) = best_attraction(&new_rs, [&rust_monad, &img_monad].into_iter())
            .expect("best match");
        assert_eq!(best_id, rust_monad.id);

        // Y al revés.
        let new_jpg = embed(&mk("/photos/new.jpg", Some("jpg"), 6_000_000));
        let (best_id, _score) = best_attraction(&new_jpg, [&rust_monad, &img_monad].into_iter())
            .expect("best match");
        assert_eq!(best_id, img_monad.id);
    }

    #[test]
    fn empty_centroid_skipped_in_attraction() {
        let mut m = MonadManifest::new("empty");
        m.members.insert(FileId::from(Ulid::new()));
        m.touch();
        // m.centroid queda vacío

        let v = embed(&mk("/x/y.rs", Some("rs"), 100));
        assert!(best_attraction(&v, [&m].into_iter()).is_none());
    }
}
