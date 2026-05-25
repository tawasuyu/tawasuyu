//! Provider de SQLite. Crate puro: cero dependencia de UI.
//! Tabla `items(id, parent_id, name, display_type, content)` con jerarquía
//! por `parent_id NULL` = raíz.

use async_trait::async_trait;
use rusqlite::Connection;
use std::io::Cursor;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncWrite};
use nahual_core::{DataProvider, DisplayType, EntityNode};

pub const PROVIDER_ID: &str = "sqlite_db";

pub struct SqliteDataProvider {
    db: Arc<Mutex<Connection>>,
}

impl SqliteDataProvider {
    pub fn new(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS items (
                id TEXT PRIMARY KEY,
                parent_id TEXT,
                name TEXT NOT NULL,
                display_type TEXT NOT NULL,
                content BLOB
            )",
            [],
        )
        .map_err(|e| e.to_string())?;

        // Seed mínimo si la tabla está vacía — para que el DatabaseExplorer
        // tenga algo que mostrar en una primera ejecución sin pre-config.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get(0))
            .unwrap_or(0);
        if count == 0 {
            let _ = conn.execute(
                "INSERT INTO items (id, parent_id, name, display_type, content) VALUES \
                 ('readme', NULL, 'README.md', 'File', ?), \
                 ('notes',  NULL, 'notes',     'Folder', NULL), \
                 ('todo',   'notes', 'TODO.md', 'File', ?)",
                rusqlite::params![
                    b"# Yahweh\n\nDemo readme stored in SQLite.\n",
                    b"- TreeView gen\xC3\xA9rico\n- containers swappables\n- layout JSON\n",
                ],
            );
        }

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }
}

#[async_trait]
impl DataProvider for SqliteDataProvider {
    fn provider_id(&self) -> String {
        PROVIDER_ID.to_string()
    }

    async fn list_children(&self, parent_id: Option<&str>) -> Result<Vec<EntityNode>, String> {
        let db = self.db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT id, name, display_type FROM items WHERE parent_id IS ?")
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([parent_id], |row| {
                let display_type_str: String = row.get(2)?;
                let display_type = match display_type_str.as_str() {
                    "Folder" => DisplayType::Folder,
                    "Stream" => DisplayType::Stream,
                    _ => DisplayType::File,
                };

                Ok(EntityNode {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    display_type,
                    mime_type: None,
                })
            })
            .map_err(|e| e.to_string())?;

        let mut children = Vec::new();
        for row in rows {
            children.push(row.map_err(|e| e.to_string())?);
        }
        Ok(children)
    }

    async fn get_read_stream(
        &self,
        entity_id: &str,
    ) -> Result<Pin<Box<dyn AsyncRead + Send>>, String> {
        let db = self.db.lock().unwrap();
        let content: Vec<u8> = db
            .query_row(
                "SELECT content FROM items WHERE id = ?",
                [entity_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;

        Ok(Box::pin(Cursor::new(content)))
    }

    async fn get_write_stream(
        &self,
        _entity_id: &str,
    ) -> Result<Pin<Box<dyn AsyncWrite + Send>>, String> {
        Err("Escritura en streaming no implementada para SQLite (todavía)".to_string())
    }
}
