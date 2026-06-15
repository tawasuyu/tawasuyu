//! profiles — capa de **I/O + cripto + escaneo** de perfiles del reproductor.
//!
//! El modelo puro vive en `media_core::profile` (perfiles, playlists, candado
//! como hash opaco). Acá se hace lo que el core no toca: persistir el
//! [`ProfileStore`] a `profiles.ron`, computar el hash BLAKE3 del password,
//! escanear un directorio **recursivamente** por archivos de medios, y
//! reemplazar **en caliente** la playlist viva del motor de audio.

use std::path::{Path, PathBuf};

use media_core::profile::{NamedPlaylist, ProfileStore};

use crate::estado::{config_file, playlist_labels_slot, playlist_slot, reset_av_sync_anchor};
use crate::playlist::Playlist;

/// Path del `profiles.ron`.
pub(crate) fn profiles_path() -> Option<PathBuf> {
    config_file("profiles.ron")
}

/// Carga los perfiles del disco (o un store vacío), saneado.
pub(crate) fn load_profiles() -> ProfileStore {
    let Some(p) = profiles_path() else {
        return ProfileStore::default();
    };
    match std::fs::read_to_string(&p) {
        Ok(body) => ron::from_str::<ProfileStore>(&body)
            .map(ProfileStore::sanitized)
            .unwrap_or_default(),
        Err(_) => ProfileStore::default(),
    }
}

/// Persiste los perfiles a `profiles.ron` (best-effort, sólo log).
pub(crate) fn save_profiles(store: &ProfileStore) {
    let Some(p) = profiles_path() else {
        return;
    };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match ron::ser::to_string_pretty(store, ron::ser::PrettyConfig::default()) {
        Ok(txt) => {
            if let Err(e) = std::fs::write(&p, txt) {
                eprintln!("media-app: no pude escribir profiles.ron: {e}");
            }
        }
        Err(e) => eprintln!("media-app: no pude serializar perfiles: {e}"),
    }
}

/// Hash BLAKE3 (hex) de una contraseña. El core sólo compara strings; la
/// fuerza criptográfica vive acá. Es un candado **blando** (local), no una
/// frontera de seguridad — protege de un vistazo casual, no de un atacante.
pub(crate) fn hash_password(password: &str) -> String {
    blake3::hash(password.as_bytes()).to_hex().to_string()
}

/// Extensiones de medios que el reproductor reconoce para una playlist.
pub(crate) fn is_media_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(
            // Audio.
            "wav" | "mp3" | "opus" | "ogg" | "flac" | "m4a" | "aac"
            // Video.
            | "mp4" | "webm" | "mkv" | "mov" | "avi" | "flv" | "m4v" | "ogv" | "ivf"
        )
    )
}

/// Escanea `root` **recursivamente** y devuelve las rutas de medios
/// ordenadas (estable). No sigue symlinks; corta a 50_000 entradas para no
/// colgarse en un árbol patológico.
pub(crate) fn scan_dir_recursive(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        for path in entries {
            if out.len() >= 50_000 {
                eprintln!("media-app: escaneo cortado en 50k entradas");
                return out;
            }
            match std::fs::metadata(&path) {
                Ok(m) if m.is_dir() => stack.push(path),
                Ok(_) if is_media_file(&path) => out.push(path),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// Construye una [`NamedPlaylist`] escaneando un directorio recursivamente.
/// El nombre por defecto es el del directorio. `None` si no halló medios.
pub(crate) fn playlist_from_dir(dir: &Path) -> Option<NamedPlaylist> {
    let entries = scan_dir_recursive(dir);
    if entries.is_empty() {
        return None;
    }
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| dir.display().to_string());
    let entries: Vec<String> = entries
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    Some(NamedPlaylist::new(name, entries))
}

/// Reemplaza **en caliente** la playlist del motor de audio vivo por las
/// `entries` dadas (rutas). Mismo `Arc<Mutex<Playlist>>` que comparte el sink,
/// así no se reabre el device. Devuelve los rótulos de las pistas cargadas
/// para refrescar la Cola, o `Err` si no hay motor / falló la primera pista.
pub(crate) fn load_entries_into_live(entries: &[String]) -> Result<Vec<String>, String> {
    let handle = playlist_slot()
        .get()
        .and_then(|o| o.as_ref())
        .ok_or_else(|| "no hay motor de audio activo".to_string())?;
    let tracks: Vec<PathBuf> = entries.iter().map(PathBuf::from).collect();
    let mut pl = handle.lock();
    pl.load_tracks(tracks)?;
    let labels = pl.track_labels();
    drop(pl);
    *playlist_labels_slot().lock() = labels.clone();
    reset_av_sync_anchor();
    Ok(labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_es_determinista_y_distingue() {
        assert_eq!(hash_password("clave"), hash_password("clave"));
        assert_ne!(hash_password("clave"), hash_password("clavE"));
        // BLAKE3 hex = 64 chars.
        assert_eq!(hash_password("x").len(), 64);
    }

    #[test]
    fn filtra_extensiones_de_medios() {
        assert!(is_media_file(Path::new("a.mp3")));
        assert!(is_media_file(Path::new("b.MKV")));
        assert!(is_media_file(Path::new("c.opus")));
        assert!(!is_media_file(Path::new("d.txt")));
        assert!(!is_media_file(Path::new("noext")));
    }

    #[test]
    fn escaneo_recursivo_ordena_y_filtra() {
        let base = std::env::temp_dir().join(format!("media-prof-test-{}", std::process::id()));
        let sub = base.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(base.join("b.mp3"), b"x").unwrap();
        std::fs::write(base.join("a.wav"), b"x").unwrap();
        std::fs::write(base.join("notas.txt"), b"x").unwrap();
        std::fs::write(sub.join("c.opus"), b"x").unwrap();

        let found = scan_dir_recursive(&base);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // Sólo medios, ordenados; recursión incluye el subdir.
        assert_eq!(names, vec!["a.wav", "b.mp3", "c.opus"]);

        let pl = playlist_from_dir(&base).expect("playlist");
        assert_eq!(pl.entries.len(), 3);
        assert_eq!(pl.name, base.file_name().unwrap().to_string_lossy());

        std::fs::remove_dir_all(&base).ok();
    }
}
