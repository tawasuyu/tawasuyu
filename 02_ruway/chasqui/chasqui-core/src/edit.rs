//! Edición del **grafo de Mónadas**: las mutaciones que convierten a
//! nahual en un *file manager* (no un mero visor).
//!
//! "Para manejar mis datos los voy a querer modificar, y submonadizar": el
//! usuario edita la **organización** (el grafo), no necesariamente los
//! bytes de los archivos. Estas operaciones reorganizan Mónadas y su
//! pertenencia sin mover un solo archivo en disco — el archivo físico es
//! agnóstico a en cuántas Mónadas vive.
//!
//! Todas las mutaciones siguen el patrón **clonar→mutar→reinsertar** sobre
//! [`MonadDb`], para que la escritura pase por `insert_monad` y herede el
//! write-through a sled cuando el store es persistente.
//!
//! Invariantes que esta capa mantiene:
//! - El grafo de contención es un **DAG**: [`add_submonad`] /
//!   [`submonadize`] rechazan cualquier arista que cierre un ciclo.
//! - Al cambiar los miembros de una Mónada, su `centroid` se recalcula
//!   (vía [`crate::embed`]) para que la atracción y las queries `Near`
//!   sigan siendo significativas.
//! - [`delete_monad`] y [`merge`] dejan el grafo coherente: ningún padre
//!   queda apuntando a una sub-Mónada que ya no existe.

use chasqui_card::{FileId, Lens, MonadId, MonadManifest, MonadQuery};
use thiserror::Error;

use crate::db::MonadDb;
use crate::embed;

/// Error de una operación de edición del grafo.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EditError {
    #[error("Mónada inexistente: {0}")]
    NotFound(MonadId),
    /// Agregar `child` bajo `parent` cerraría un ciclo, porque `parent` ya
    /// es alcanzable desde `child` (o son la misma).
    #[error("ciclo: {child} ya contiene (transitivamente) a {parent}")]
    Cycle { parent: MonadId, child: MonadId },
}

/// `true` si `target` es alcanzable desde `from` bajando por sub-Mónadas
/// (incluye el caso `from == target`). Sirve para detectar ciclos antes de
/// agregar una arista de contención.
fn reachable(db: &MonadDb, from: MonadId, target: MonadId) -> bool {
    if from == target {
        return true;
    }
    let mut stack = vec![from];
    let mut visto = std::collections::BTreeSet::new();
    while let Some(id) = stack.pop() {
        if !visto.insert(id) {
            continue;
        }
        if let Some(m) = db.monad(id) {
            for sub in &m.submonads {
                if *sub == target {
                    return true;
                }
                stack.push(*sub);
            }
        }
    }
    false
}

/// Recalcula el centroide de una Mónada desde los embeddings de sus
/// miembros actuales. Sin miembros, el centroide queda vacío (y `Near`
/// deja de matchear, como corresponde).
fn recompute_centroid(db: &mut MonadDb, id: MonadId) {
    let Some(m) = db.monad(id) else { return };
    let mut m = m.clone();
    let vecs: Vec<Vec<f32>> = m
        .members
        .iter()
        .filter_map(|fid| db.file(*fid))
        .map(|f| embed::embed(f).to_vec())
        .collect();
    m.centroid = embed::centroid(&vecs);
    m.centroid_model = if m.centroid.is_empty() {
        None
    } else {
        Some(embed::MODEL_ID.to_string())
    };
    m.touch();
    db.insert_monad(m);
}

/// Crea una Mónada vacía con un label y un lente, y la inserta. Devuelve
/// su id. Queda sin miembros hasta que se le agreguen (o se le ponga una
/// query); recién ahí valida como no-vacía.
pub fn create_monad(db: &mut MonadDb, label: impl Into<String>, lens: Lens) -> MonadId {
    let mut m = MonadManifest::new(label);
    m.dominant_lens = lens;
    m.touch();
    let id = m.id;
    db.insert_monad(m);
    id
}

/// Renombra una Mónada.
pub fn rename(db: &mut MonadDb, id: MonadId, label: impl Into<String>) -> Result<(), EditError> {
    let mut m = db.monad(id).ok_or(EditError::NotFound(id))?.clone();
    m.label = label.into();
    m.touch();
    db.insert_monad(m);
    Ok(())
}

/// Cambia el lente dominante (la "vista" que el shell elige para la
/// Mónada).
pub fn set_lens(db: &mut MonadDb, id: MonadId, lens: Lens) -> Result<(), EditError> {
    let mut m = db.monad(id).ok_or(EditError::NotFound(id))?.clone();
    m.dominant_lens = lens;
    m.touch();
    db.insert_monad(m);
    Ok(())
}

/// Fija (o quita, con `None`) el cuerpo intensional. Convertir una Mónada
/// extensional en intensional no borra sus `members` curados: la query
/// *suma* (ver `resolve::effective_members`).
pub fn set_query(
    db: &mut MonadDb,
    id: MonadId,
    query: Option<MonadQuery>,
) -> Result<(), EditError> {
    let mut m = db.monad(id).ok_or(EditError::NotFound(id))?.clone();
    m.query = query;
    m.touch();
    db.insert_monad(m);
    Ok(())
}

/// Agrega un archivo como miembro curado de la Mónada y recalcula el
/// centroide.
pub fn add_member(db: &mut MonadDb, id: MonadId, file: FileId) -> Result<(), EditError> {
    let mut m = db.monad(id).ok_or(EditError::NotFound(id))?.clone();
    m.members.insert(file);
    m.touch();
    db.insert_monad(m);
    recompute_centroid(db, id);
    Ok(())
}

/// Quita un archivo de los miembros curados (y de los pines). Recalcula el
/// centroide.
pub fn remove_member(db: &mut MonadDb, id: MonadId, file: FileId) -> Result<(), EditError> {
    let mut m = db.monad(id).ok_or(EditError::NotFound(id))?.clone();
    m.members.remove(&file);
    m.pins.remove(&file);
    m.touch();
    db.insert_monad(m);
    recompute_centroid(db, id);
    Ok(())
}

/// Agrega `child` como sub-Mónada de `parent`. Rechaza la arista si
/// cerraría un ciclo (si `parent` ya es alcanzable desde `child`).
pub fn add_submonad(db: &mut MonadDb, parent: MonadId, child: MonadId) -> Result<(), EditError> {
    if db.monad(parent).is_none() {
        return Err(EditError::NotFound(parent));
    }
    if db.monad(child).is_none() {
        return Err(EditError::NotFound(child));
    }
    if reachable(db, child, parent) {
        return Err(EditError::Cycle { parent, child });
    }
    let mut m = db.monad(parent).unwrap().clone();
    m.submonads.insert(child);
    m.touch();
    db.insert_monad(m);
    Ok(())
}

/// Quita la arista de contención `parent → child`. No borra la sub-Mónada,
/// sólo la desvincula de ese padre (puede seguir colgando de otros).
pub fn remove_submonad(
    db: &mut MonadDb,
    parent: MonadId,
    child: MonadId,
) -> Result<(), EditError> {
    let mut m = db.monad(parent).ok_or(EditError::NotFound(parent))?.clone();
    m.submonads.remove(&child);
    m.touch();
    db.insert_monad(m);
    Ok(())
}

/// **Submonadizar**: crea una nueva Mónada hija de `parent` y le *traslada*
/// la selección de archivos y sub-Mónadas indicada (la saca de `parent` y
/// la mete en la hija). Es la operación canónica de "tomar un puñado de
/// cosas dentro de una Mónada y agruparlas en una sub-Mónada propia".
///
/// La hija hereda el lente de `parent` por defecto; su centroide se calcula
/// de los archivos trasladados. Devuelve el id de la hija.
pub fn submonadize(
    db: &mut MonadDb,
    parent: MonadId,
    label: impl Into<String>,
    members: &[FileId],
    submonads: &[MonadId],
) -> Result<MonadId, EditError> {
    let padre = db.monad(parent).ok_or(EditError::NotFound(parent))?.clone();

    // La hija nace con la selección y el lente del padre.
    let mut hija = MonadManifest::new(label);
    hija.dominant_lens = padre.dominant_lens;
    hija.lineage = Some(parent);
    for f in members {
        hija.members.insert(*f);
    }
    for s in submonads {
        hija.submonads.insert(*s);
    }
    hija.touch();
    let hija_id = hija.id;
    db.insert_monad(hija);
    recompute_centroid(db, hija_id);

    // El padre suelta la selección y adopta a la hija.
    let mut padre = padre;
    for f in members {
        padre.members.remove(f);
        padre.pins.remove(f);
    }
    for s in submonads {
        padre.submonads.remove(s);
    }
    padre.submonads.insert(hija_id);
    padre.touch();
    db.insert_monad(padre);
    recompute_centroid(db, parent);

    Ok(hija_id)
}

/// **Fusionar** `from` en `into`: traslada todos los miembros y
/// sub-Mónadas de `from` a `into`, repunta a todo padre que apuntaba a
/// `from` para que ahora apunte a `into`, y borra `from`. El grafo queda
/// sin referencias colgadas.
pub fn merge(db: &mut MonadDb, into: MonadId, from: MonadId) -> Result<(), EditError> {
    if into == from {
        return Ok(());
    }
    let origen = db.monad(from).ok_or(EditError::NotFound(from))?.clone();
    let mut destino = db.monad(into).ok_or(EditError::NotFound(into))?.clone();

    for f in &origen.members {
        destino.members.insert(*f);
    }
    for p in &origen.pins {
        destino.pins.insert(*p);
    }
    for s in &origen.submonads {
        if *s != into {
            destino.submonads.insert(*s);
        }
    }
    destino.submonads.remove(&from);
    destino.touch();
    db.insert_monad(destino);

    // Repuntar padres de `from` → `into`.
    let padres: Vec<MonadId> = db
        .monads()
        .filter(|m| m.submonads.contains(&from))
        .map(|m| m.id)
        .collect();
    for pid in padres {
        if pid == into {
            continue;
        }
        let mut p = db.monad(pid).unwrap().clone();
        p.submonads.remove(&from);
        if pid != into {
            p.submonads.insert(into);
        }
        p.touch();
        db.insert_monad(p);
    }

    db.remove_monad(from);
    recompute_centroid(db, into);
    Ok(())
}

/// Borra una Mónada y la desvincula de todo padre que la contenía. Los
/// archivos y sub-Mónadas que contenía NO se borran — sólo se disuelve el
/// agrupamiento.
pub fn delete_monad(db: &mut MonadDb, id: MonadId) {
    let padres: Vec<MonadId> = db
        .monads()
        .filter(|m| m.submonads.contains(&id))
        .map(|m| m.id)
        .collect();
    for pid in padres {
        if let Some(p) = db.monad(pid) {
            let mut p = p.clone();
            p.submonads.remove(&id);
            p.touch();
            db.insert_monad(p);
        }
    }
    db.remove_monad(id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve;
    use chasqui_card::FileEntry;
    use std::path::PathBuf;
    use ulid::Ulid;

    fn mk(path: &str, ext: &str) -> FileEntry {
        FileEntry {
            id: FileId::from(Ulid::new()),
            path: PathBuf::from(path),
            content_hash: None,
            size: 100,
            mtime_ms: 1_700_000_000_000,
            extension: Some(ext.into()),
        }
    }

    fn db_con_archivos(n: usize) -> (MonadDb, Vec<FileId>) {
        let mut db = MonadDb::new();
        let mut ids = Vec::new();
        for i in 0..n {
            let f = mk(&format!("/proj/src/f{i}.rs"), "rs");
            ids.push(f.id);
            db.insert_file(f);
        }
        (db, ids)
    }

    #[test]
    fn crear_agregar_y_recalcular_centroide() {
        let (mut db, files) = db_con_archivos(2);
        let m = create_monad(&mut db, "src", Lens::Code);
        assert!(db.monad(m).unwrap().centroid.is_empty());

        add_member(&mut db, m, files[0]).unwrap();
        add_member(&mut db, m, files[1]).unwrap();
        let mon = db.monad(m).unwrap();
        assert_eq!(mon.cardinality, 2);
        assert!(!mon.centroid.is_empty(), "el centroide se recalcula al agregar");
        assert_eq!(mon.centroid_model.as_deref(), Some(embed::MODEL_ID));

        remove_member(&mut db, m, files[0]).unwrap();
        assert_eq!(db.monad(m).unwrap().cardinality, 1);
    }

    #[test]
    fn add_submonad_rechaza_ciclo() {
        let mut db = MonadDb::new();
        let a = create_monad(&mut db, "A", Lens::Grid);
        let b = create_monad(&mut db, "B", Lens::Grid);
        add_submonad(&mut db, a, b).unwrap(); // A ⊃ B
        // B ⊃ A cerraría el ciclo.
        assert_eq!(add_submonad(&mut db, b, a), Err(EditError::Cycle { parent: b, child: a }));
        // A ⊃ A también.
        assert_eq!(add_submonad(&mut db, a, a), Err(EditError::Cycle { parent: a, child: a }));
    }

    #[test]
    fn submonadizar_traslada_la_seleccion() {
        let (mut db, files) = db_con_archivos(3);
        let padre = create_monad(&mut db, "todo", Lens::Code);
        for f in &files {
            add_member(&mut db, padre, *f).unwrap();
        }
        assert_eq!(db.monad(padre).unwrap().cardinality, 3);

        // Submonadizar dos de los tres archivos a una hija "sub".
        let hija = submonadize(&mut db, padre, "sub", &files[0..2], &[]).unwrap();

        // El padre queda con 1 archivo + la hija como sub-Mónada.
        let p = db.monad(padre).unwrap();
        assert_eq!(p.cardinality, 1, "el padre soltó 2 archivos");
        assert!(p.submonads.contains(&hija));

        // La hija tiene los 2 archivos y lineage al padre.
        let h = db.monad(hija).unwrap();
        assert_eq!(h.cardinality, 2);
        assert_eq!(h.lineage, Some(padre));

        // Los 3 archivos siguen alcanzables transitivamente desde el padre.
        assert_eq!(resolve::transitive_files(&db, padre).len(), 3);
    }

    #[test]
    fn fusionar_repunta_padres_y_borra_origen() {
        let (mut db, files) = db_con_archivos(2);
        let raiz = create_monad(&mut db, "raíz", Lens::Grid);
        let a = create_monad(&mut db, "A", Lens::Code);
        let b = create_monad(&mut db, "B", Lens::Code);
        add_member(&mut db, a, files[0]).unwrap();
        add_member(&mut db, b, files[1]).unwrap();
        add_submonad(&mut db, raiz, a).unwrap();
        add_submonad(&mut db, raiz, b).unwrap();

        // Fusionar B en A: A se queda con ambos archivos, B desaparece, y
        // raíz deja de apuntar a B.
        merge(&mut db, a, b).unwrap();
        assert!(db.monad(b).is_none(), "B se borró");
        assert_eq!(db.monad(a).unwrap().cardinality, 2);
        let raiz = db.monad(raiz).unwrap();
        assert!(raiz.submonads.contains(&a));
        assert!(!raiz.submonads.contains(&b), "raíz no apunta a una Mónada borrada");
    }

    #[test]
    fn borrar_desvincula_de_padres() {
        let mut db = MonadDb::new();
        let raiz = create_monad(&mut db, "raíz", Lens::Grid);
        let hija = create_monad(&mut db, "hija", Lens::Grid);
        add_submonad(&mut db, raiz, hija).unwrap();

        delete_monad(&mut db, hija);
        assert!(db.monad(hija).is_none());
        assert!(!db.monad(raiz).unwrap().submonads.contains(&hija), "sin referencia colgada");
    }

    #[test]
    fn set_query_convierte_a_intensional_sin_perder_curados() {
        let (mut db, files) = db_con_archivos(1);
        let png = mk("/a/foto.png", "png");
        let png_id = png.id;
        db.insert_file(png);

        let m = create_monad(&mut db, "mix", Lens::Grid);
        add_member(&mut db, m, files[0]).unwrap(); // un .rs curado
        set_query(&mut db, m, Some(MonadQuery::imagenes())).unwrap();

        // effective_members = el .rs curado + el .png por la query.
        let eff = resolve::effective_members(&db, m);
        assert!(eff.contains(&files[0]));
        assert!(eff.contains(&png_id));
        assert_eq!(eff.len(), 2);
    }
}
