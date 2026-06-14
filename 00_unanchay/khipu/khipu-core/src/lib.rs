//! `khipu_app-core` — el núcleo agnóstico de la toma de notas.
//!
//! Una nota es texto con título, etiquetas y enlaces `[[...]]`. El
//! [`NoteStore`] las guarda y deriva el grafo: forward-links, backlinks,
//! huérfanas y enlaces colgantes. Sin UI, sin storage en disco, sin red
//! — tipos puros y deterministas.
//!
//! - [`note`] — el modelo [`Note`].
//! - [`links`] — el parser de wiki-links `[[...]]`.
//! - [`store`] — el [`NoteStore`] y el grafo de enlaces.
//! - [`region`] — regiones emergentes del mapa: detección de clústeres
//!   densos + propuesta de topónimo (el #3 del mapa mental).
//!
//! La gravedad semántica (clustering por afinidad de embeddings) vive en
//! `khipu_app-gravity`; las lentes visuales, en los crates de frontend.

#![forbid(unsafe_code)]

pub mod links;
pub mod note;
pub mod region;
pub mod store;

pub use links::parse_links;
pub use note::{Note, NoteId};
pub use region::{emergent_regions, propose_region_name, EmergentRegion, REGION_MATCH_DIST, REGION_MIN_MEMBERS};
pub use store::NoteStore;
