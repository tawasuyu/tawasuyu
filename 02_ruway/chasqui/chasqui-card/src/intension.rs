//! Cuerpo **intensional** de una Mónada: la regla que (re)deriva sus
//! miembros en cada scan, en vez de curarlos a mano.
//!
//! Una Mónada **extensional** lista sus miembros explícitamente (un
//! álbum: estas 40 fotos). Una Mónada **intensional** no guarda la
//! lista: guarda un *predicado* ([`MonadQuery`]) que el motor evalúa
//! contra el corpus para producir los miembros ("Fotos" = todo lo que
//! sea imagen, en cualquier parte). Es la misma idea que una
//! smart-playlist o un saved-search: la membresía es una consulta, no
//! una colección.
//!
//! Este crate sólo define el **tipo** de la regla (dato puro,
//! serializable). La **evaluación** —que necesita leer el corpus y, para
//! [`MonadQuery::Near`], los embeddings— vive en `chasqui-core::resolve`,
//! igual que el resto de la lógica de scan/cluster/attraction.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::Lens;

/// Predicado que define los miembros de una Mónada intensional.
///
/// Es un álgebra chica y cerrada bajo composición ([`All`](Self::All) /
/// [`Any`](Self::Any) / [`Not`](Self::Not)), de modo que casos reales se
/// expresan combinando hojas simples:
///
/// - `"Fotos"` = `Lens { lens: Gallery }` (o `Extension` de los formatos
///   de imagen, si se prefiere por extensión).
/// - `"Código Rust grande"` = `All { of: [Extension{rs}, ...] }`.
/// - `"Cerca de este viaje"` = `Near { min_similarity }`, comparando el
///   embedding de cada archivo contra el `centroid` de la propia Mónada
///   (que vive en el manifiesto, no acá — la query es agnóstica del
///   centroide concreto).
///
/// Las hojas [`Extension`](Self::Extension) y [`Lens`](Self::Lens) son
/// deterministas y baratas (no requieren embeddings); [`Near`](Self::Near)
/// es la hoja semántica que apoya en el modelo vectorial de `chasqui`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MonadQuery {
    /// Verdadero si la extensión del archivo (lowercase, sin punto) está
    /// en el conjunto. `{"png","jpg","jpeg",...}` = "imágenes por formato".
    Extension {
        /// Extensiones aceptadas, ya normalizadas a lowercase sin punto.
        exts: BTreeSet<String>,
    },
    /// Verdadero si el lente discernido del archivo coincide. Es el modo
    /// natural de "todo lo que se ve con tal vista" (Gallery, Code, …).
    Lens {
        /// Lente exigido.
        lens: Lens,
    },
    /// Verdadero si el embedding del archivo está a coseno ≥
    /// `min_similarity` del centroide de la Mónada. La hoja semántica:
    /// "lo que se parece a esto".
    Near {
        /// Umbral de similitud coseno en `[-1, 1]`. Típico ≈ 0.7.
        min_similarity: f32,
    },
    /// Conjunción: el archivo entra si satisface **todas** las
    /// subconsultas. `of` vacío ⇒ verdadero (neutro del AND).
    All {
        /// Subconsultas que deben cumplirse todas.
        of: Vec<MonadQuery>,
    },
    /// Disyunción: el archivo entra si satisface **alguna**. `of` vacío
    /// ⇒ falso (neutro del OR).
    Any {
        /// Subconsultas, basta una.
        of: Vec<MonadQuery>,
    },
    /// Negación: invierte la subconsulta.
    Not {
        /// Subconsulta a negar.
        inner: Box<MonadQuery>,
    },
}

impl MonadQuery {
    /// Atajo: una query de imágenes por las extensiones más comunes.
    /// Útil para sembrar la Mónada intensional canónica "Fotos".
    pub fn imagenes() -> Self {
        MonadQuery::Lens { lens: Lens::Gallery }
    }

    /// Atajo: una query por un único formato de archivo.
    pub fn extension(ext: impl Into<String>) -> Self {
        let mut exts = BTreeSet::new();
        exts.insert(ext.into().to_lowercase());
        MonadQuery::Extension { exts }
    }

    /// `true` si la query es puramente léxica (Extension/composición de
    /// Extension) y por lo tanto evaluable **sin** leer el contenido ni
    /// los embeddings — sólo mirando la ruta/extensión. Le sirve al
    /// motor para decidir si puede resolverla en frío.
    pub fn es_lexica(&self) -> bool {
        match self {
            MonadQuery::Extension { .. } => true,
            MonadQuery::Lens { .. } | MonadQuery::Near { .. } => false,
            MonadQuery::All { of } | MonadQuery::Any { of } => of.iter().all(Self::es_lexica),
            MonadQuery::Not { inner } => inner.es_lexica(),
        }
    }

    /// `true` si la query usa la hoja semántica [`Near`](Self::Near) en
    /// algún lugar — es decir, su evaluación necesita un centroide y los
    /// embeddings del corpus.
    pub fn usa_embeddings(&self) -> bool {
        match self {
            MonadQuery::Near { .. } => true,
            MonadQuery::Extension { .. } | MonadQuery::Lens { .. } => false,
            MonadQuery::All { of } | MonadQuery::Any { of } => of.iter().any(Self::usa_embeddings),
            MonadQuery::Not { inner } => inner.usa_embeddings(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_json_con_tag_op() {
        let q = MonadQuery::All {
            of: vec![
                MonadQuery::extension("rs"),
                MonadQuery::Not {
                    inner: Box::new(MonadQuery::Near { min_similarity: 0.8 }),
                },
            ],
        };
        let s = serde_json::to_string(&q).unwrap();
        assert!(s.contains(r#""op":"all""#), "{s}");
        let back: MonadQuery = serde_json::from_str(&s).unwrap();
        assert_eq!(back, q);
    }

    #[test]
    fn lexica_vs_semantica() {
        assert!(MonadQuery::extension("png").es_lexica());
        assert!(!MonadQuery::imagenes().es_lexica()); // Lens necesita discernir
        assert!(!MonadQuery::Near { min_similarity: 0.7 }.es_lexica());

        let mixta = MonadQuery::Any {
            of: vec![MonadQuery::extension("png"), MonadQuery::Near { min_similarity: 0.7 }],
        };
        assert!(!mixta.es_lexica());
        assert!(mixta.usa_embeddings());
        assert!(!MonadQuery::extension("png").usa_embeddings());
    }
}
