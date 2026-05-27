//! Núcleo puro de Minga: AST normalizado, direccionamiento por contenido
//! semántico y Merkle Search Tree. Sin IO, sin red, sin filesystem.
//!
//! La separación es deliberada: este crate jamás importa libp2p, fuser ni
//! ningún tipo asociado a un canal de IO. Si algo aquí necesita IO, el
//! contrato se expone como trait y la implementación vive en otro crate.

pub mod alpha;
pub mod ast;
pub mod attestation;
pub mod cas;
pub mod identity;
pub mod mst;
pub mod parse;
pub mod retraction;
pub mod store;

pub use alpha::hash_node_alpha;
pub use ast::SemanticNode;
pub use attestation::{Attestation, AttestationError, AttestationStore};
pub use cas::{hash_components, hash_node, ContentHash};
pub use identity::{Did, Keypair, KeypairCryptoError, Signature};
pub use mst::{empty_subtree_hash, Mst, MstDiff, NodeProbe};
pub use retraction::{Retraction, RetractionError, RetractionStore, RETRACTION_DOMAIN};
pub use store::{hash_stored, MemStore, NodeStore, StoredNode};
