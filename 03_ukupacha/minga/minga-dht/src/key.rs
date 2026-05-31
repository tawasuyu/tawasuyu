//! Claves namespaced del DHT — **re-export** de la primitiva unificadora.
//!
//! `DhtKey`/`RecordKind` se mudaron a `card-net` (Capa 1 de Brahman) porque
//! son el namespace COMÚN que comparten minga, agora y card-discovery — no
//! pertenecen a un dominio concreto. Este módulo las re-exporta para no
//! romper los `use minga_dht::{DhtKey, RecordKind}` históricos.

pub use card_net::key::*;
