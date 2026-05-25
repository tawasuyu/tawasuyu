//! `chasqui-core` — el explorador de Mónadas.
//!
//! Implementa la pipeline determinista descrita en el diseño de Kairos:
//!
//! 1. [`scanner`]: recorre directorios y emite [`FileEntry`] (sin tocar
//!    contenido en Phase 0 — sólo metadatos).
//! 2. [`cluster`]: agrupa archivos en [`MonadManifest`] usando
//!    heurísticas (parent dir + extensión dominante). 0 LLM.
//! 3. [`db`]: store en memoria con índices files↔monads.
//!
//! Pipeline:
//! ```text
//! scan_directory(path)
//!     → Vec<FileEntry>
//!         → cluster::by_directory(min_files=N)
//!             → Vec<MonadManifest>
//!                 → MonadDb::ingest(...)
//! ```
//!
//! Lo importante: en este crate no hay IA, no hay embeddings. Es la
//! capa determinista que cubre el 90% de los casos. Los embeddings
//! (`Phase C`) y Nous (`Phase D`) se enchufan después como módulos
//! separados que producen flows brahman.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod cluster;
pub mod db;
pub mod embed;
pub mod engine_socket;
pub mod scanner;

pub use chasqui_card::*;
