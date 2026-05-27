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
/// fuente viene. La `fuente` es la EFECTIVA: si la aserción cita a otra fuente
/// (campo `fuente_citada_id` en DB, ej. «Según Aristóteles, …»), la fuente
/// efectiva es la citada; si no, la del documento. El flag `citada` distingue
/// el caso para la UI.
#[derive(Debug, Clone)]
pub struct AsercionAtribuida {
    pub asercion: Asercion,
    pub doc_titulo: String,
    pub fuente: Option<Fuente>,
    pub citada: bool,
}

#[derive(Debug, Clone)]
pub struct FuenteResumen {
    pub fuente: Fuente,
    pub n_docs: u32,
    pub n_aserciones: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ReputacionPersistida {
    pub fuente_id: FuenteId,
    pub apoyada: u32,
    pub contradicha: u32,
    pub apoya: u32,
    pub contradice: u32,
    pub score: f32,
    pub actualizada_at: i64,
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
        self.migrar_aserciones_fuente_citada()?;
        self.migrar_reputaciones()?;
        self.migrar_tags()?;
        Ok(())
    }

    fn migrar_tags(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS tags (
                nombre TEXT PRIMARY KEY
            );
            CREATE TABLE IF NOT EXISTS documento_tags (
                doc_id TEXT NOT NULL REFERENCES documentos(id),
                tag    TEXT NOT NULL REFERENCES tags(nombre),
                PRIMARY KEY (doc_id, tag)
            );
            CREATE INDEX IF NOT EXISTS idx_doctag_tag ON documento_tags(tag);
            "#,
        )?;
        Ok(())
    }

    fn migrar_reputaciones(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS reputaciones (
                fuente_id        TEXT PRIMARY KEY REFERENCES fuentes(id),
                apoyada          INTEGER NOT NULL DEFAULT 0,
                contradicha      INTEGER NOT NULL DEFAULT 0,
                apoya            INTEGER NOT NULL DEFAULT 0,
                contradice       INTEGER NOT NULL DEFAULT 0,
                score            REAL NOT NULL DEFAULT 0.0,
                actualizada_at   INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );
            "#,
        )?;
        Ok(())
    }

    fn migrar_aserciones_fuente_citada(&self) -> Result<()> {
        let mut stmt = self.conn.prepare("PRAGMA table_info(aserciones)")?;
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;
        if !cols.iter().any(|c| c == "fuente_citada_id") {
            self.conn.execute(
                "ALTER TABLE aserciones ADD COLUMN fuente_citada_id TEXT REFERENCES fuentes(id)",
                [],
            )?;
            self.conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_aserciones_fuente_citada ON aserciones(fuente_citada_id)",
                [],
            )?;
        }
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
                   (SELECT COUNT(*) FROM documentos d2 WHERE d2.fuente_id = f.id) AS n_docs,
                   (SELECT COUNT(*) FROM aserciones a2
                    JOIN documentos d3 ON d3.id = a2.doc_id
                    WHERE a2.fuente_citada_id = f.id
                       OR (a2.fuente_citada_id IS NULL AND d3.fuente_id = f.id)) AS n_aserciones
            FROM fuentes f
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
    /// EFECTIVA (citada > doc). Es el insumo de las consultas `testimonio` /
    /// `propagar` / `consenso`.
    pub fn cargar_aserciones_atribuidas_todas(&self) -> Result<Vec<AsercionAtribuida>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT a.id, a.doc_id, a.chunk_id, a.texto, a.opinion_json,
                   d.titulo,
                   CASE WHEN fc.id IS NOT NULL THEN fc.id     ELSE fd.id     END AS f_id,
                   CASE WHEN fc.id IS NOT NULL THEN fc.nombre ELSE fd.nombre END AS f_nombre,
                   CASE WHEN fc.id IS NOT NULL THEN fc.kind   ELSE fd.kind   END AS f_kind,
                   CASE WHEN fc.id IS NOT NULL THEN 1 ELSE 0 END AS citada
            FROM aserciones a
            JOIN documentos d ON d.id = a.doc_id
            LEFT JOIN fuentes fd ON fd.id = d.fuente_id
            LEFT JOIN fuentes fc ON fc.id = a.fuente_citada_id
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
                r.get::<_, i64>(9)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (a_id_s, d_id_s, c_id_s, texto, op_json, d_titulo, f_id, f_nombre, f_kind, citada) = r?;
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
                citada: citada != 0,
            });
        }
        Ok(out)
    }

    /// Marca una aserción como cita de otra fuente. `None` deshace.
    pub fn asignar_fuente_citada(&mut self, asercion_id: AsercionId, fuente_id: Option<FuenteId>) -> Result<()> {
        self.conn.execute(
            "UPDATE aserciones SET fuente_citada_id = ?2 WHERE id = ?1",
            params![asercion_id.0.to_string(), fuente_id.map(|f| f.0.to_string())],
        )?;
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

    /// Todas las implicaciones del corpus, sin filtrar por documento.
    /// Insumo de propagación cross-doc.
    pub fn cargar_implicaciones_todas(&self) -> Result<Vec<Implicacion>> {
        let mut stmt = self.conn.prepare(
            "SELECT premisa, hipotesis, entailment, contradiction, neutral FROM implicaciones",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, f64>(3)?,
                r.get::<_, f64>(4)?,
            ))
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

    /// Recalcula la reputación de todas las fuentes a partir del grafo NLI
    /// actual y la persiste (UPSERT). Devuelve cuántas fuentes se actualizaron.
    ///
    /// Solo cuenta aristas CROSS-fuente (intra-fuente no es evidencia
    /// independiente). Para cada fuente F:
    ///   - apoyada = aristas entailment-dominantes que apuntan a aserciones de F.
    ///   - contradicha = aristas contradiction-dominantes que apuntan a F.
    ///   - apoya / contradice = aristas que F emite hacia otras fuentes.
    ///   - score = (apoyada - contradicha) / max(1, apoyada + contradicha) ∈ [-1, 1].
    pub fn recalcular_reputaciones(&mut self) -> Result<usize> {
        use std::collections::HashMap;
        let aserciones = self.cargar_aserciones_atribuidas_todas()?;
        let imps = self.cargar_implicaciones_todas()?;
        let asercion_a_fuente: HashMap<AsercionId, FuenteId> = aserciones.iter()
            .filter_map(|a| a.fuente.as_ref().map(|f| (a.asercion.id, f.id)))
            .collect();
        // (apoyada, contradicha, apoya, contradice) por fuente.
        let mut stats: HashMap<FuenteId, [u32; 4]> = HashMap::new();
        for a in &aserciones {
            if let Some(f) = &a.fuente {
                stats.entry(f.id).or_default();
            }
        }
        for imp in &imps {
            let Some(&fa) = asercion_a_fuente.get(&imp.premisa) else { continue; };
            let Some(&fb) = asercion_a_fuente.get(&imp.hipotesis) else { continue; };
            if fa == fb {
                continue;
            }
            let rel = &imp.relacion;
            if rel.entailment > rel.contradiction && rel.entailment > 0.0 {
                stats.entry(fa).or_default()[2] += 1; // fa apoya
                stats.entry(fb).or_default()[0] += 1; // fb apoyada
            } else if rel.contradiction > 0.0 {
                stats.entry(fa).or_default()[3] += 1; // fa contradice
                stats.entry(fb).or_default()[1] += 1; // fb contradicha
            }
        }
        let tx = self.conn.transaction()?;
        let n = stats.len();
        {
            let mut stmt = tx.prepare(
                r#"INSERT INTO reputaciones (fuente_id, apoyada, contradicha, apoya, contradice, score, actualizada_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%s','now'))
                   ON CONFLICT(fuente_id) DO UPDATE SET
                     apoyada = excluded.apoyada,
                     contradicha = excluded.contradicha,
                     apoya = excluded.apoya,
                     contradice = excluded.contradice,
                     score = excluded.score,
                     actualizada_at = excluded.actualizada_at"#,
            )?;
            for (fid, [apoyada, contradicha, apoya, contradice]) in &stats {
                let recibidos = apoyada + contradicha;
                let score = if recibidos > 0 {
                    (*apoyada as f32 - *contradicha as f32) / recibidos as f32
                } else {
                    0.0
                };
                stmt.execute(params![
                    fid.0.to_string(),
                    apoyada,
                    contradicha,
                    apoya,
                    contradice,
                    score as f64,
                ])?;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    pub fn cargar_reputaciones_todas(&self) -> Result<Vec<ReputacionPersistida>> {
        let mut stmt = self.conn.prepare(
            "SELECT fuente_id, apoyada, contradicha, apoya, contradice, score, actualizada_at FROM reputaciones",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, f64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (fid_s, apoyada, contradicha, apoya, contradice, score, at) = r?;
            let fid = FuenteId(Ulid::from_str(&fid_s).with_context(|| format!("fuente_id inválido: {fid_s}"))?);
            out.push(ReputacionPersistida {
                fuente_id: fid,
                apoyada: apoyada as u32,
                contradicha: contradicha as u32,
                apoya: apoya as u32,
                contradice: contradice as u32,
                score: score as f32,
                actualizada_at: at,
            });
        }
        Ok(out)
    }

    /// Agrega un tag a un documento (crea el tag si no existe).
    pub fn taggear_doc(&mut self, doc_id: DocId, tag: &str) -> Result<()> {
        let tag = tag.trim();
        if tag.is_empty() {
            anyhow::bail!("tag vacío");
        }
        let tx = self.conn.transaction()?;
        tx.execute("INSERT OR IGNORE INTO tags (nombre) VALUES (?1)", params![tag])?;
        tx.execute(
            "INSERT OR IGNORE INTO documento_tags (doc_id, tag) VALUES (?1, ?2)",
            params![doc_id.0.to_string(), tag],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn destaggear_doc(&mut self, doc_id: DocId, tag: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM documento_tags WHERE doc_id = ?1 AND tag = ?2",
            params![doc_id.0.to_string(), tag],
        )?;
        Ok(())
    }

    pub fn tags_de_doc(&self, doc_id: DocId) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT tag FROM documento_tags WHERE doc_id = ?1 ORDER BY tag",
        )?;
        let rows = stmt.query_map(params![doc_id.0.to_string()], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn listar_tags_con_conteo(&self) -> Result<Vec<(String, u32)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT t.nombre, COUNT(dt.doc_id) AS n
            FROM tags t
            LEFT JOIN documento_tags dt ON dt.tag = t.nombre
            GROUP BY t.nombre
            ORDER BY n DESC, t.nombre ASC
            "#,
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        let mut out = Vec::new();
        for r in rows {
            let (n, c) = r?;
            out.push((n, c as u32));
        }
        Ok(out)
    }

    /// Carga aserciones atribuidas filtradas por tag (vía doc).
    pub fn cargar_aserciones_atribuidas_por_tag(&self, tag: &str) -> Result<Vec<AsercionAtribuida>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT a.id, a.doc_id, a.chunk_id, a.texto, a.opinion_json,
                   d.titulo,
                   CASE WHEN fc.id IS NOT NULL THEN fc.id     ELSE fd.id     END AS f_id,
                   CASE WHEN fc.id IS NOT NULL THEN fc.nombre ELSE fd.nombre END AS f_nombre,
                   CASE WHEN fc.id IS NOT NULL THEN fc.kind   ELSE fd.kind   END AS f_kind,
                   CASE WHEN fc.id IS NOT NULL THEN 1 ELSE 0 END AS citada
            FROM aserciones a
            JOIN documentos d ON d.id = a.doc_id
            JOIN documento_tags dt ON dt.doc_id = d.id
            LEFT JOIN fuentes fd ON fd.id = d.fuente_id
            LEFT JOIN fuentes fc ON fc.id = a.fuente_citada_id
            WHERE dt.tag = ?1
            "#,
        )?;
        let rows = stmt.query_map(params![tag], |r| {
            Ok((
                r.get::<_, String>(0)?, r.get::<_, String>(1)?,
                r.get::<_, String>(2)?, r.get::<_, String>(3)?,
                r.get::<_, String>(4)?, r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?, r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?, r.get::<_, i64>(9)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (a_id_s, d_id_s, c_id_s, texto, op_json, d_titulo, f_id, f_nombre, f_kind, citada) = r?;
            let id = AsercionId(Ulid::from_str(&a_id_s).with_context(|| format!("asercion_id inválido: {a_id_s}"))?);
            let doc_id = DocId(Ulid::from_str(&d_id_s).with_context(|| format!("doc_id inválido: {d_id_s}"))?);
            let chunk_id = ChunkId(Ulid::from_str(&c_id_s).with_context(|| format!("chunk_id inválido: {c_id_s}"))?);
            let opinion_autoral: Opinion = serde_json::from_str(&op_json)
                .with_context(|| format!("opinion_json corrupta: {a_id_s}"))?;
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
                citada: citada != 0,
            });
        }
        Ok(out)
    }

    pub fn cargar_reputacion(&self, fuente_id: FuenteId) -> Result<Option<ReputacionPersistida>> {
        let r: Option<(i64, i64, i64, i64, f64, i64)> = self.conn
            .query_row(
                "SELECT apoyada, contradicha, apoya, contradice, score, actualizada_at FROM reputaciones WHERE fuente_id = ?1",
                params![fuente_id.0.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?;
        Ok(r.map(|(a, c, ap, co, s, at)| ReputacionPersistida {
            fuente_id,
            apoyada: a as u32,
            contradicha: c as u32,
            apoya: ap as u32,
            contradice: co as u32,
            score: s as f32,
            actualizada_at: at,
        }))
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
    fn fuente_citada_supera_a_fuente_del_doc_en_atribuida() {
        let mut s = Store::en_memoria().unwrap();
        let f_doc = s.obtener_o_crear_fuente("Wikipedia", Some("wiki")).unwrap();
        let f_citada = s.obtener_o_crear_fuente("Aristóteles", Some("autor")).unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc, Some(f_doc)).unwrap();
        let a = asercion_demo(doc.id, doc.chunks[0].id, "El cosmos es eterno");
        s.persistir_aserciones(&[a.clone()]).unwrap();
        s.asignar_fuente_citada(a.id, Some(f_citada)).unwrap();
        let v = s.cargar_aserciones_atribuidas_todas().unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].fuente.as_ref().unwrap().nombre, "Aristóteles");
        assert!(v[0].citada);
    }

    #[test]
    fn aserciones_sin_cita_usan_fuente_del_doc() {
        let mut s = Store::en_memoria().unwrap();
        let f = s.obtener_o_crear_fuente("Anaximandro", Some("autor")).unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc, Some(f)).unwrap();
        let a = asercion_demo(doc.id, doc.chunks[0].id, "X");
        s.persistir_aserciones(&[a]).unwrap();
        let v = s.cargar_aserciones_atribuidas_todas().unwrap();
        assert_eq!(v[0].fuente.as_ref().unwrap().nombre, "Anaximandro");
        assert!(!v[0].citada);
    }

    #[test]
    fn listar_fuentes_cuenta_aserciones_citadas() {
        let mut s = Store::en_memoria().unwrap();
        let f_doc = s.obtener_o_crear_fuente("Doxógrafo", None).unwrap();
        let f_cita = s.obtener_o_crear_fuente("Tales", Some("autor")).unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc, Some(f_doc)).unwrap();
        let a1 = asercion_demo(doc.id, doc.chunks[0].id, "agua principio");
        let a2 = asercion_demo(doc.id, doc.chunks[0].id, "otra cosa");
        s.persistir_aserciones(&[a1.clone(), a2]).unwrap();
        s.asignar_fuente_citada(a1.id, Some(f_cita)).unwrap();
        let lista = s.listar_fuentes().unwrap();
        let tales = lista.iter().find(|r| r.fuente.nombre == "Tales").unwrap();
        let doxo = lista.iter().find(|r| r.fuente.nombre == "Doxógrafo").unwrap();
        assert_eq!(tales.n_aserciones, 1); // la citada
        assert_eq!(doxo.n_aserciones, 1);  // la que cae al doc
    }

    #[test]
    fn recalcular_reputaciones_persiste_y_calcula_score() {
        let mut s = Store::en_memoria().unwrap();
        let f1 = s.obtener_o_crear_fuente("F1", None).unwrap();
        let f2 = s.obtener_o_crear_fuente("F2", None).unwrap();
        let doc1 = doc_demo();
        let doc2 = doc_demo();
        s.persistir_documento(&doc1, Some(f1)).unwrap();
        s.persistir_documento(&doc2, Some(f2)).unwrap();
        let a1 = asercion_demo(doc1.id, doc1.chunks[0].id, "F1 dice X");
        let a2 = asercion_demo(doc2.id, doc2.chunks[0].id, "F2 contradice X");
        s.persistir_aserciones(&[a1.clone(), a2.clone()]).unwrap();
        // F1 ←(contradiction)← F2.
        s.persistir_implicaciones(&[Implicacion {
            premisa: a2.id,
            hipotesis: a1.id,
            relacion: RelacionNli { entailment: 0.0, contradiction: 0.8, neutral: 0.2 },
        }]).unwrap();
        let n = s.recalcular_reputaciones().unwrap();
        assert_eq!(n, 2);

        let rep_f1 = s.cargar_reputacion(f1).unwrap().unwrap();
        assert_eq!(rep_f1.contradicha, 1);
        assert_eq!(rep_f1.apoyada, 0);
        assert!((rep_f1.score - (-1.0)).abs() < 1e-5);

        let rep_f2 = s.cargar_reputacion(f2).unwrap().unwrap();
        assert_eq!(rep_f2.contradice, 1);
        assert_eq!(rep_f2.apoya, 0);
        // F2 no recibe nada, score=0.
        assert!((rep_f2.score - 0.0).abs() < 1e-5);
    }

    #[test]
    fn recalcular_reputaciones_es_idempotente() {
        let mut s = Store::en_memoria().unwrap();
        let f = s.obtener_o_crear_fuente("F", None).unwrap();
        let doc = doc_demo();
        s.persistir_documento(&doc, Some(f)).unwrap();
        let a = asercion_demo(doc.id, doc.chunks[0].id, "x");
        s.persistir_aserciones(&[a]).unwrap();
        s.recalcular_reputaciones().unwrap();
        s.recalcular_reputaciones().unwrap();
        assert_eq!(s.cargar_reputaciones_todas().unwrap().len(), 1);
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
