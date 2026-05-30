//! Aserciones, implicaciones, reputaciones y tags.

use super::*;

impl Store {
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

}
