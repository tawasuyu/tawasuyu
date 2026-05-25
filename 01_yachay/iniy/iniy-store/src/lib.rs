//! iniy-store — persistencia SQLite del corpus, aserciones y matriz NLI.
//!
//! Esquema mínimo: documentos, chunks, aserciones, implicaciones.
//! Stub inicial: solo abre la DB y crea las tablas. Las queries reales
//! se cablean a medida que cada subcomando las necesita.

use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;

pub struct Store {
    pub conn: Connection,
}

impl Store {
    pub fn abrir(ruta: &Path) -> Result<Self> {
        let conn = Connection::open(ruta)?;
        let store = Self { conn };
        store.migrar()?;
        Ok(store)
    }

    pub fn en_memoria() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.migrar()?;
        Ok(store)
    }

    fn migrar(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS documentos (
                id      TEXT PRIMARY KEY,
                titulo  TEXT NOT NULL,
                creado  INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );
            CREATE TABLE IF NOT EXISTS chunks (
                id      TEXT PRIMARY KEY,
                doc_id  TEXT NOT NULL REFERENCES documentos(id),
                orden   INTEGER NOT NULL,
                texto   TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS aserciones (
                id              TEXT PRIMARY KEY,
                doc_id          TEXT NOT NULL REFERENCES documentos(id),
                chunk_id        TEXT NOT NULL REFERENCES chunks(id),
                texto           TEXT NOT NULL,
                opinion_json    TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS implicaciones (
                premisa     TEXT NOT NULL REFERENCES aserciones(id),
                hipotesis   TEXT NOT NULL REFERENCES aserciones(id),
                entailment      REAL NOT NULL,
                contradiction   REAL NOT NULL,
                neutral         REAL NOT NULL,
                PRIMARY KEY (premisa, hipotesis)
            );
            "#,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_en_memoria_migra_ok() {
        let _ = Store::en_memoria().unwrap();
    }
}
