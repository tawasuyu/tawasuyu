//! iniy-store — persistencia SQLite del corpus, aserciones y matriz NLI.
//!
//! Esquema: documentos, chunks, aserciones, implicaciones. Las queries se
//! cablean a medida que cada subcomando las necesita.

use anyhow::{Context, Result};
use iniy_core::{Asercion, AsercionId, ChunkId, DocId, Implicacion, Opinion, RelacionNli};
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
            CREATE INDEX IF NOT EXISTS idx_aserciones_doc ON aserciones(doc_id);
            CREATE TABLE IF NOT EXISTS implicaciones (
                premisa     TEXT NOT NULL REFERENCES aserciones(id),
                hipotesis   TEXT NOT NULL REFERENCES aserciones(id),
                entailment      REAL NOT NULL,
                contradiction   REAL NOT NULL,
                neutral         REAL NOT NULL,
                PRIMARY KEY (premisa, hipotesis)
            );
            CREATE INDEX IF NOT EXISTS idx_imp_premisa ON implicaciones(premisa);
            CREATE INDEX IF NOT EXISTS idx_imp_hipotesis ON implicaciones(hipotesis);
            "#,
        )?;
        Ok(())
    }

    /// Persiste un documento recién ingerido (doc + chunks) en una sola transacción.
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

    /// Bulk-insert de aserciones en una sola transacción. `INSERT OR REPLACE`
    /// para que volver a correr `extract` sobre el mismo doc no duplique.
    pub fn persistir_aserciones(&mut self, aserciones: &[Asercion]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO aserciones (id, doc_id, chunk_id, texto, opinion_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for a in aserciones {
                let opinion_json = serde_json::to_string(&a.opinion_autoral)?;
                stmt.execute(params![
                    a.id.0.to_string(),
                    a.doc_id.0.to_string(),
                    a.chunk_id.0.to_string(),
                    a.texto,
                    opinion_json,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn cargar_aserciones(&self, doc_id: DocId) -> Result<Vec<Asercion>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chunk_id, texto, opinion_json FROM aserciones WHERE doc_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![doc_id.0.to_string()], |r| {
            let id_s: String = r.get(0)?;
            let chunk_s: String = r.get(1)?;
            let texto: String = r.get(2)?;
            let opinion_json: String = r.get(3)?;
            Ok((id_s, chunk_s, texto, opinion_json))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id_s, chunk_s, texto, opinion_json) = r?;
            let id = AsercionId(Ulid::from_str(&id_s).with_context(|| format!("asercion_id inválido: {id_s}"))?);
            let chunk_id = ChunkId(Ulid::from_str(&chunk_s).with_context(|| format!("chunk_id inválido: {chunk_s}"))?);
            let opinion_autoral: Opinion = serde_json::from_str(&opinion_json)
                .with_context(|| format!("opinion_json corrupta en asercion {id_s}"))?;
            out.push(Asercion { id, doc_id, chunk_id, texto, opinion_autoral });
        }
        Ok(out)
    }

    /// Bulk-insert/replace de implicaciones. Idempotente sobre (premisa, hipotesis).
    pub fn persistir_implicaciones(&mut self, imps: &[Implicacion]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO implicaciones (premisa, hipotesis, entailment, contradiction, neutral) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for i in imps {
                stmt.execute(params![
                    i.premisa.0.to_string(),
                    i.hipotesis.0.to_string(),
                    i.relacion.entailment as f64,
                    i.relacion.contradiction as f64,
                    i.relacion.neutral as f64,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Implicaciones cuyos dos extremos viven en `doc_id`.
    pub fn cargar_implicaciones_del_doc(&self, doc_id: DocId) -> Result<Vec<Implicacion>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT i.premisa, i.hipotesis, i.entailment, i.contradiction, i.neutral
            FROM implicaciones i
            JOIN aserciones ap ON ap.id = i.premisa
            JOIN aserciones ah ON ah.id = i.hipotesis
            WHERE ap.doc_id = ?1 AND ah.doc_id = ?1
            "#,
        )?;
        let rows = stmt.query_map(params![doc_id.0.to_string()], |r| {
            let p: String = r.get(0)?;
            let h: String = r.get(1)?;
            let e: f64 = r.get(2)?;
            let c: f64 = r.get(3)?;
            let n: f64 = r.get(4)?;
            Ok((p, h, e, c, n))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (p, h, e, c, n) = r?;
            let premisa = AsercionId(Ulid::from_str(&p).with_context(|| format!("premisa inválida: {p}"))?);
            let hipotesis = AsercionId(Ulid::from_str(&h).with_context(|| format!("hipotesis inválida: {h}"))?);
            out.push(Implicacion {
                premisa,
                hipotesis,
                relacion: RelacionNli {
                    entailment: e as f32,
                    contradiction: c as f32,
                    neutral: n as f32,
                },
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iniy_core::{AsercionId, DocId, Opinion};
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

    fn asercion_demo(doc_id: DocId, chunk_id: ChunkId, texto: &str) -> Asercion {
        Asercion {
            id: AsercionId::nuevo(),
            doc_id,
            chunk_id,
            texto: texto.into(),
            opinion_autoral: Opinion::nueva(0.6, 0.1, 0.3, 0.5).unwrap(),
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

    #[test]
    fn round_trip_aserciones_preserva_opinion() {
        let mut s = Store::en_memoria().unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc).unwrap();
        let a = asercion_demo(doc.id, doc.chunks[0].id, "aserción de prueba");
        s.persistir_aserciones(&[a.clone()]).unwrap();
        let cargadas = s.cargar_aserciones(doc.id).unwrap();
        assert_eq!(cargadas.len(), 1);
        assert_eq!(cargadas[0].id, a.id);
        assert_eq!(cargadas[0].texto, a.texto);
        assert!((cargadas[0].opinion_autoral.creencia - 0.6).abs() < 1e-5);
    }

    #[test]
    fn round_trip_implicaciones_filtra_por_doc() {
        let mut s = Store::en_memoria().unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc).unwrap();
        let a1 = asercion_demo(doc.id, doc.chunks[0].id, "primera");
        let a2 = asercion_demo(doc.id, doc.chunks[1].id, "segunda");
        s.persistir_aserciones(&[a1.clone(), a2.clone()]).unwrap();
        let imp = Implicacion {
            premisa: a1.id,
            hipotesis: a2.id,
            relacion: RelacionNli { entailment: 0.0, contradiction: 0.7, neutral: 0.3 },
        };
        s.persistir_implicaciones(&[imp]).unwrap();
        let imps = s.cargar_implicaciones_del_doc(doc.id).unwrap();
        assert_eq!(imps.len(), 1);
        assert!((imps[0].relacion.contradiction - 0.7).abs() < 1e-5);
    }

    #[test]
    fn aserciones_y_implicaciones_son_idempotentes() {
        let mut s = Store::en_memoria().unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc).unwrap();
        let a = asercion_demo(doc.id, doc.chunks[0].id, "x");
        s.persistir_aserciones(&[a.clone()]).unwrap();
        s.persistir_aserciones(&[a.clone()]).unwrap();
        assert_eq!(s.cargar_aserciones(doc.id).unwrap().len(), 1);
    }
}
