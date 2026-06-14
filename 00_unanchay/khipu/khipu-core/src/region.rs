//! Regiones emergentes — el #3 del mapa mental.
//!
//! Un clúster denso de notas vecinas en el lienzo es una *región*: un
//! topónimo que **surge del layout** (proximidad + atención), no una
//! carpeta impuesta. Este módulo es la lógica agnóstica de dos cosas:
//!
//! 1. **Detección** ([`emergent_regions`]): a partir de los grupos por
//!    afinidad (que calcula `khipu-gravity`) y las notas ya colocadas,
//!    encuentra los clústeres lo bastante densos que todavía no tienen un
//!    topónimo cerca, y devuelve su centroide + miembros.
//! 2. **Asignación de nombre** ([`propose_region_name`]): propone un
//!    topónimo a partir del contenido del clúster — el término más
//!    recurrente y saliente de sus títulos y etiquetas. El frontend lo
//!    ofrece como nombre por defecto del bautizo; el usuario lo acepta o
//!    lo edita. La región se bautiza *después* de ver el patrón.
//!
//! Todo es puro y determinista — sin reloj, sin UI, sin física temporal
//! (la masa/visibilidad la filtra el caller antes de pasar las notas).

use crate::note::{Note, NoteId};

/// Mínimo de notas visibles para que un clúster cuente como región
/// emergente. Menos que esto no es una "zona", es ruido.
pub const REGION_MIN_MEMBERS: usize = 3;

/// Distancia de mundo dentro de la cual un topónimo ya "posee" un clúster:
/// si hay una región así de cerca del centroide, no se vuelve a ofrecer
/// bautizarla.
pub const REGION_MATCH_DIST: f32 = 140.0;

/// Una región emergente detectada en el mapa: un clúster denso de notas
/// vecinas, con su centroide de mundo, sus miembros y un nombre propuesto
/// del contenido. Es un candidato a topónimo, no una región ya bautizada.
#[derive(Debug, Clone, PartialEq)]
pub struct EmergentRegion {
    /// Centroide de mundo del clúster — dónde el mapa ofrece el bautizo.
    pub centroid: (f32, f32),
    /// Ids de las notas que forman el clúster (sólo las visibles/colocadas).
    pub members: Vec<NoteId>,
    /// Nombre propuesto a partir de los títulos y etiquetas del clúster.
    pub suggested_name: String,
}

/// Detecta las regiones emergentes a partir de los clústeres semánticos.
///
/// - `placed`: las notas **visibles y ya colocadas** (con `pos`). El
///   caller aplica antes el filtro de masa/horizonte — acá sólo importa
///   la geometría.
/// - `clusters`: los grupos por afinidad (`khipu-gravity::clusters`).
/// - `named_spots`: centroides de los topónimos ya existentes; un clúster
///   con una región a menos de `match_dist` no se re-ofrece.
/// - `min_members`: tamaño mínimo del clúster para contar como región.
/// - `match_dist`: radio de "posesión" de un topónimo existente.
///
/// Devuelve un candidato por clúster denso sin nombre cerca, en el mismo
/// orden que `clusters` (determinista).
pub fn emergent_regions(
    placed: &[&Note],
    clusters: &[Vec<NoteId>],
    named_spots: &[(f32, f32)],
    min_members: usize,
    match_dist: f32,
) -> Vec<EmergentRegion> {
    let d2 = match_dist * match_dist;
    let mut out = Vec::new();
    for cluster in clusters {
        // Miembros del clúster presentes en `placed` (visibles + colocados).
        let members: Vec<&Note> = cluster
            .iter()
            .filter_map(|id| placed.iter().copied().find(|n| n.id == *id && n.pos.is_some()))
            .collect();
        if members.len() < min_members {
            continue;
        }
        // Centroide de mundo: promedio de las anclas de los miembros.
        let (sx, sy) = members.iter().fold((0.0f32, 0.0f32), |(ax, ay), n| {
            let (x, y) = n.pos.unwrap_or((0.0, 0.0));
            (ax + x, ay + y)
        });
        let centroid = (sx / members.len() as f32, sy / members.len() as f32);
        // ¿Ya hay un topónimo dueño de este clúster? Entonces no se ofrece.
        let owned = named_spots
            .iter()
            .any(|(rx, ry)| (rx - centroid.0).powi(2) + (ry - centroid.1).powi(2) <= d2);
        if owned {
            continue;
        }
        let suggested_name =
            propose_region_name(&members).unwrap_or_else(|| "zona".to_string());
        out.push(EmergentRegion {
            centroid,
            members: members.iter().map(|n| n.id).collect(),
            suggested_name,
        });
    }
    out
}

/// Propone un topónimo para un grupo de notas: el término más recurrente
/// y saliente de sus títulos y etiquetas. Prefiere lo **compartido** (lo
/// que aparece en más notas es lo que vuelve "región" al clúster), luego
/// lo más frecuente, y desempata alfabético para ser determinista.
/// `None` si no hay ninguna palabra con señal (todo vacío o palabras
/// vacías).
pub fn propose_region_name(notes: &[&Note]) -> Option<String> {
    use std::collections::{BTreeMap, BTreeSet};

    // Por término: (en cuántas notas aparece, frecuencia total).
    let mut stats: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for n in notes {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for tok in note_tokens(n) {
            let e = stats.entry(tok.clone()).or_insert((0, 0));
            e.1 += 1;
            if seen.insert(tok) {
                e.0 += 1;
            }
        }
    }
    if stats.is_empty() {
        return None;
    }
    // max_by: gana doc-freq, luego freq total; en empate, el alfabético
    // menor (lo invertimos con `kb.cmp(ka)` para que el menor sea "mayor").
    let best = stats
        .iter()
        .max_by(|(ka, va), (kb, vb)| {
            va.0.cmp(&vb.0).then(va.1.cmp(&vb.1)).then(kb.cmp(ka))
        })
        .map(|(k, _)| k.clone())?;
    Some(capitalize(&best))
}

/// Tokens significativos de una nota: palabras del título + etiquetas,
/// en minúsculas, sin palabras vacías ni fragmentos cortos.
fn note_tokens(n: &Note) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |raw: &str, out: &mut Vec<String>| {
        let t = raw.to_lowercase();
        if t.chars().count() >= 3 && t.chars().any(|c| c.is_alphabetic()) && !is_stopword(&t) {
            out.push(t);
        }
    };
    for raw in n.title.split(|c: char| !c.is_alphanumeric()) {
        push(raw, &mut out);
    }
    for tag in &n.tags {
        for raw in tag.split(|c: char| !c.is_alphanumeric()) {
            push(raw, &mut out);
        }
    }
    out
}

/// Mayúscula inicial respetando unicode (para mostrar el topónimo).
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Palabras vacías (español, con algunas de inglés) que no sirven de
/// topónimo aunque sean frecuentes.
fn is_stopword(t: &str) -> bool {
    const STOP: &[&str] = &[
        "los", "las", "una", "unos", "unas", "del", "por", "para", "con", "sin", "que",
        "como", "más", "mas", "pero", "sus", "este", "esta", "esto", "estos", "estas",
        "ese", "esa", "eso", "esos", "esas", "the", "and", "for", "with", "you", "este",
        "muy", "son", "fue", "han", "hay", "sobre", "entre", "cuando", "donde", "porque",
    ];
    STOP.contains(&t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: NoteId, title: &str, tags: &[&str], pos: Option<(f32, f32)>) -> Note {
        Note {
            id,
            title: title.into(),
            body: String::new(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            created_at: 0,
            updated_at: 0,
            last_access: 0,
            mass: 1.0,
            pos,
        }
    }

    #[test]
    fn propose_picks_the_shared_term() {
        let a = note(1, "Receta de pan", &["cocina"], None);
        let b = note(2, "Horno y pan", &["cocina"], None);
        let c = note(3, "Masa madre", &["cocina"], None);
        // "cocina" está en las tres notas → gana por doc-freq.
        let name = propose_region_name(&[&a, &b, &c]).unwrap();
        assert_eq!(name, "Cocina");
    }

    #[test]
    fn propose_prefers_shared_over_merely_frequent() {
        // "pan" aparece 2 veces pero en una sola nota; "viaje" en dos notas.
        let a = note(1, "Pan pan pan", &[], None);
        let b = note(2, "Viaje a la costa", &[], None);
        let c = note(3, "Viaje largo", &[], None);
        let name = propose_region_name(&[&a, &b, &c]).unwrap();
        assert_eq!(name, "Viaje");
    }

    #[test]
    fn propose_ignores_stopwords_and_short_tokens() {
        let a = note(1, "El de la y", &[], None);
        let b = note(2, "Por que con", &[], None);
        // Todo son palabras vacías o de <3 letras → sin señal.
        assert!(propose_region_name(&[&a, &b]).is_none());
    }

    #[test]
    fn propose_is_deterministic_on_ties() {
        // "alpha" y "omega" empatan en doc-freq y freq → gana el alfabético.
        let a = note(1, "alpha omega", &[], None);
        let b = note(2, "omega alpha", &[], None);
        assert_eq!(propose_region_name(&[&a, &b]).unwrap(), "Alpha");
    }

    #[test]
    fn emergent_requires_min_members() {
        let a = note(1, "pan", &["cocina"], Some((0.0, 0.0)));
        let b = note(2, "horno", &["cocina"], Some((10.0, 0.0)));
        let placed = [&a, &b];
        let clusters = vec![vec![1u64, 2]];
        // Sólo 2 miembros, el mínimo es 3 → nada emerge.
        let regions = emergent_regions(&placed, &clusters, &[], REGION_MIN_MEMBERS, REGION_MATCH_DIST);
        assert!(regions.is_empty());
    }

    #[test]
    fn emergent_computes_centroid_members_and_name() {
        let a = note(1, "Receta de pan", &["cocina"], Some((0.0, 0.0)));
        let b = note(2, "Horno", &["cocina"], Some((30.0, 0.0)));
        let c = note(3, "Masa", &["cocina"], Some((0.0, 30.0)));
        let placed = [&a, &b, &c];
        let clusters = vec![vec![1u64, 2, 3]];
        let regions = emergent_regions(&placed, &clusters, &[], REGION_MIN_MEMBERS, REGION_MATCH_DIST);
        assert_eq!(regions.len(), 1);
        let r = &regions[0];
        assert_eq!(r.members, vec![1, 2, 3]);
        assert!((r.centroid.0 - 10.0).abs() < 1e-3);
        assert!((r.centroid.1 - 10.0).abs() < 1e-3);
        assert_eq!(r.suggested_name, "Cocina");
    }

    #[test]
    fn emergent_skips_clusters_already_named_nearby() {
        let a = note(1, "pan", &["cocina"], Some((0.0, 0.0)));
        let b = note(2, "horno", &["cocina"], Some((30.0, 0.0)));
        let c = note(3, "masa", &["cocina"], Some((0.0, 30.0)));
        let placed = [&a, &b, &c];
        let clusters = vec![vec![1u64, 2, 3]];
        // Topónimo ya pinchado encima del centroide (10,10) → no se re-ofrece.
        let named = [(12.0f32, 12.0f32)];
        let regions =
            emergent_regions(&placed, &clusters, &named, REGION_MIN_MEMBERS, REGION_MATCH_DIST);
        assert!(regions.is_empty());
    }

    #[test]
    fn emergent_ignores_unplaced_members() {
        let a = note(1, "pan", &["cocina"], Some((0.0, 0.0)));
        let b = note(2, "horno", &["cocina"], Some((30.0, 0.0)));
        let c = note(3, "masa", &["cocina"], None); // sin colocar
        let placed = [&a, &b, &c];
        let clusters = vec![vec![1u64, 2, 3]];
        // Sólo 2 colocados de 3 → bajo el mínimo, no emerge.
        let regions = emergent_regions(&placed, &clusters, &[], REGION_MIN_MEMBERS, REGION_MATCH_DIST);
        assert!(regions.is_empty());
    }

    #[test]
    fn emergent_falls_back_to_zona_without_text() {
        let a = note(1, "", &[], Some((0.0, 0.0)));
        let b = note(2, "", &[], Some((30.0, 0.0)));
        let c = note(3, "", &[], Some((0.0, 30.0)));
        let placed = [&a, &b, &c];
        let clusters = vec![vec![1u64, 2, 3]];
        let regions = emergent_regions(&placed, &clusters, &[], REGION_MIN_MEMBERS, REGION_MATCH_DIST);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].suggested_name, "zona");
    }
}
