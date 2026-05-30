//! Fuentes y documentos: alta, listados, chunks, stats.

use super::*;

impl Store {
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
            SELECT d.id, d.titulo, COUNT(c.id) AS n, d.creado,
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
                r.get::<_, String>(0)?, r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?, r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id_s, titulo, n, creado, f_id, f_nombre, f_kind) = r?;
            let ulid = Ulid::from_str(&id_s).with_context(|| format!("doc_id inválido: {id_s}"))?;
            let fuente = match (f_id, f_nombre) {
                (Some(fid_s), Some(nombre)) => {
                    let fid = Ulid::from_str(&fid_s).with_context(|| format!("fuente_id inválido: {fid_s}"))?;
                    Some(Fuente { id: FuenteId(fid), nombre, kind: f_kind })
                }
                _ => None,
            };
            out.push(DocumentoResumen {
                id: DocId(ulid), titulo, fuente, n_chunks: n as u32, creado_unix: creado,
            });
        }
        Ok(out)
    }

    /// Documentos ordenados cronológicamente (asc, los más antiguos primero).
    /// Cada uno con su n_aserciones + lista de tags. Filtros opcionales:
    /// `desde_unix` / `hasta_unix` (inclusivo); `tag` exacto.
    pub fn listar_cronologicamente(
        &self,
        desde_unix: Option<i64>,
        hasta_unix: Option<i64>,
        tag: Option<&str>,
    ) -> Result<Vec<DocumentoCronologico>> {
        let (sql, params_dyn): (&str, Vec<Box<dyn rusqlite::ToSql>>) = match (desde_unix, hasta_unix, tag) {
            (Some(d), Some(h), Some(t)) => (r#"
                SELECT d.id, d.titulo, d.creado, COUNT(a.id) AS n,
                       f.id, f.nombre, f.kind
                FROM documentos d
                LEFT JOIN aserciones a ON a.doc_id = d.id
                LEFT JOIN fuentes f ON f.id = d.fuente_id
                JOIN documento_tags dt ON dt.doc_id = d.id
                WHERE d.creado >= ?1 AND d.creado <= ?2 AND dt.tag = ?3
                GROUP BY d.id ORDER BY d.creado ASC, d.id ASC
            "#, vec![Box::new(d), Box::new(h), Box::new(t.to_string())]),
            (Some(d), Some(h), None) => (r#"
                SELECT d.id, d.titulo, d.creado, COUNT(a.id) AS n,
                       f.id, f.nombre, f.kind
                FROM documentos d
                LEFT JOIN aserciones a ON a.doc_id = d.id
                LEFT JOIN fuentes f ON f.id = d.fuente_id
                WHERE d.creado >= ?1 AND d.creado <= ?2
                GROUP BY d.id ORDER BY d.creado ASC, d.id ASC
            "#, vec![Box::new(d), Box::new(h)]),
            (None, None, Some(t)) => (r#"
                SELECT d.id, d.titulo, d.creado, COUNT(a.id) AS n,
                       f.id, f.nombre, f.kind
                FROM documentos d
                LEFT JOIN aserciones a ON a.doc_id = d.id
                LEFT JOIN fuentes f ON f.id = d.fuente_id
                JOIN documento_tags dt ON dt.doc_id = d.id
                WHERE dt.tag = ?1
                GROUP BY d.id ORDER BY d.creado ASC, d.id ASC
            "#, vec![Box::new(t.to_string())]),
            _ => (r#"
                SELECT d.id, d.titulo, d.creado, COUNT(a.id) AS n,
                       f.id, f.nombre, f.kind
                FROM documentos d
                LEFT JOIN aserciones a ON a.doc_id = d.id
                LEFT JOIN fuentes f ON f.id = d.fuente_id
                GROUP BY d.id ORDER BY d.creado ASC, d.id ASC
            "#, vec![]),
        };
        let mut stmt = self.conn.prepare(sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = params_dyn.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(params.as_slice(), |r| {
            Ok((
                r.get::<_, String>(0)?, r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?, r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id_s, titulo, creado, n, f_id, f_nombre, f_kind) = r?;
            let did = DocId(Ulid::from_str(&id_s).with_context(|| format!("doc_id inválido: {id_s}"))?);
            let fuente = match (f_id, f_nombre) {
                (Some(fid_s), Some(nombre)) => {
                    let fid = Ulid::from_str(&fid_s).with_context(|| format!("fuente_id inválido: {fid_s}"))?;
                    Some(Fuente { id: FuenteId(fid), nombre, kind: f_kind })
                }
                _ => None,
            };
            let tags = self.tags_de_doc(did)?;
            out.push(DocumentoCronologico {
                id: did, titulo, fuente,
                n_aserciones: n as u32,
                creado_unix: creado, tags,
            });
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

    /// Overview cuantitativo del corpus. Devuelve conteos por tabla,
    /// distribución NLI por clase dominante, y el rango temporal.
    pub fn stats(&self) -> Result<CorpusStats> {
        fn count(c: &Connection, t: &str) -> Result<u64> {
            Ok(c.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get::<_, i64>(0))? as u64)
        }
        let c = &self.conn;
        let n_fuentes = count(c, "fuentes")?;
        let n_documentos = count(c, "documentos")?;
        let n_chunks = count(c, "chunks")?;
        let n_aserciones = count(c, "aserciones")?;
        let n_implicaciones = count(c, "implicaciones")?;
        let n_tags = count(c, "tags")?;
        let n_documento_tags = count(c, "documento_tags")?;
        // Distribución NLI por clase dominante.
        let mut nli_entail = 0u64;
        let mut nli_contra = 0u64;
        let mut nli_neutral = 0u64;
        let mut stmt = c.prepare(
            "SELECT entailment, contradiction, neutral FROM implicaciones"
        )?;
        let rows = stmt.query_map([], |r| Ok((
            r.get::<_, f64>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?,
        )))?;
        for r in rows {
            let (e, k, n) = r?;
            if e >= k && e >= n { nli_entail += 1; }
            else if k >= n { nli_contra += 1; }
            else { nli_neutral += 1; }
        }
        // Rango temporal de docs.
        let temporal: Option<(i64, i64)> = c.query_row(
            "SELECT MIN(creado), MAX(creado) FROM documentos",
            [],
            |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
        ).optional()?
        .and_then(|(min, max)| Some((min?, max?)));
        Ok(CorpusStats {
            n_fuentes, n_documentos, n_chunks, n_aserciones, n_implicaciones,
            n_tags, n_documento_tags,
            nli_entail, nli_contra, nli_neutral,
            primero_unix: temporal.map(|(min, _)| min),
            ultimo_unix: temporal.map(|(_, max)| max),
        })
    }

    /// Lista los docs que aún no tienen aserciones extraídas. Pensado para
    /// `iniy extract-all` tras un import masivo (Wikipedia, OCR de mil PDFs).
    pub fn documentos_sin_aserciones(&self) -> Result<Vec<DocId>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT d.id FROM documentos d
            LEFT JOIN aserciones a ON a.doc_id = d.id
            WHERE a.id IS NULL
            ORDER BY d.creado ASC, d.id ASC
            "#,
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            let s = r?;
            out.push(DocId(Ulid::from_str(&s).with_context(|| format!("doc_id inválido: {s}"))?));
        }
        Ok(out)
    }

}
