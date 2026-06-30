//! Resolución del **grafo de Mónadas**: contención (sub-Mónadas) +
//! cuerpo intensional (queries).
//!
//! El modelo (`chasqui-card`) ahora describe un DAG: una Mónada contiene
//! archivos (`members`) y/o otras Mónadas (`submonads`), y puede definir
//! su membresía por una regla ([`MonadQuery`]) en vez de a mano. Este
//! módulo es la lógica que *recorre* ese grafo:
//!
//! - [`resolve_query`] evalúa un predicado contra un corpus de archivos.
//! - [`effective_members`] da los archivos directos de una Mónada
//!   (curados + intensional + pines).
//! - [`transitive_files`] baja por las sub-Mónadas (guardando ciclos y
//!   diamantes) y junta todos los archivos alcanzables.
//! - [`child_monads`] resuelve las sub-Mónadas a sus manifiestos.
//!
//! Determinista y sin red. La hoja semántica [`MonadQuery::Near`] usa los
//! pseudo-embeddings de [`crate::embed`]; el resto es puramente léxico.

use std::collections::BTreeSet;

use chasqui_card::{FileEntry, FileId, MonadId, MonadManifest, MonadQuery};

use crate::cluster::lens_from_ext;
use crate::db::MonadDb;
use crate::embed;

/// Evalúa una [`MonadQuery`] contra un corpus y devuelve los IDs de los
/// archivos que la satisfacen.
///
/// `centroid` es el centroide de la Mónada dueña de la query — lo que la
/// hoja [`MonadQuery::Near`] usa como referencia de "cerca de esto". Si
/// la query no usa `Near`, `centroid` se ignora (pasá `&[]`).
pub fn resolve_query<'a, I>(query: &MonadQuery, files: I, centroid: &[f32]) -> BTreeSet<FileId>
where
    I: IntoIterator<Item = &'a FileEntry>,
{
    files
        .into_iter()
        .filter(|f| matches(query, f, centroid))
        .map(|f| f.id)
        .collect()
}

/// Decide si un archivo satisface la query. Recursivo sobre el álgebra.
fn matches(query: &MonadQuery, file: &FileEntry, centroid: &[f32]) -> bool {
    match query {
        MonadQuery::Extension { exts } => file
            .extension
            .as_deref()
            .is_some_and(|e| exts.contains(e)),
        // Lente determinista por extensión. Refinamiento futuro: discernir
        // por contenido (shuma-discern) cuando la extensión miente; hoy nos
        // quedamos en el mapeo barato y sin disco, coherente con el lente
        // que el clustering asigna.
        MonadQuery::Lens { lens } => lens_from_ext(file.extension.as_deref()) == *lens,
        MonadQuery::Near { min_similarity } => {
            if centroid.is_empty() {
                return false;
            }
            // Recalcula el embedding por nodo Near; las queries reales
            // tienen ≤1, así que no vale la pena cachear todavía.
            let v = embed::embed(file);
            embed::cosine_similarity(&v, centroid) >= *min_similarity
        }
        MonadQuery::All { of } => of.iter().all(|sub| matches(sub, file, centroid)),
        MonadQuery::Any { of } => of.iter().any(|sub| matches(sub, file, centroid)),
        MonadQuery::Not { inner } => !matches(inner, file, centroid),
    }
}

/// Archivos **directos** de una Mónada: la unión de sus miembros curados,
/// los miembros derivados de su cuerpo intensional (si tiene `query`), y
/// sus pines. No baja por las sub-Mónadas (para eso, [`transitive_files`]).
///
/// Mónada inexistente ⇒ conjunto vacío.
pub fn effective_members(db: &MonadDb, id: MonadId) -> BTreeSet<FileId> {
    let Some(m) = db.monad(id) else {
        return BTreeSet::new();
    };
    let mut out: BTreeSet<FileId> = m.members.iter().copied().collect();
    out.extend(m.pins.iter().copied());
    if let Some(query) = &m.query {
        out.extend(resolve_query(query, db.files(), &m.centroid));
    }
    out
}

/// Sub-Mónadas directas de una Mónada, resueltas a sus manifiestos.
/// Skipea silenciosamente IDs colgados (sub-Mónada borrada).
pub fn child_monads(db: &MonadDb, id: MonadId) -> Vec<&MonadManifest> {
    match db.monad(id) {
        Some(m) => m.submonads.iter().filter_map(|sid| db.monad(*sid)).collect(),
        None => Vec::new(),
    }
}

/// Todos los archivos alcanzables desde una Mónada bajando por el DAG de
/// sub-Mónadas. Cuenta cada archivo una sola vez aunque sea alcanzable por
/// varios caminos (es un `BTreeSet`), y termina aunque el grafo tenga
/// ciclos o diamantes gracias al set de visitados.
pub fn transitive_files(db: &MonadDb, root: MonadId) -> BTreeSet<FileId> {
    let mut visited: BTreeSet<MonadId> = BTreeSet::new();
    let mut files: BTreeSet<FileId> = BTreeSet::new();
    collect_files(db, root, &mut visited, &mut files);
    files
}

fn collect_files(
    db: &MonadDb,
    id: MonadId,
    visited: &mut BTreeSet<MonadId>,
    files: &mut BTreeSet<FileId>,
) {
    // Guarda de ciclos/diamantes: cada Mónada se visita una vez.
    if !visited.insert(id) {
        return;
    }
    let Some(m) = db.monad(id) else {
        return;
    };
    files.extend(effective_members(db, id));
    for sub in &m.submonads {
        collect_files(db, *sub, visited, files);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chasqui_card::{FileId, Lens};
    use std::collections::BTreeSet;
    use std::path::PathBuf;
    use ulid::Ulid;

    fn mk(path: &str, ext: Option<&str>, size: u64) -> FileEntry {
        FileEntry {
            id: FileId::from(Ulid::new()),
            path: PathBuf::from(path),
            content_hash: None,
            size,
            mtime_ms: 1_700_000_000_000,
            extension: ext.map(String::from),
        }
    }

    fn exts(list: &[&str]) -> MonadQuery {
        MonadQuery::Extension {
            exts: list.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn query_extension_selecciona_por_formato() {
        let corpus = vec![
            mk("/a/1.png", Some("png"), 100),
            mk("/a/2.jpg", Some("jpg"), 100),
            mk("/a/3.rs", Some("rs"), 100),
        ];
        let hit = resolve_query(&exts(&["png", "jpg"]), &corpus, &[]);
        assert_eq!(hit.len(), 2);
        assert!(hit.contains(&corpus[0].id));
        assert!(hit.contains(&corpus[1].id));
        assert!(!hit.contains(&corpus[2].id));
    }

    #[test]
    fn query_lens_gallery_capta_imagenes() {
        let corpus = vec![
            mk("/a/1.png", Some("png"), 100),
            mk("/a/2.heic", Some("heic"), 100),
            mk("/a/3.rs", Some("rs"), 100),
        ];
        let hit = resolve_query(&MonadQuery::imagenes(), &corpus, &[]);
        assert_eq!(hit.len(), 2, "png + heic son Gallery, rs no");
    }

    #[test]
    fn query_not_y_all_componen() {
        let corpus = vec![
            mk("/a/1.png", Some("png"), 100),
            mk("/a/2.rs", Some("rs"), 100),
        ];
        // "imágenes que NO sean png" → vacío (sólo hay png entre imágenes).
        let q = MonadQuery::All {
            of: vec![
                MonadQuery::imagenes(),
                MonadQuery::Not { inner: Box::new(exts(&["png"])) },
            ],
        };
        assert!(resolve_query(&q, &corpus, &[]).is_empty());
    }

    #[test]
    fn query_near_usa_centroide() {
        // Centroide de un cluster de .rs en /proj/src.
        let rust = vec![
            embed::embed(&mk("/proj/src/a.rs", Some("rs"), 1000)).to_vec(),
            embed::embed(&mk("/proj/src/b.rs", Some("rs"), 1100)).to_vec(),
        ];
        let centroid = embed::centroid(&rust);

        let corpus = vec![
            mk("/proj/src/c.rs", Some("rs"), 1200),       // cerca
            mk("/photos/x.jpg", Some("jpg"), 5_000_000),  // lejos
        ];
        let hit = resolve_query(&MonadQuery::Near { min_similarity: 0.7 }, &corpus, &centroid);
        assert!(hit.contains(&corpus[0].id), "el .rs debe caer cerca del centroide");
        assert!(!hit.contains(&corpus[1].id), "la foto debe quedar lejos");
    }

    #[test]
    fn near_sin_centroide_no_matchea() {
        let corpus = vec![mk("/a/x.rs", Some("rs"), 100)];
        let hit = resolve_query(&MonadQuery::Near { min_similarity: 0.0 }, &corpus, &[]);
        assert!(hit.is_empty(), "sin centroide, Near no puede decidir");
    }

    #[test]
    fn effective_members_une_curados_y_query() {
        let mut db = MonadDb::new();
        let png = mk("/a/1.png", Some("png"), 100);
        let rs = mk("/a/2.rs", Some("rs"), 100);
        let png_id = png.id;
        let rs_id = rs.id;
        db.insert_file(png.clone());
        db.insert_file(rs.clone());

        // Mónada intensional "imágenes" + un .rs fijado a mano como miembro.
        let mut m = MonadManifest::new("mixta");
        m.query = Some(MonadQuery::imagenes());
        m.members.insert(rs_id);
        m.touch();
        let mid = m.id;
        db.insert_monad(m);

        let eff = effective_members(&db, mid);
        assert!(eff.contains(&png_id), "el png entra por la query");
        assert!(eff.contains(&rs_id), "el rs entra por miembro curado");
        assert_eq!(eff.len(), 2);
    }

    #[test]
    fn transitive_baja_por_submonadas_y_dedup() {
        let mut db = MonadDb::new();
        let f1 = mk("/album/1.jpg", Some("jpg"), 100);
        let f2 = mk("/album/2.jpg", Some("jpg"), 100);
        let f1_id = f1.id;
        let f2_id = f2.id;
        db.insert_file(f1);
        db.insert_file(f2);

        // Álbum extensional con dos fotos.
        let mut album = MonadManifest::new("Viaje");
        album.members.insert(f1_id);
        album.members.insert(f2_id);
        album.touch();
        let album_id = album.id;
        db.insert_monad(album);

        // "Fotos" contiene al álbum (sub-Mónada) y además una de las fotos
        // directamente — el mismo archivo alcanzable por dos caminos.
        let mut fotos = MonadManifest::new("Fotos");
        fotos.submonads.insert(album_id);
        fotos.members.insert(f1_id);
        fotos.touch();
        let fotos_id = fotos.id;
        db.insert_monad(fotos);

        let all = transitive_files(&db, fotos_id);
        assert_eq!(all.len(), 2, "f1 contado una sola vez pese a dos caminos");
        assert!(all.contains(&f1_id) && all.contains(&f2_id));

        let kids = child_monads(&db, fotos_id);
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].label, "Viaje");
    }

    #[test]
    fn transitive_termina_con_ciclo() {
        let mut db = MonadDb::new();
        let f = mk("/x/a.rs", Some("rs"), 100);
        let f_id = f.id;
        db.insert_file(f);

        let mut a = MonadManifest::new("A");
        a.members.insert(f_id);
        let a_id = a.id;
        let mut b = MonadManifest::new("B");
        b.members.insert(f_id);
        let b_id = b.id;
        // Ciclo: A contiene B, B contiene A.
        a.submonads.insert(b_id);
        b.submonads.insert(a_id);
        a.touch();
        b.touch();
        db.insert_monad(a);
        db.insert_monad(b);

        // No debe colgarse; junta el único archivo.
        let all = transitive_files(&db, a_id);
        assert_eq!(all, BTreeSet::from([f_id]));
    }
}
