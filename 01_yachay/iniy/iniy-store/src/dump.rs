//! Export/import (SQLite y dump nativo) + carga puntual.

use super::*;

impl Store {
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

    /// Exporta la DB completa a un archivo SQLite usando `VACUUM INTO`.
    /// El archivo destino debe NO existir; queda como una DB independiente
    /// y compacta (VACUUM elimina espacio libre).
    pub fn exportar_sqlite(&self, destino: &Path) -> Result<()> {
        if destino.exists() {
            anyhow::bail!("destino ya existe: {}", destino.display());
        }
        // VACUUM INTO requiere literal de string SQL, no parámetro. Validamos
        // la ruta para evitar inyección (sin comillas simples).
        let s = destino.to_string_lossy();
        if s.contains('\'') {
            anyhow::bail!("ruta con comillas simples no soportada");
        }
        self.conn.execute_batch(&format!("VACUUM INTO '{}'", s))?;
        Ok(())
    }

    /// Importa otra DB SQLite de iniy mergeando vía ATTACH + INSERT OR IGNORE.
    /// La DB actual es destino; `origen` se ataca con alias `src` y sus
    /// tablas se copian. Reputaciones se recalculan al final.
    pub fn importar_sqlite(&mut self, origen: &Path) -> Result<ImportStats> {
        let s = origen.to_string_lossy();
        if s.contains('\'') {
            anyhow::bail!("ruta con comillas simples no soportada");
        }
        // ATTACH es una statement aparte; no usa parámetros bind para la ruta.
        self.conn.execute_batch(&format!("ATTACH DATABASE '{}' AS src", s))?;
        let mut stats = ImportStats::default();
        let tx = self.conn.transaction()?;
        // Por cada tabla, hacemos INSERT OR IGNORE y contamos las filas
        // afectadas con `changes()`.
        let pares: [(&str, &str, &mut usize, &mut usize); 6] = [
            ("fuentes", "id, nombre, kind", &mut stats.fuentes, &mut stats.fuentes_omitidas),
            ("documentos", "id, titulo, fuente_id", &mut stats.documentos, &mut stats.documentos_omitidos),
            ("chunks", "id, doc_id, orden, texto", &mut stats.chunks, &mut stats.chunks_omitidos),
            ("aserciones", "id, doc_id, chunk_id, texto, opinion_json, fuente_citada_id", &mut stats.aserciones, &mut stats.aserciones_omitidas),
            ("implicaciones", "premisa, hipotesis, entailment, contradiction, neutral", &mut stats.implicaciones, &mut stats.implicaciones_omitidas),
            ("documento_tags", "doc_id, tag", &mut stats.tags, &mut stats.tags_omitidos),
        ];
        for (tabla, cols, nuevos, omitidos) in pares {
            let total: i64 = tx.query_row(
                &format!("SELECT COUNT(*) FROM src.{tabla}"),
                [], |r| r.get(0))?;
            let antes: i64 = tx.query_row(
                &format!("SELECT COUNT(*) FROM {tabla}"),
                [], |r| r.get(0))?;
            if tabla == "documento_tags" {
                tx.execute("INSERT OR IGNORE INTO tags (nombre) SELECT DISTINCT tag FROM src.documento_tags", [])?;
            }
            tx.execute(
                &format!("INSERT OR IGNORE INTO {tabla} ({cols}) SELECT {cols} FROM src.{tabla}"),
                [],
            )?;
            let despues: i64 = tx.query_row(
                &format!("SELECT COUNT(*) FROM {tabla}"),
                [], |r| r.get(0))?;
            let inserted = (despues - antes).max(0) as usize;
            *nuevos = inserted;
            *omitidos = (total as usize).saturating_sub(inserted);
        }
        tx.commit()?;
        self.conn.execute_batch("DETACH DATABASE src")?;
        Ok(stats)
    }

    /// Exporta toda la DB a un struct serializable. Para federación:
    /// dos instancias intercambian dumps JSON y mergean por id (Ulid es
    /// globally unique, no hay colisión espuria).
    pub fn exportar_todo(&self) -> Result<DbDump> {
        // Fuentes.
        let fuentes: Vec<Fuente> = self.listar_fuentes()?.into_iter().map(|f| f.fuente).collect();
        // Documentos.
        let mut docs_stmt = self.conn.prepare(
            "SELECT id, titulo, fuente_id FROM documentos"
        )?;
        let documentos: Vec<DumpDocumento> = docs_stmt.query_map([], |r| Ok((
            r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?,
        )))?.map(|r| {
            let (id_s, titulo, fid_s) = r.map_err(anyhow::Error::from)?;
            let id = DocId(Ulid::from_str(&id_s)?);
            let fuente_id = fid_s.map(|s| Ulid::from_str(&s)).transpose()?.map(FuenteId);
            Ok::<_, anyhow::Error>(DumpDocumento { id, titulo, fuente_id })
        }).collect::<Result<_>>()?;
        // Chunks (usamos iniy_ingest::Chunk que ya tiene serde).
        let mut chunks_stmt = self.conn.prepare("SELECT id, doc_id, orden, texto FROM chunks")?;
        let chunks: Vec<Chunk> = chunks_stmt.query_map([], |r| Ok((
            r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?,
        )))?.map(|r| {
            let (id_s, doc_s, orden, texto) = r.map_err(anyhow::Error::from)?;
            Ok::<_, anyhow::Error>(Chunk {
                id: ChunkId(Ulid::from_str(&id_s)?),
                doc_id: DocId(Ulid::from_str(&doc_s)?),
                orden: orden as u32,
                texto,
            })
        }).collect::<Result<_>>()?;
        // Aserciones.
        let mut a_stmt = self.conn.prepare(
            "SELECT id, doc_id, chunk_id, texto, opinion_json, fuente_citada_id FROM aserciones"
        )?;
        let aserciones: Vec<DumpAsercion> = a_stmt.query_map([], |r| Ok((
            r.get::<_, String>(0)?, r.get::<_, String>(1)?,
            r.get::<_, String>(2)?, r.get::<_, String>(3)?,
            r.get::<_, String>(4)?, r.get::<_, Option<String>>(5)?,
        )))?.map(|r| {
            let (id_s, doc_s, c_s, texto, op_json, fc_s) = r.map_err(anyhow::Error::from)?;
            let opinion: Opinion = serde_json::from_str(&op_json)?;
            Ok::<_, anyhow::Error>(DumpAsercion {
                asercion: Asercion {
                    id: AsercionId(Ulid::from_str(&id_s)?),
                    doc_id: DocId(Ulid::from_str(&doc_s)?),
                    chunk_id: ChunkId(Ulid::from_str(&c_s)?),
                    texto,
                    opinion_autoral: opinion,
                },
                fuente_citada_id: fc_s.map(|s| Ulid::from_str(&s)).transpose()?.map(FuenteId),
            })
        }).collect::<Result<_>>()?;
        // Implicaciones.
        let implicaciones = self.cargar_implicaciones_todas()?;
        // Tags.
        let mut tags_stmt = self.conn.prepare("SELECT doc_id, tag FROM documento_tags")?;
        let documento_tags: Vec<(DocId, String)> = tags_stmt.query_map([], |r| Ok((
            r.get::<_, String>(0)?, r.get::<_, String>(1)?,
        )))?.map(|r| {
            let (d, t) = r.map_err(anyhow::Error::from)?;
            Ok::<_, anyhow::Error>((DocId(Ulid::from_str(&d)?), t))
        }).collect::<Result<_>>()?;

        Ok(DbDump {
            iniy_version: env!("CARGO_PKG_VERSION").to_string(),
            exportado_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            fuentes,
            documentos,
            chunks,
            aserciones,
            implicaciones,
            documento_tags,
        })
    }

    /// Importa un dump producido por otra instancia. INSERT OR IGNORE en
    /// cada tabla: la entidad con id ya existente se respeta. Las
    /// reputaciones NO se importan (son derivadas; recalcular después).
    pub fn importar_dump(&mut self, dump: &DbDump) -> Result<ImportStats> {
        let mut stats = ImportStats::default();
        let tx = self.conn.transaction()?;
        for f in &dump.fuentes {
            let r = tx.execute(
                "INSERT OR IGNORE INTO fuentes (id, nombre, kind) VALUES (?1, ?2, ?3)",
                params![f.id.0.to_string(), f.nombre, f.kind],
            )?;
            if r > 0 { stats.fuentes += 1; } else { stats.fuentes_omitidas += 1; }
        }
        for d in &dump.documentos {
            let r = tx.execute(
                "INSERT OR IGNORE INTO documentos (id, titulo, fuente_id) VALUES (?1, ?2, ?3)",
                params![d.id.0.to_string(), d.titulo, d.fuente_id.map(|f| f.0.to_string())],
            )?;
            if r > 0 { stats.documentos += 1; } else { stats.documentos_omitidos += 1; }
        }
        for c in &dump.chunks {
            let r = tx.execute(
                "INSERT OR IGNORE INTO chunks (id, doc_id, orden, texto) VALUES (?1, ?2, ?3, ?4)",
                params![c.id.0.to_string(), c.doc_id.0.to_string(), c.orden, c.texto],
            )?;
            if r > 0 { stats.chunks += 1; } else { stats.chunks_omitidos += 1; }
        }
        for a in &dump.aserciones {
            let op_json = serde_json::to_string(&a.asercion.opinion_autoral)?;
            let r = tx.execute(
                "INSERT OR IGNORE INTO aserciones (id, doc_id, chunk_id, texto, opinion_json, fuente_citada_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    a.asercion.id.0.to_string(),
                    a.asercion.doc_id.0.to_string(),
                    a.asercion.chunk_id.0.to_string(),
                    a.asercion.texto, op_json,
                    a.fuente_citada_id.map(|f| f.0.to_string()),
                ],
            )?;
            if r > 0 { stats.aserciones += 1; } else { stats.aserciones_omitidas += 1; }
        }
        for i in &dump.implicaciones {
            let r = tx.execute(
                "INSERT OR IGNORE INTO implicaciones (premisa, hipotesis, entailment, contradiction, neutral) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    i.premisa.0.to_string(), i.hipotesis.0.to_string(),
                    i.relacion.entailment as f64, i.relacion.contradiction as f64, i.relacion.neutral as f64,
                ],
            )?;
            if r > 0 { stats.implicaciones += 1; } else { stats.implicaciones_omitidas += 1; }
        }
        for (doc_id, tag) in &dump.documento_tags {
            tx.execute("INSERT OR IGNORE INTO tags (nombre) VALUES (?1)", params![tag])?;
            let r = tx.execute(
                "INSERT OR IGNORE INTO documento_tags (doc_id, tag) VALUES (?1, ?2)",
                params![doc_id.0.to_string(), tag],
            )?;
            if r > 0 { stats.tags += 1; } else { stats.tags_omitidos += 1; }
        }
        tx.commit()?;
        Ok(stats)
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
