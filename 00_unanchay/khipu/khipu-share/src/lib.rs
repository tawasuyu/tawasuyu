//! `khipu-share` — compartir notas por la red soberana de gioser sin
//! perder su gravedad local.
//!
//! Lo que viaja es el **contenido** de la nota (título, cuerpo, etiquetas),
//! nunca su física temporal: la masa y el último acceso son la atención
//! *de quien tiene la nota*, no una propiedad transferible. Al importar,
//! cada nota nace fresca (`mass = 1.0`, `last_access = now`) — su gravedad
//! arranca en el cuaderno que la recibe.
//!
//! El sobre es:
//! - **direccionado por contenido**: su identidad es `BLAKE3(postcard(bundle))`,
//!   así dos sobres con las mismas notas tienen el mismo hash;
//! - **firmado Ed25519** sobre ese hash con la clave del autor, vía
//!   [`agora_core`] — verificable sin autoridad central ni red.
//!
//! ```
//! use agora_core::Keypair;
//! use khipu_share::{seal, open, SharedNote};
//!
//! let kp = Keypair::from_seed([7u8; 32]);
//! let notas = vec![SharedNote {
//!     title: "Receta".into(),
//!     body: "sopa; ver [[Mercado]]".into(),
//!     tags: vec!["cocina".into()],
//! }];
//! let sobre = seal(&kp, notas, 1_700_000_000).unwrap();
//!
//! // El receptor verifica firma + hash antes de confiar en el contenido.
//! let bundle = open(&sobre).unwrap();
//! assert_eq!(bundle.notes[0].title, "Receta");
//! ```

#![forbid(unsafe_code)]

pub mod discovery;
pub mod net;

use std::collections::HashSet;

use agora_core::{verify_signature, Keypair};
use khipu_core::{Note, NoteId, NoteStore};
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

/// El contenido transferible de una nota — sin id, masa ni timestamps.
/// La gravedad temporal se queda en el cuaderno de origen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedNote {
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
}

impl SharedNote {
    /// Proyecta una [`Note`] local a su contenido compartible, descartando
    /// id, masa y marcas de tiempo.
    pub fn from_note(n: &Note) -> Self {
        Self {
            title: n.title.clone(),
            body: n.body.clone(),
            tags: n.tags.clone(),
        }
    }
}

/// El cuerpo firmable: el autor, cuándo se selló, y las notas. Es lo que
/// se serializa y se hashea — el orden de las notas y las etiquetas es
/// significativo para que el hash sea estable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    /// Clave pública Ed25519 del autor (32 bytes).
    pub author: [u8; 32],
    /// Segundo Unix en que se selló el sobre.
    pub created_at: u64,
    pub notes: Vec<SharedNote>,
}

/// Un [`Bundle`] con la firma del autor sobre su hash de contenido.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedBundle {
    pub bundle: Bundle,
    /// Firma Ed25519 sobre [`Bundle::content_hash`].
    #[serde(with = "BigArray")]
    pub signature: [u8; 64],
}

impl Bundle {
    /// La dirección de contenido del sobre: `BLAKE3(postcard(self))`. Es
    /// también el mensaje que el autor firma.
    pub fn content_hash(&self) -> Result<[u8; 32], ShareError> {
        let bytes = postcard::to_allocvec(self).map_err(|_| ShareError::Serializacion)?;
        Ok(*blake3::hash(&bytes).as_bytes())
    }
}

impl SignedBundle {
    /// Serializa el sobre a bytes (postcard) para escribirlo a disco o
    /// mandarlo por cualquier canal.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ShareError> {
        postcard::to_allocvec(self).map_err(|_| ShareError::Serializacion)
    }

    /// Reconstruye un sobre desde bytes. No verifica la firma — para eso
    /// está [`open`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ShareError> {
        postcard::from_bytes(bytes).map_err(|_| ShareError::Serializacion)
    }

    /// La dirección de contenido del sobre (delegada a [`Bundle::content_hash`]).
    pub fn content_address(&self) -> Result<[u8; 32], ShareError> {
        self.bundle.content_hash()
    }
}

/// Sella las notas en un sobre firmado por `kp`.
pub fn seal(
    kp: &Keypair,
    notes: Vec<SharedNote>,
    created_at: u64,
) -> Result<SignedBundle, ShareError> {
    let bundle = Bundle {
        author: kp.public_key(),
        created_at,
        notes,
    };
    let hash = bundle.content_hash()?;
    let signature = kp.sign(&hash);
    Ok(SignedBundle { bundle, signature })
}

/// Abre un sobre: recomputa su hash de contenido y verifica que la firma
/// corresponda a la clave del autor declarado. Devuelve el [`Bundle`]
/// verificado, o un error si el contenido fue alterado o la firma no
/// corresponde. Verificación offline — sin autoridad central.
pub fn open(signed: &SignedBundle) -> Result<&Bundle, ShareError> {
    let hash = signed.bundle.content_hash()?;
    verify_signature(&signed.bundle.author, &hash, &signed.signature)
        .map_err(|_| ShareError::FirmaInvalida)?;
    Ok(&signed.bundle)
}

/// Resultado de ingerir un sobre en un cuaderno.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportOutcome {
    /// Ids de las notas efectivamente creadas.
    pub created: Vec<NoteId>,
    /// Notas omitidas por título ya presente (la importación repetida es
    /// inofensiva — no duplica).
    pub skipped: usize,
}

/// Inserta las notas de un `bundle` en `store` como notas nuevas: id e
/// historia local frescos, `mass = 1.0`, `last_access = now`. La gravedad
/// arranca en el cuaderno receptor.
///
/// Las notas cuyo título (sin distinguir mayúsculas) ya existe en el
/// cuaderno se omiten, de modo que importar dos veces el mismo sobre no
/// duplica. Los wiki-links `[[Título]]` sobreviven solos: khipu resuelve
/// enlaces por título, así que si las notas enlazadas también se importan,
/// el grafo se rearma sin remapear ids.
///
/// Cada nota importada recibe una etiqueta de procedencia [`tag_de`]
/// (`de:<hex8 del autor>`) — así queda asentado de quién vino sin tocar
/// el modelo `Note` ni la persistencia. Es una etiqueta normal: visible,
/// buscable y removible.
pub fn import_into(store: &mut NoteStore, bundle: &Bundle, now: u64) -> ImportOutcome {
    let mut existing: HashSet<String> =
        store.iter().map(|n| n.title.to_lowercase()).collect();
    let marca = tag_de(&bundle.author);
    let mut out = ImportOutcome::default();
    for sn in &bundle.notes {
        let key = sn.title.to_lowercase();
        // Sólo deduplicamos por títulos no vacíos: dos "(sin título)" no
        // son la misma nota.
        if !sn.title.is_empty() && existing.contains(&key) {
            out.skipped += 1;
            continue;
        }
        let mut tags = sn.tags.clone();
        if !tags.iter().any(|t| t == &marca) {
            tags.push(marca.clone());
        }
        let id = store.create(sn.title.clone(), sn.body.clone(), tags, now);
        existing.insert(key);
        out.created.push(id);
    }
    out
}

/// Prefijo hex (4 bytes / 8 hex) de una clave o hash — identifica un
/// autor de forma legible sin volcar los 32 bytes.
pub fn hex8(bytes: &[u8; 32]) -> String {
    bytes[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// La etiqueta de procedencia para un autor: `de:<hex8>`.
pub fn tag_de(author: &[u8; 32]) -> String {
    format!("de:{}", hex8(author))
}

/// Falla al sellar, abrir o transportar un sobre.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ShareError {
    #[error("no se pudo (de)serializar el sobre")]
    Serializacion,
    #[error("la firma no corresponde al contenido y la clave del autor")]
    FirmaInvalida,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nota(title: &str, body: &str, tags: &[&str]) -> SharedNote {
        SharedNote {
            title: title.into(),
            body: body.into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn seal_then_open_roundtrips() {
        let kp = Keypair::from_seed([1u8; 32]);
        let notes = vec![nota("A", "cuerpo a", &["x"]), nota("B", "[[A]]", &[])];
        let sobre = seal(&kp, notes.clone(), 42).unwrap();
        let bundle = open(&sobre).unwrap();
        assert_eq!(bundle.author, kp.public_key());
        assert_eq!(bundle.created_at, 42);
        assert_eq!(bundle.notes, notes);
    }

    #[test]
    fn content_hash_is_stable_and_content_addressed() {
        let kp = Keypair::from_seed([2u8; 32]);
        let notes = vec![nota("A", "a", &["t"])];
        let a = seal(&kp, notes.clone(), 100).unwrap();
        let b = seal(&kp, notes, 100).unwrap();
        // Mismas notas + mismo autor + mismo instante → mismo hash y firma.
        assert_eq!(a.content_address().unwrap(), b.content_address().unwrap());
        assert_eq!(a.signature, b.signature);
    }

    #[test]
    fn tampering_with_content_is_rejected() {
        let kp = Keypair::from_seed([3u8; 32]);
        let mut sobre = seal(&kp, vec![nota("real", "intacto", &[])], 1).unwrap();
        // Alteramos el cuerpo sin rehacer la firma.
        sobre.bundle.notes[0].body = "manipulado".into();
        assert_eq!(open(&sobre), Err(ShareError::FirmaInvalida));
    }

    #[test]
    fn forged_author_is_rejected() {
        let autor = Keypair::from_seed([4u8; 32]);
        let impostor = Keypair::from_seed([5u8; 32]);
        let mut sobre = seal(&autor, vec![nota("n", "c", &[])], 1).unwrap();
        // El impostor pone su clave pero no puede producir la firma válida.
        sobre.bundle.author = impostor.public_key();
        assert_eq!(open(&sobre), Err(ShareError::FirmaInvalida));
    }

    #[test]
    fn bytes_roundtrip_preserves_verification() {
        let kp = Keypair::from_seed([6u8; 32]);
        let sobre = seal(&kp, vec![nota("n", "c", &["a", "b"])], 9).unwrap();
        let bytes = sobre.to_bytes().unwrap();
        let recuperado = SignedBundle::from_bytes(&bytes).unwrap();
        assert_eq!(recuperado, sobre);
        assert!(open(&recuperado).is_ok());
    }

    #[test]
    fn import_creates_fresh_notes_with_local_gravity() {
        let kp = Keypair::from_seed([7u8; 32]);
        let sobre = seal(&kp, vec![nota("Receta", "sopa", &["cocina"])], 1).unwrap();
        let bundle = open(&sobre).unwrap();

        let mut store = NoteStore::new();
        let out = import_into(&mut store, bundle, 5_000);
        assert_eq!(out.created.len(), 1);
        assert_eq!(out.skipped, 0);

        let n = store.get(out.created[0]).unwrap();
        assert_eq!(n.title, "Receta");
        // Conserva sus etiquetas y suma la de procedencia.
        assert!(n.tags.contains(&"cocina".to_string()));
        assert!(n.tags.iter().any(|t| t.starts_with("de:")));
        // Gravedad fresca: masa plena y acceso = ahora del receptor.
        assert_eq!(n.mass, 1.0);
        assert_eq!(n.last_access, 5_000);
        assert_eq!(n.created_at, 5_000);
    }

    #[test]
    fn import_marks_author_provenance() {
        let kp = Keypair::from_seed([13u8; 32]);
        let sobre = seal(&kp, vec![nota("N", "c", &[])], 1).unwrap();
        let bundle = open(&sobre).unwrap();
        let mut store = NoteStore::new();
        let out = import_into(&mut store, bundle, 1);
        let n = store.get(out.created[0]).unwrap();
        assert!(n.tags.contains(&tag_de(&kp.public_key())));
    }

    #[test]
    fn reimport_skips_existing_titles() {
        let kp = Keypair::from_seed([8u8; 32]);
        let sobre = seal(
            &kp,
            vec![nota("A", "a", &[]), nota("B", "b", &[])],
            1,
        )
        .unwrap();
        let bundle = open(&sobre).unwrap();

        let mut store = NoteStore::new();
        let first = import_into(&mut store, bundle, 10);
        assert_eq!(first.created.len(), 2);

        // Segunda importación del mismo sobre: nada nuevo.
        let second = import_into(&mut store, bundle, 20);
        assert_eq!(second.created.len(), 0);
        assert_eq!(second.skipped, 2);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn wiki_links_survive_import_by_title() {
        let kp = Keypair::from_seed([9u8; 32]);
        let sobre = seal(
            &kp,
            vec![
                nota("Índice", "ver [[Receta]]", &[]),
                nota("Receta", "sopa", &[]),
            ],
            1,
        )
        .unwrap();
        let bundle = open(&sobre).unwrap();

        let mut store = NoteStore::new();
        let out = import_into(&mut store, bundle, 1);
        let indice = out.created[0];
        // El enlace [[Receta]] resuelve a la otra nota importada.
        assert_eq!(store.forward_links(indice).len(), 1);
    }
}
