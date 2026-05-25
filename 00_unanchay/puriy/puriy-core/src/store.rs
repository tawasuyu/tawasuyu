//! Persistencia del [`Profile`] a un archivo JSON. Escritura atómica
//! (tmp → fsync → rename) con envelope versionado.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::profile::Profile;

/// Versión actual del esquema en disco.
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
    profile: Profile,
}

pub fn load(ruta: &Path) -> Result<Profile> {
    let f = File::open(ruta)?;
    let env: Envelope = serde_json::from_reader(BufReader::new(f))?;
    if env.schema != SCHEMA {
        return Err(Error::SchemaDesconocida { found: env.schema });
    }
    Ok(env.profile)
}

pub fn save(ruta: &Path, profile: &Profile) -> Result<()> {
    let env = Envelope { schema: SCHEMA, profile: profile.clone() };
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

    fn profile_ejemplo() -> Profile {
        let mut p = Profile::nuevo("sergio");
        let tab = p.session.open("https://gioser.net", 100);
        p.session.set_title(tab, "gioser").unwrap();
        p.history.record("https://gioser.net", "gioser", 100);
        p.history.record("https://docs.rs", "docs.rs", 110);
        p.bookmarks.add("https://gioser.net", "gioser", None, 100);
        p.bookmarks.add("https://docs.rs", "docs", Some("dev".into()), 110);
        p
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("perfil.json");
        let original = profile_ejemplo();
        save(&ruta, &original).unwrap();
        let cargado = load(&ruta).unwrap();

        assert_eq!(cargado.name, original.name);
        assert_eq!(cargado.session.len(), original.session.len());
        assert_eq!(cargado.session.active(), original.session.active());
        assert_eq!(cargado.history.len(), original.history.len());
        assert_eq!(cargado.bookmarks.len(), original.bookmarks.len());
    }

    #[test]
    fn save_no_deja_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("p.json");
        save(&ruta, &profile_ejemplo()).unwrap();
        assert!(ruta.exists());
        assert!(!tmp_path(&ruta).exists());
    }

    #[test]
    fn schema_desconocida_falla() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("futuro.json");
        fs::write(
            &ruta,
            r#"{"schema": 999, "profile": {"name": "x", "session": {"tabs": [], "active": null}, "history": {"entries": []}, "bookmarks": {"items": []}}}"#,
        )
        .unwrap();
        assert!(matches!(load(&ruta), Err(Error::SchemaDesconocida { found: 999 })));
    }
}
