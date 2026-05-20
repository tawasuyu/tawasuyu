//! Clustering determinista (Phase A).
//!
//! Estrategia: agrupar por **directorio padre** + ranking por
//! **extensión dominante**. No hay LLM ni embeddings — sólo metadatos.
//! Esta capa cubre el 90% de los casos prácticos:
//!
//! - Un proyecto Rust en `~/dev/foo/src/` → Mónada coherente (.rs).
//! - Un dump de fotos en `~/Pictures/2024/` → Mónada con lente Gallery.
//! - Notas en `~/notes/` → Mónada con lente Markdown.
//!
//! Los casos donde esta heurística falla (archivos relacionados pero
//! dispersos en el FS) son el dominio de los embeddings (Phase C) y
//! del clustering por Nous (Phase D).

use std::collections::BTreeMap;
use std::path::PathBuf;

use chasqui_card::{FileEntry, Lens, MonadManifest};

use crate::embed;

/// Mínimo de archivos para que un directorio sea promovido a Mónada.
/// Por debajo de eso, los archivos quedan "huérfanos" (no asignados).
pub const DEFAULT_MIN_FILES_PER_MONAD: usize = 3;

/// Agrupa archivos en Mónadas por directorio padre.
///
/// Devuelve un `Vec<MonadManifest>` ordenado por path. Archivos en
/// directorios con menos de `min_files` no producen Mónada.
pub fn by_directory(files: &[FileEntry], min_files: usize) -> Vec<MonadManifest> {
    by_directory_hydrated(files, min_files, None)
}

/// Variante con hidratación: si `prior` está presente, busca Mónadas
/// previas con el mismo `path_hint` y `centroid_model` válido, y reusa
/// su `id` y `lineage`. Esto preserva identidad across re-scans —
/// fundamental para que el daemon pueda republicar tras hidratar de
/// sled sin generar duplicados en el broker.
pub fn by_directory_hydrated(
    files: &[FileEntry],
    min_files: usize,
    prior: Option<&crate::db::MonadDb>,
) -> Vec<MonadManifest> {
    let mut by_parent: BTreeMap<PathBuf, Vec<&FileEntry>> = BTreeMap::new();
    for f in files {
        if let Some(parent) = f.path.parent() {
            by_parent.entry(parent.to_path_buf()).or_default().push(f);
        }
    }

    let mut out = Vec::new();
    for (parent, group) in by_parent {
        if group.len() < min_files {
            continue;
        }
        let mut m = build_monad(&parent, &group);
        if let Some(db) = prior {
            // Reusamos id si encontramos Mónada previa con mismo
            // path_hint Y mismo centroid_model. Distintas hipótesis
            // de modelo no comparten identidad — son objetos
            // semánticos distintos, aunque parecidos.
            if let Some(existing) = db.monads().find(|prev| {
                prev.path_hint.as_deref() == m.path_hint.as_deref()
                    && prev.centroid_model == m.centroid_model
            }) {
                m.id = existing.id;
                m.lineage = existing.lineage;
                m.created_at_ms = existing.created_at_ms;
                m.touch();
            }
        }
        out.push(m);
    }
    out
}

fn build_monad(parent: &std::path::Path, group: &[&FileEntry]) -> MonadManifest {
    let label = label_from_path(parent);

    let keywords = top_extensions(group, 5);
    let lens = pick_lens(group);
    let entropy = shannon_entropy_normalized(group);

    let summary = build_summary(parent, group, &keywords);

    // Centroide vectorial: promedio de los embeddings de los miembros.
    // Esto es lo que permite "atracción" determinista de archivos
    // nuevos sin tocar Nous.
    let member_vecs: Vec<Vec<f32>> = group.iter().map(|f| embed::embed(f).to_vec()).collect();
    let centroid = embed::centroid(&member_vecs);

    let mut m = MonadManifest::new(label);
    m.summary = summary;
    m.keywords = keywords;
    m.dominant_lens = lens;
    m.entropy = entropy;
    m.centroid = centroid;
    // Taggeamos el centroide con su modelo. attract verifica esto
    // antes de comparar para no mezclar pseudo-32d con real-384d.
    m.centroid_model = Some(embed::MODEL_ID.to_string());
    // path_hint = identidad estable across re-scans para
    // hidratación. Display es lossy con UTF-8 inválido pero los
    // paths legítimos se imprimen consistentes.
    m.path_hint = Some(parent.display().to_string());
    m.members = group.iter().map(|f| f.id).collect();
    m.touch();
    m
}

/// Construye un label legible tomando los últimos hasta 2 componentes
/// del path. Esto desambigua `src/` repetidos en monorepos: en lugar
/// de 5 Mónadas con label "src", quedan "ente-zero/src", "ente-brain/src",
/// etc. Para directorios shallow (root o un nivel), cae al
/// `file_name()` simple.
fn label_from_path(p: &std::path::Path) -> String {
    let normals: Vec<&str> = p
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    if normals.is_empty() {
        return "unnamed".to_string();
    }
    let take = normals.len().min(2);
    let start = normals.len() - take;
    normals[start..].join("/")
}

fn build_summary(parent: &std::path::Path, group: &[&FileEntry], keywords: &[String]) -> String {
    let path_str = parent.display();
    let n = group.len();
    let exts = if keywords.is_empty() {
        "(sin extensiones)".to_string()
    } else {
        keywords.join(", ")
    };
    format!("{n} archivos en {path_str} (ext: {exts})")
}

/// Top-N extensiones por frecuencia, descendente. Empate por orden alfabético.
fn top_extensions(files: &[&FileEntry], n: usize) -> Vec<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for f in files {
        if let Some(ext) = &f.extension {
            *counts.entry(ext.clone()).or_default() += 1;
        }
    }
    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    sorted.into_iter().take(n).map(|(k, _)| k).collect()
}

/// Elige el lente dominante según la extensión más frecuente, con
/// fallback a `shuma-discern` sobre el head del archivo más
/// representativo cuando la extensión no da hint claro (Lens::Grid).
fn pick_lens(files: &[&FileEntry]) -> Lens {
    let dominant = top_extensions(files, 1).into_iter().next();
    let by_ext = match dominant.as_deref() {
        Some("rs" | "py" | "ts" | "tsx" | "js" | "jsx" | "go" | "java" | "kt" | "c" | "cpp"
        | "cc" | "h" | "hpp" | "rb" | "swift" | "zig") => Lens::Code,
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tiff" | "heic") => {
            Lens::Gallery
        }
        Some("md" | "markdown" | "rst" | "txt" | "org" | "tex") => Lens::Markdown,
        Some("db" | "sqlite" | "sqlite3" | "csv" | "tsv" | "parquet") => Lens::Database,
        _ => Lens::Grid,
    };
    if by_ext != Lens::Grid {
        return by_ext;
    }
    // Fallback: samplear el primer archivo del grupo con shuma-discern.
    // Sólo si tiene path real (FileEntry con path absoluto/relativo).
    if let Some(first) = files.first() {
        if let Some(lens) = discern_lens(&first.path) {
            return lens;
        }
    }
    Lens::Grid
}

fn discern_lens(path: &std::path::Path) -> Option<Lens> {
    use std::io::Read;
    let mut buf = vec![0u8; 4096];
    let mut f = std::fs::File::open(path).ok()?;
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    let pipeline = shuma_discern::DiscernPipeline::default_pipeline();
    let path_str = path.to_str();
    let d = pipeline.discern(
        &buf,
        &shuma_discern::Hint {
            path: path_str,
            size_total: None,
        },
    )?;
    match d.lens.as_deref()? {
        "code" => Some(Lens::Code),
        "gallery" => Some(Lens::Gallery),
        "markdown" => Some(Lens::Markdown),
        "database" => Some(Lens::Database),
        "tree" => Some(Lens::Tree),
        _ => None,
    }
}

/// Entropía de Shannon normalizada sobre la distribución de extensiones.
/// `0.0` = todos los archivos comparten extensión. `1.0` = uniformly
/// distributed entre `n` extensiones (máx información).
fn shannon_entropy_normalized(files: &[&FileEntry]) -> f32 {
    let total = files.len() as f32;
    if total <= 1.0 {
        return 0.0;
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for f in files {
        let ext = f.extension.as_deref().unwrap_or("(none)");
        *counts.entry(ext.to_string()).or_default() += 1;
    }
    let entropy: f32 = counts
        .values()
        .map(|&c| {
            let p = c as f32 / total;
            -p * p.log2()
        })
        .sum();
    let max_entropy = (counts.len() as f32).log2();
    if max_entropy <= 0.0 {
        0.0
    } else {
        (entropy / max_entropy).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chasqui_card::FileId;
    use std::path::PathBuf;
    use ulid::Ulid;

    fn mkfile(path: &str, ext: Option<&str>) -> FileEntry {
        FileEntry {
            id: FileId::from(Ulid::new()),
            path: PathBuf::from(path),
            content_hash: None,
            size: 100,
            mtime_ms: 0,
            extension: ext.map(String::from),
        }
    }

    #[test]
    fn groups_by_parent_directory() {
        let files = vec![
            mkfile("/proj/src/a.rs", Some("rs")),
            mkfile("/proj/src/b.rs", Some("rs")),
            mkfile("/proj/src/c.rs", Some("rs")),
            mkfile("/proj/docs/readme.md", Some("md")),
            mkfile("/proj/docs/guide.md", Some("md")),
            mkfile("/proj/docs/notes.md", Some("md")),
        ];
        let monads = by_directory(&files, 3);
        assert_eq!(monads.len(), 2);
        let labels: std::collections::BTreeSet<_> = monads.iter().map(|m| &m.label).collect();
        // Phase B: labels usan los últimos 2 componentes del path para
        // desambiguar (proj/src vs proj/docs en lugar de src vs docs).
        assert!(labels.iter().any(|l| l.as_str() == "proj/src"));
        assert!(labels.iter().any(|l| l.as_str() == "proj/docs"));
    }

    #[test]
    fn small_groups_not_promoted() {
        let files = vec![
            mkfile("/proj/single.txt", Some("txt")),
            mkfile("/proj/sub/a.txt", Some("txt")),
            mkfile("/proj/sub/b.txt", Some("txt")),
            mkfile("/proj/sub/c.txt", Some("txt")),
        ];
        // min=3 → /proj/single solo no se promueve, /proj/sub sí.
        let monads = by_directory(&files, 3);
        assert_eq!(monads.len(), 1);
        assert_eq!(monads[0].label, "proj/sub");
    }

    #[test]
    fn label_from_root_only_one_component() {
        // Un solo componente normal en el path → no hay "padre" útil.
        let p = std::path::Path::new("/onlyone");
        assert_eq!(label_from_path(p), "onlyone");
    }

    #[test]
    fn label_from_deep_path_takes_last_two() {
        let p = std::path::Path::new("/a/b/c/d/e");
        assert_eq!(label_from_path(p), "d/e");
    }

    #[test]
    fn lens_picked_by_dominant_extension() {
        let files = vec![
            mkfile("/x/a.rs", Some("rs")),
            mkfile("/x/b.rs", Some("rs")),
            mkfile("/x/c.rs", Some("rs")),
        ];
        let monads = by_directory(&files, 3);
        assert_eq!(monads[0].dominant_lens, Lens::Code);

        let files = vec![
            mkfile("/y/1.png", Some("png")),
            mkfile("/y/2.png", Some("png")),
            mkfile("/y/3.png", Some("png")),
        ];
        let monads = by_directory(&files, 3);
        assert_eq!(monads[0].dominant_lens, Lens::Gallery);
    }

    #[test]
    fn entropy_zero_for_homogeneous() {
        let files = vec![
            mkfile("/x/a.rs", Some("rs")),
            mkfile("/x/b.rs", Some("rs")),
            mkfile("/x/c.rs", Some("rs")),
        ];
        let monads = by_directory(&files, 3);
        assert_eq!(monads[0].entropy, 0.0);
    }

    #[test]
    fn entropy_high_for_diverse() {
        let files = vec![
            mkfile("/x/a.rs", Some("rs")),
            mkfile("/x/b.md", Some("md")),
            mkfile("/x/c.json", Some("json")),
            mkfile("/x/d.png", Some("png")),
        ];
        let monads = by_directory(&files, 3);
        // 4 extensiones distintas, distribución uniforme → entropy ≈ 1.0
        assert!(monads[0].entropy > 0.9, "got {}", monads[0].entropy);
    }

    #[test]
    fn top_extensions_orders_by_freq_then_alpha() {
        let files = vec![
            mkfile("/x/a.rs", Some("rs")),
            mkfile("/x/b.rs", Some("rs")),
            mkfile("/x/c.md", Some("md")),
            mkfile("/x/d.py", Some("py")),
        ];
        let refs: Vec<&FileEntry> = files.iter().collect();
        let top = top_extensions(&refs, 3);
        assert_eq!(top, vec!["rs", "md", "py"]);
    }
}
