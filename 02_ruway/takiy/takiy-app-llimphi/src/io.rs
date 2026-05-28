//! Persistencia del score — escritura atómica y carga con fallback.

use std::io;
use std::path::{Path, PathBuf};

use takiy_core::Score;

/// Errores al cargar un score desde disco.
#[derive(Debug)]
pub enum LoadError {
    /// No se pudo leer el archivo (no existe, permisos, etc.).
    Read(io::Error),
    /// El JSON no parsea contra el tipo `Score`.
    Parse(serde_json::Error),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(e) => write!(f, "no se pudo leer: {e}"),
            Self::Parse(e) => write!(f, "JSON inválido: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Serializa el score a JSON pretty y lo escribe atómicamente a `path`:
/// primero escribe a `<path>.tmp` y después renombra, así una interrupción
/// (Ctrl+C, kill, falla de disco a mitad del write) no deja el archivo
/// truncado. Si el rename falla, devuelve el error de `rename`.
pub fn write_score(score: &Score, path: &Path) -> io::Result<()> {
    let json = serde_json::to_string_pretty(score)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("takiy.json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

/// Carga un score desde `path`. No tiene fallback — el caller decide
/// qué hacer si falla (typ. caer a un demo built-in).
pub fn load_score(path: &Path) -> Result<Score, LoadError> {
    let s = std::fs::read_to_string(path).map_err(LoadError::Read)?;
    serde_json::from_str(&s).map_err(LoadError::Parse)
}

/// Resuelve el path donde guardar cuando el usuario aprieta `S` y no hay
/// `TAKIY_SCORE_JSON` configurado: `/tmp/takiy_<unix>.takiy.json`. Se
/// expone para que el test pueda construirlo determinista sin tocar el
/// reloj del sistema.
pub fn default_save_path(unix_secs: u64) -> PathBuf {
    PathBuf::from(format!("/tmp/takiy_{unix_secs}.takiy.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use takiy_core::{Pitch, ScoreNote, Track};

    fn sample_score() -> Score {
        let mut s = Score::new(96.0);
        let mut t = Track::new("melodía");
        t.add(ScoreNote::new(Pitch::A4, 0.0, 1.0, 100));
        t.add(ScoreNote::new(Pitch::MIDDLE_C, 1.0, 0.5, 80));
        s.add_track(t);
        s
    }

    #[test]
    fn write_and_read_roundtrip() {
        let s = sample_score();
        let path = std::env::temp_dir().join("takiy-io-roundtrip.takiy.json");
        write_score(&s, &path).unwrap();
        let back = load_score(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, s);
    }

    #[test]
    fn write_is_atomic_no_tmp_left_behind() {
        let s = sample_score();
        let path = std::env::temp_dir().join("takiy-io-atomic.takiy.json");
        let tmp = path.with_extension("takiy.json.tmp");
        write_score(&s, &path).unwrap();
        assert!(path.exists());
        assert!(!tmp.exists(), "tmp leftover: {}", tmp.display());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_score_returns_parse_error_on_garbage() {
        let path = std::env::temp_dir().join("takiy-io-garbage.takiy.json");
        std::fs::write(&path, "not json").unwrap();
        let err = load_score(&path).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(matches!(err, LoadError::Parse(_)));
    }

    #[test]
    fn load_score_returns_read_error_on_missing_file() {
        let path = std::env::temp_dir().join("takiy-io-missing-XYZ.takiy.json");
        let _ = std::fs::remove_file(&path);
        let err = load_score(&path).unwrap_err();
        assert!(matches!(err, LoadError::Read(_)));
    }

    #[test]
    fn default_save_path_uses_unix_timestamp() {
        let p = default_save_path(1_700_000_000);
        assert_eq!(p, PathBuf::from("/tmp/takiy_1700000000.takiy.json"));
    }
}
