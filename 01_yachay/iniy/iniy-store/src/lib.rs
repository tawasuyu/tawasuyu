//! iniy-store — persistencia SQLite del corpus, aserciones y matriz NLI.
//!
//! Esquema: documentos, chunks, aserciones, implicaciones. Las queries se
//! cablean a medida que cada subcomando las necesita.

use anyhow::{Context, Result};
use iniy_core::{Asercion, AsercionId, ChunkId, DocId, Fuente, FuenteId, Implicacion, Opinion, RelacionNli};
use iniy_ingest::{Chunk, Documento};
use rusqlite::{params, Connection, OptionalExtension};
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
    pub fuente: Option<Fuente>,
    pub n_chunks: u32,
}

/// Una aserción con su contexto atribucional: en qué doc apareció y de qué
/// fuente viene. Es el átomo de la consulta `testimonio`: "quién dice qué".
#[derive(Debug, Clone)]
pub struct AsercionAtribuida {
    pub asercion: Asercion,
    pub doc_titulo: String,
    pub fuente: Option<Fuente>,
}

#[derive(Debug, Clone)]
pub struct FuenteResumen {
    pub fuente: Fuente,
    pub n_docs: u32,
    pub n_aserciones: u32,
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
            CREATE TABLE IF NOT EXISTS fuentes (
                id      TEXT PRIMARY KEY,
                nombre  TEXT NOT NULL UNIQUE,
                kind    TEXT,
                creado  INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );
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
        self.migrar_documentos_fuente_id()?;
        Ok(())
    }

    /// SQLite no admite `ADD COLUMN IF NOT EXISTS`. Detectamos por
    /// `PRAGMA table_info` y agregamos `documentos.fuente_id` solo si falta.
    /// Idempotente sobre DBs nuevas y sobre DBs viejas (pre-fuentes).
    fn migrar_documentos_fuente_id(&self) -> Result<()> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(documentos)")?;
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;
        if !cols.iter().any(|c| c == "fuente_id") {
            self.conn.execute(
                "ALTER TABLE documentos ADD COLUMN fuente_id TEXT REFERENCES fuentes(id)",
                [],
            )?;
            self.conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_documentos_fuente ON documentos(fuente_id)",
                [],
            )?;
        }
        Ok(())
    }

    /// Crea una fuente si no existe (por nombre) y devuelve su id. Si ya existe
    /// y `kind` viene Some pero la existente tiene None, actualiza el kind.
    pub fn obtener_o_crear_fuente(&mut self, nombre: &str, kind: Option<&str>) -> Result<FuenteId> {
        let existente: Option<(String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT id, kind FROM fuentes WHERE nombre = ?1",
                params![nombre],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        if let Some((id_s, kind_existente)) = existente {
            let id = Ulid::from_str(&id_s).with_context(|| format!("fuente_id inválido: {id_s}"))?;
            if kind_existente.is_none() && kind.is_some() {
                self.conn.execute(
                    "UPDATE fuentes SET kind = ?1 WHERE id = ?2",
                    params![kind, id_s],
                )?;
            }
            return Ok(FuenteId(id));
        }
        let id = FuenteId::nuevo();
        self.conn.execute(
            "INSERT INTO fuentes (id, nombre, kind) VALUES (?1, ?2, ?3)",
            params![id.0.to_string(), nombre, kind],
        )?;
        Ok(id)
    }

    pub fn cargar_fuente(&self, id: FuenteId) -> Result<Option<Fuente>> {
        let r = self
            .conn
            .query_row(
                "SELECT nombre, kind FROM fuentes WHERE id = ?1",
                params![id.0.to_string()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        Ok(r.map(|(nombre, kind)| Fuente { id, nombre, kind }))
    }

    pub fn listar_fuentes(&self) -> Result<Vec<FuenteResumen>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT f.id, f.nombre, f.kind,
                   COUNT(DISTINCT d.id) AS n_docs,
                   COUNT(DISTINCT a.id) AS n_aserciones
            FROM fuentes f
            LEFT JOIN documentos d ON d.fuente_id = f.id
            LEFT JOIN aserciones a ON a.doc_id = d.id
            GROUP BY f.id
            ORDER BY f.nombre ASC
            "#,
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id_s, nombre, kind, nd, na) = r?;
            let id = FuenteId(Ulid::from_str(&id_s).with_context(|| format!("fuente_id inválido: {id_s}"))?);
            out.push(FuenteResumen {
                fuente: Fuente { id, nombre, kind },
                n_docs: nd as u32,
                n_aserciones: na as u32,
            });
        }
        Ok(out)
    }

    /// Persiste un documento recién ingerido (doc + chunks) en una sola transacción,
    /// opcionalmente atribuyéndolo a una fuente.
    pub fn persistir_documento(&mut self, doc: &Documento, fuente_id: Option<FuenteId>) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO documentos (id, titulo, fuente_id) VALUES (?1, ?2, ?3)",
            params![doc.id.0.to_string(), doc.titulo, fuente_id.map(|f| f.0.to_string())],
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

    /// Reatribuye un documento ya persistido a una fuente (o la quita si `None`).
    pub fn asignar_fuente_a_doc(&mut self, doc_id: DocId, fuente_id: Option<FuenteId>) -> Result<()> {
        self.conn.execute(
            "UPDATE documentos SET fuente_id = ?2 WHERE id = ?1",
            params![doc_id.0.to_string(), fuente_id.map(|f| f.0.to_string())],
        )?;
        Ok(())
    }

    pub fn listar_documentos(&self) -> Result<Vec<DocumentoResumen>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT d.id, d.titulo, COUNT(c.id) AS n,
                   f.id AS f_id, f.nombre AS f_nombre, f.kind AS f_kind
            FROM documentos d
            LEFT JOIN chunks c ON c.doc_id = d.id
            LEFT JOIN fuentes f ON f.id = d.fuente_id
            GROUP BY d.id
            ORDER BY d.creado DESC, d.id DESC
            "#,
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id_s, titulo, n, f_id, f_nombre, f_kind) = r?;
            let ulid = Ulid::from_str(&id_s).with_context(|| format!("doc_id inválido: {id_s}"))?;
            let fuente = match (f_id, f_nombre) {
                (Some(fid_s), Some(nombre)) => {
                    let fid = Ulid::from_str(&fid_s).with_context(|| format!("fuente_id inválido: {fid_s}"))?;
                    Some(Fuente { id: FuenteId(fid), nombre, kind: f_kind })
                }
                _ => None,
            };
            out.push(DocumentoResumen { id: DocId(ulid), titulo, fuente, n_chunks: n as u32 });
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

    /// Todas las aserciones del corpus, cada una con su doc.titulo y fuente
    /// (resuelta). Es el insumo de la consulta `testimonio`, que itera todo
    /// y filtra por relación léxica contra el texto query.
    pub fn cargar_aserciones_atribuidas_todas(&self) -> Result<Vec<AsercionAtribuida>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT a.id, a.doc_id, a.chunk_id, a.texto, a.opinion_json,
                   d.titulo,
                   f.id AS f_id, f.nombre AS f_nombre, f.kind AS f_kind
            FROM aserciones a
            JOIN documentos d ON d.id = a.doc_id
            LEFT JOIN fuentes f ON f.id = d.fuente_id
            "#,
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (a_id_s, d_id_s, c_id_s, texto, op_json, d_titulo, f_id, f_nombre, f_kind) = r?;
            let id = AsercionId(Ulid::from_str(&a_id_s).with_context(|| format!("asercion_id inválido: {a_id_s}"))?);
            let doc_id = DocId(Ulid::from_str(&d_id_s).with_context(|| format!("doc_id inválido: {d_id_s}"))?);
            let chunk_id = ChunkId(Ulid::from_str(&c_id_s).with_context(|| format!("chunk_id inválido: {c_id_s}"))?);
            let opinion_autoral: Opinion = serde_json::from_str(&op_json)
                .with_context(|| format!("opinion_json corrupta en asercion {a_id_s}"))?;
            let fuente = match (f_id, f_nombre) {
                (Some(fid_s), Some(nombre)) => {
                    let fid = Ulid::from_str(&fid_s).with_context(|| format!("fuente_id inválido: {fid_s}"))?;
                    Some(Fuente { id: FuenteId(fid), nombre, kind: f_kind })
                }
                _ => None,
            };
            out.push(AsercionAtribuida {
                asercion: Asercion { id, doc_id, chunk_id, texto, opinion_autoral },
                doc_titulo: d_titulo,
                fuente,
            });
        }
        Ok(out)
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
        s.persistir_documento(&doc, None).unwrap();
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
        s.persistir_documento(&doc, None).unwrap();
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
        s.persistir_documento(&doc, None).unwrap();
        assert!(s.persistir_documento(&doc, None).is_err());
    }

    #[test]
    fn round_trip_aserciones_preserva_opinion() {
        let mut s = Store::en_memoria().unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc, None).unwrap();
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
        s.persistir_documento(&doc, None).unwrap();
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
        s.persistir_documento(&doc, None).unwrap();
        let a = asercion_demo(doc.id, doc.chunks[0].id, "x");
        s.persistir_aserciones(&[a.clone()]).unwrap();
        s.persistir_aserciones(&[a.clone()]).unwrap();
        assert_eq!(s.cargar_aserciones(doc.id).unwrap().len(), 1);
    }

    #[test]
    fn obtener_o_crear_fuente_es_idempotente_por_nombre() {
        let mut s = Store::en_memoria().unwrap();
        let f1 = s.obtener_o_crear_fuente("Aristóteles", Some("autor")).unwrap();
        let f2 = s.obtener_o_crear_fuente("Aristóteles", None).unwrap();
        assert_eq!(f1, f2);
        assert_eq!(s.listar_fuentes().unwrap().len(), 1);
    }

    #[test]
    fn obtener_o_crear_fuente_actualiza_kind_si_estaba_vacio() {
        let mut s = Store::en_memoria().unwrap();
        s.obtener_o_crear_fuente("Voltaire", None).unwrap();
        s.obtener_o_crear_fuente("Voltaire", Some("autor")).unwrap();
        let lista = s.listar_fuentes().unwrap();
        assert_eq!(lista[0].fuente.kind.as_deref(), Some("autor"));
    }

    #[test]
    fn documento_atribuido_resuelve_fuente_al_listar() {
        let mut s = Store::en_memoria().unwrap();
        let f = s.obtener_o_crear_fuente("Heráclito", Some("autor")).unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc, Some(f)).unwrap();
        let docs = s.listar_documentos().unwrap();
        assert_eq!(docs[0].fuente.as_ref().unwrap().nombre, "Heráclito");
    }

    #[test]
    fn listar_fuentes_cuenta_docs_y_aserciones() {
        let mut s = Store::en_memoria().unwrap();
        let f = s.obtener_o_crear_fuente("F1", None).unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc, Some(f)).unwrap();
        let a1 = asercion_demo(doc.id, doc.chunks[0].id, "uno");
        let a2 = asercion_demo(doc.id, doc.chunks[0].id, "dos");
        s.persistir_aserciones(&[a1, a2]).unwrap();
        let lista = s.listar_fuentes().unwrap();
        assert_eq!(lista[0].n_docs, 1);
        assert_eq!(lista[0].n_aserciones, 2);
    }

    #[test]
    fn aserciones_atribuidas_trae_fuente_y_titulo() {
        let mut s = Store::en_memoria().unwrap();
        let f = s.obtener_o_crear_fuente("Plotino", Some("autor")).unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc, Some(f)).unwrap();
        let a = asercion_demo(doc.id, doc.chunks[0].id, "el uno trasciende al ser");
        s.persistir_aserciones(&[a]).unwrap();
        let v = s.cargar_aserciones_atribuidas_todas().unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].fuente.as_ref().unwrap().nombre, "Plotino");
        assert_eq!(v[0].doc_titulo, "demo");
    }

    #[test]
    fn migracion_anade_fuente_id_a_documentos_viejos() {
        // DB que existía antes del modelo de fuentes: la primer migración (sin
        // fuentes) corre, y luego una segunda re-migración añade la columna.
        // El esquema final tiene que tener `documentos.fuente_id`.
        let s = Store::en_memoria().unwrap();
        // Forzamos otra migración para verificar idempotencia.
        s.migrar().unwrap();
        let mut stmt = s.conn.prepare("PRAGMA table_info(documentos)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(cols.iter().any(|c| c == "fuente_id"));
    }
}
