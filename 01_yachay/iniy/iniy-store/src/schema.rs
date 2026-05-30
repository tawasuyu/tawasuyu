//! Apertura del store y migraciones idempotentes del esquema.

use super::*;

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

    pub(crate) fn migrar(&self) -> Result<()> {
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

}
