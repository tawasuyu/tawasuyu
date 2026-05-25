//! `pluma-notebook-store` — save/load atómico de un [`Notebook`] a disco.
//!
//! Formato: JSON con header de versión de esquema. La escritura es atómica
//! (write → fsync → rename), así que un crash a media escritura no corrompe
//! el archivo final. Convención de extensión: `.pluma-nb`.

#![forbid(unsafe_code)]

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use pluma_notebook_core::Notebook;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Versión actual del esquema en disco. Subir solo ante cambios de formato
/// incompatibles; la lectura rechaza versiones desconocidas.
pub const SCHEMA: u32 = 1;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("versión de esquema desconocida: {found} (esta build soporta {SCHEMA})")]
    SchemaDesconocida { found: u32 },
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Serialize, Deserialize)]
struct Envelope {
    schema: u32,
    notebook: Notebook,
}

/// Carga un notebook desde un archivo `.pluma-nb`.
pub fn load(ruta: &Path) -> Result<Notebook> {
    let f = File::open(ruta)?;
    let env: Envelope = serde_json::from_reader(BufReader::new(f))?;
    if env.schema != SCHEMA {
        return Err(Error::SchemaDesconocida { found: env.schema });
    }
    Ok(env.notebook)
}

/// Guarda un notebook de forma atómica: escribe a `<ruta>.tmp`, hace fsync,
/// y renombra. Si algo falla a mitad, el archivo original queda intacto.
pub fn save(ruta: &Path, notebook: &Notebook) -> Result<()> {
    let env = Envelope { schema: SCHEMA, notebook: notebook.clone() };
    let tmp = tmp_path(ruta);

    {
        let f = File::create(&tmp)?;
        let mut w = BufWriter::new(f);
        serde_json::to_writer_pretty(&mut w, &env)?;
        w.flush()?;
        w.into_inner()
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .sync_all()?;
    }

    fs::rename(&tmp, ruta)?;
    Ok(())
}

fn tmp_path(ruta: &Path) -> PathBuf {
    let mut s = ruta.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pluma_notebook_core::CellKind;

    fn notebook_ejemplo() -> Notebook {
        let mut nb = Notebook::new();
        let a = nb.push(CellKind::Markdown, "# titulo");
        let b = nb.push(CellKind::Code { language: "rust".into() }, "let x = 1;");
        nb.add_dependency(b, a);
        nb
    }

    #[test]
    fn save_load_roundtrip_preserva_digest() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("ejemplo.pluma-nb");
        let original = notebook_ejemplo();
        save(&ruta, &original).unwrap();
        let cargado = load(&ruta).unwrap();
        assert_eq!(original.notebook_digest(), cargado.notebook_digest());
        assert_eq!(original.len(), cargado.len());
    }

    #[test]
    fn save_es_atomico_no_deja_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("at.pluma-nb");
        save(&ruta, &notebook_ejemplo()).unwrap();
        assert!(ruta.exists());
        assert!(!tmp_path(&ruta).exists());
    }

    #[test]
    fn schema_desconocida_falla() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("futuro.pluma-nb");
        fs::write(&ruta, r#"{"schema": 999, "notebook": {"cells": [], "next_id": 1}}"#).unwrap();
        assert!(matches!(load(&ruta), Err(Error::SchemaDesconocida { found: 999 })));
    }
}
