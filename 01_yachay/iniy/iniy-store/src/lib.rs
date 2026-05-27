//! iniy-store — persistencia SQLite del corpus, aserciones y matriz NLI.
//!
//! Esquema mínimo: documentos, chunks, aserciones, implicaciones.
//! Las queries se cablean a medida que cada subcomando las necesita.

use anyhow::{Context, Result};
use iniy_core::{ChunkId, DocId};
use iniy_ingest::{Chunk, Documento};
use rusqlite::{params, Connection};
use std::path::Path;
use std::str::FromStr;
use ulid::Ulid;

pub struct Store {
    pub conn: Connection,
}

#[derive(Debug, Clone)]
pub struct DocumentoResumen {
    pub id: DocId,
    pub titulo: String,
    pub n_chunks: u32,
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
            CREATE INDEX IF NOT EXISTS idx_chunks_doc ON chunks(doc_id, orden);
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

    /// Persiste un documento recién ingerido (doc + chunks) en una sola transacción.
    /// Si el `doc.id` ya existe, falla — los IDs son Ulid recién acuñados, colisión = bug.
    pub fn persistir_documento(&mut self, doc: &Documento) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO documentos (id, titulo) VALUES (?1, ?2)",
            params![doc.id.0.to_string(), doc.titulo],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO chunks (id, doc_id, orden, texto) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for c in &doc.chunks {
                stmt.execute(params![
                    c.id.0.to_string(),
                    c.doc_id.0.to_string(),
                    c.orden,
                    c.texto,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Lista todos los documentos con su cantidad de chunks, más recientes primero.
    pub fn listar_documentos(&self) -> Result<Vec<DocumentoResumen>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT d.id, d.titulo, COUNT(c.id) AS n
            FROM documentos d
            LEFT JOIN chunks c ON c.doc_id = d.id
            GROUP BY d.id
            ORDER BY d.creado DESC, d.id DESC
            "#,
        )?;
        let rows = stmt.query_map([], |r| {
            let id_s: String = r.get(0)?;
            let titulo: String = r.get(1)?;
            let n: i64 = r.get(2)?;
            Ok((id_s, titulo, n))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id_s, titulo, n) = r?;
            let ulid = Ulid::from_str(&id_s).with_context(|| format!("doc_id inválido: {id_s}"))?;
            out.push(DocumentoResumen { id: DocId(ulid), titulo, n_chunks: n as u32 });
        }
        Ok(out)
    }

    /// Carga los chunks de un documento, ordenados.
    pub fn cargar_chunks(&self, doc_id: DocId) -> Result<Vec<Chunk>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, orden, texto FROM chunks WHERE doc_id = ?1 ORDER BY orden ASC",
        )?;
        let rows = stmt.query_map(params![doc_id.0.to_string()], |r| {
            let id_s: String = r.get(0)?;
            let orden: i64 = r.get(1)?;
            let texto: String = r.get(2)?;
            Ok((id_s, orden, texto))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id_s, orden, texto) = r?;
            let ulid = Ulid::from_str(&id_s).with_context(|| format!("chunk_id inválido: {id_s}"))?;
            out.push(Chunk { id: ChunkId(ulid), doc_id, orden: orden as u32, texto });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iniy_core::DocId;
    use iniy_ingest::{Chunk, Documento};

    fn doc_demo() -> Documento {
        let doc_id = DocId::nuevo();
        Documento {
            id: doc_id,
            titulo: "demo".into(),
            chunks: vec![
                Chunk { id: ChunkId::nuevo(), doc_id, orden: 0, texto: "primer párrafo del corpus de prueba.".into() },
                Chunk { id: ChunkId::nuevo(), doc_id, orden: 1, texto: "segundo párrafo del corpus de prueba.".into() },
            ],
        }
    }

    #[test]
    fn store_en_memoria_migra_ok() {
        let _ = Store::en_memoria().unwrap();
    }

    #[test]
    fn persistir_y_listar_documento() {
        let mut s = Store::en_memoria().unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc).unwrap();
        let lista = s.listar_documentos().unwrap();
        assert_eq!(lista.len(), 1);
        assert_eq!(lista[0].titulo, "demo");
        assert_eq!(lista[0].n_chunks, 2);
        assert_eq!(lista[0].id, doc.id);
    }

    #[test]
    fn round_trip_chunks_preserva_orden_y_texto() {
        let mut s = Store::en_memoria().unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc).unwrap();
        let chunks = s.cargar_chunks(doc.id).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].orden, 0);
        assert_eq!(chunks[1].orden, 1);
        assert!(chunks[0].texto.starts_with("primer"));
        assert!(chunks[1].texto.starts_with("segundo"));
        assert_eq!(chunks[0].id, doc.chunks[0].id);
    }

    #[test]
    fn persistir_id_duplicado_falla() {
        let mut s = Store::en_memoria().unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc).unwrap();
        assert!(s.persistir_documento(&doc).is_err());
    }
}
