//! `minga-store`: backing persistente con `sled` para los stores de Minga.
//!
//! Tres stores paralelos a los de `minga-core`:
//! - [`SledNodeStore`]: hashes → `StoredNode`s, equivalente persistente
//!   de `MemStore`.
//! - [`SledAttestationStore`]: pruebas criptográficas de autoría
//!   indexadas por content hash.
//! - [`SledMstStore`]: conjunto de claves del MST. La estructura
//!   probabilística del MST se reconstruye en memoria al cargar
//!   ([`SledMstStore::to_in_memory`]) — solo persistimos las claves
//!   porque el árbol es deterministicamente derivable de ellas.
//!
//! Una `PersistentRepo` agrupa los tres sobre una única `sled::Db`
//! (tres trees con namespaces separados).
//!
//! El núcleo (`minga-core`) sigue siendo agnóstico de IO: estos tipos
//! tienen APIs paralelas (devuelven `Result`, deserializan vía
//! postcard) y los protocolos de sync se quedan operando sobre los
//! tipos in-memory. La integración con `MingaPeer` (que hoy usa
//! `MemStore` concreto) llegará tras un trait genérico — esta
//! iteración se centra en que la capa de persistencia esté correcta
//! y testeada.

pub mod alpha_paths_store;
pub mod attestation_store;
pub mod error;
pub mod keypair_file;
pub mod mst_store;
pub mod node_store;
pub mod path_history_store;
pub mod repo;
pub mod retraction_store;
pub mod roots_store;
pub mod timestamp_store;

pub use alpha_paths_store::SledAlphaPathsStore;
pub use attestation_store::SledAttestationStore;
pub use error::StoreError;
pub use keypair_file::KeypairFileError;
pub use mst_store::SledMstStore;
pub use node_store::SledNodeStore;
pub use path_history_store::SledPathHistoryStore;
pub use repo::PersistentRepo;
pub use retraction_store::SledRetractionStore;
pub use roots_store::SledRootsStore;
pub use timestamp_store::SledTimestampStore;
