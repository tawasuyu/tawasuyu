//! Provider de filesystem local. Crate puro: cero dependencia de UI.
//! Implementa `nahual_core::DataProvider` listando hijos de un path con
//! `std::fs::read_dir` y leyendo archivos a `Vec<u8>` via `tokio::io`.

use async_trait::async_trait;
use shuma_discern::{DiscernPipeline, Hint};
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use nahual_core::{DataProvider, DisplayType, EntityNode};

pub const PROVIDER_ID: &str = "local_fs";

/// Bytes que samplea el discerner por archivo. 4 KiB cubre headers de
/// formatos comunes (PNG, ELF, JSON/TOML hasta una clave de profundidad
/// razonable) sin saturar I/O al expandir un directorio.
const DISCERN_SAMPLE_BYTES: usize = 4096;

/// Tamaño máximo de archivo que sampleamos. Archivos más grandes se
/// discernen igual via los primeros 4 KiB: el `seek/read` siempre lee
/// head, y el costo es O(SAMPLE) sin importar el size total.
/// Mantenemos esta constante por documentación; no se usa para skipear.
const _DISCERN_SAMPLE_DOC: () = ();

pub struct FileDataProvider {
    discerner: Arc<DiscernPipeline>,
}

impl FileDataProvider {
    pub fn new() -> Self {
        Self {
            discerner: Arc::new(DiscernPipeline::default_pipeline()),
        }
    }
}

impl Default for FileDataProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DataProvider for FileDataProvider {
    fn provider_id(&self) -> String {
        PROVIDER_ID.to_string()
    }

    async fn list_children(&self, parent_id: Option<&str>) -> Result<Vec<EntityNode>, String> {
        let path = parent_id.unwrap_or(".");
        let mut children = Vec::new();

        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let is_dir = path.is_dir();
                let display_type = if is_dir { DisplayType::Folder } else { DisplayType::File };

                // Discernimos sólo archivos. Folders no tienen MIME útil.
                let mime_type = if is_dir {
                    None
                } else {
                    discern_head(&path, &self.discerner)
                };

                children.push(EntityNode {
                    id: path.to_string_lossy().into_owned(),
                    name,
                    display_type,
                    mime_type,
                });
            }
        }

        Ok(children)
    }

    async fn get_read_stream(
        &self,
        entity_id: &str,
    ) -> Result<Pin<Box<dyn AsyncRead + Send>>, String> {
        let content = fs::read(Path::new(entity_id)).map_err(|e| e.to_string())?;
        Ok(Box::pin(Cursor::new(content)))
    }

    async fn get_write_stream(
        &self,
        _entity_id: &str,
    ) -> Result<Pin<Box<dyn AsyncWrite + Send>>, String> {
        Err("Escritura en streaming no implementada para FS".to_string())
    }
}

/// Lee el head del archivo y lo pasa por el DiscernPipeline. Devuelve el
/// MIME detectado (si alguno) o `None` si no hubo match.
///
/// Sync intencional: estamos dentro del runtime que ya es async, pero la
/// lectura es de tamaño fijo (4 KiB) y va a page cache; el costo de
/// `tokio::fs` no compensaría para esto.
fn discern_head(path: &Path, discerner: &DiscernPipeline) -> Option<String> {
    let mut buf = vec![0u8; DISCERN_SAMPLE_BYTES];
    let mut f = fs::File::open(path).ok()?;
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    let path_str = path.to_str();
    let hint = Hint {
        path: path_str,
        size_total: None,
    };
    discerner.discern(&buf, &hint).and_then(|d| d.mime)
}
