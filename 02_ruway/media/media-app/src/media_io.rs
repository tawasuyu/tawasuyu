use std::path::{Path, PathBuf};
use std::time::Duration;

use media_core::SubtitleTrack;
use media_core::metadata::{self, Metadata};
use media_core::chapters::Chapters;

use crate::estado::{
    current_media_path, playlist_slot, video_path_slot,
};
use crate::playlist::{current_track_key, playback_snapshot};
use crate::config_io::bookmarks;

/// Verdadero si `s` parece una URL de red.
pub(crate) fn is_network_url(s: &str) -> bool {
    match s.split_once("://") {
        Some((scheme, rest)) => {
            !scheme.is_empty()
                && !rest.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
                && !scheme.eq_ignore_ascii_case("file")
        }
        None => false,
    }
}

/// Lee y parsea un archivo de subtítulos (autodetect SRT/WebVTT/ASS por cabecera).
pub(crate) fn load_subtitle_file(path: &Path) -> Option<SubtitleTrack> {
    match SubtitleTrack::load(path) {
        Ok(t) => {
            eprintln!("media-app: subtitles {} · {} cues", path.display(), t.len());
            Some(t)
        }
        Err(e) => {
            eprintln!("media-app: subtítulos en {}: {e}", path.display());
            None
        }
    }
}

/// S5: busca junto al video un subtítulo con su mismo nombre base y lo carga.
pub(crate) fn auto_load_sidecar_subtitles() -> Option<SubtitleTrack> {
    let video = video_path_slot().get()?;
    if is_network_url(&video.to_string_lossy()) {
        return None;
    }
    let cand = SubtitleTrack::find_sidecar(video)?;
    eprintln!("media-app: subtítulo sidecar {}", cand.display());
    load_subtitle_file(&cand)
}

/// Lee los primeros ~2 MB del archivo y parsea sus tags.
pub(crate) fn load_media_metadata(path: &Path) -> Metadata {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return Metadata::default();
    };
    let mut buf = Vec::new();
    let _ = file.take(2 * 1024 * 1024).read_to_end(&mut buf);
    metadata::parse(&buf)
}

/// Carátula decodificada (`peniko::Image`) del medio actual. Cacheada por la
/// clave del medio para refrescar cuando se abre otro en caliente.
pub(crate) fn cover_image() -> Option<llimphi_image::Image> {
    use parking_lot::Mutex;
    static CACHE: std::sync::OnceLock<Mutex<(String, Option<llimphi_image::Image>)>> =
        std::sync::OnceLock::new();
    let key = current_media_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cache = CACHE.get_or_init(|| Mutex::new((String::new(), None)));
    let mut g = cache.lock();
    if g.0 != key {
        let decoded = crate::estado::media_metadata_slot()
            .lock()
            .cover
            .as_ref()
            .and_then(|cover| match llimphi_image::decode_bytes(&cover.data) {
                Ok(img) => Some(img),
                Err(e) => {
                    eprintln!("media-app: carátula no decodifica: {e}");
                    None
                }
            });
        *g = (key, decoded);
    }
    g.1.clone()
}

/// Extrae los capítulos del archivo vía ffmpeg (ffmetadata) y los parsea.
pub(crate) fn load_chapters(path: &Path) -> Chapters {
    match foreign_av::ffmetadata(path) {
        Ok(text) => Chapters::parse_ffmetadata(&text),
        Err(_) => Chapters::default(),
    }
}

/// Fracciones (0..1) de las marcas del medio actual sobre la duración total.
pub(crate) fn bookmark_fractions() -> Vec<f32> {
    let Some(key) = current_track_key() else {
        return Vec::new();
    };
    let s = playback_snapshot();
    let dur = s.duration.unwrap_or(Duration::ZERO).as_secs_f64();
    if dur <= 0.0 {
        return Vec::new();
    }
    bookmarks()
        .lock()
        .for_media(&key)
        .iter()
        .map(|m| (m.position.as_secs_f64() / dur).clamp(0.0, 1.0) as f32)
        .collect()
}

/// Carga un .m3u simple.
pub(crate) fn load_playlist_file(path: &str) -> Result<Vec<PathBuf>, String> {
    let p = PathBuf::from(path);
    let body = std::fs::read_to_string(&p).map_err(|e| format!("io: {e}"))?;
    let base = p.parent().map(|d| d.to_path_buf());
    let mut out = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let item = PathBuf::from(line);
        let resolved = if item.is_absolute() {
            item
        } else if let Some(b) = &base {
            b.join(item)
        } else {
            item
        };
        out.push(resolved);
    }
    Ok(out)
}

/// Formatea una duración como `M:SS`.
pub(crate) fn fmt_secs(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

/// Formatea una duración como `M:SS` (o `H:MM:SS` si pasa la hora).
pub(crate) fn fmt_mmss(d: Duration) -> String {
    let t = d.as_secs();
    let (h, m, s) = (t / 3600, (t % 3600) / 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Título del medio para mostrar.
pub(crate) fn media_title_string() -> String {
    let md = crate::estado::media_metadata_slot().lock();
    let base = md
        .title
        .clone()
        .or_else(|| crate::estado::config_slot().get().map(|c| c.label.clone()))
        .unwrap_or_default();
    let mut label = match md.artist.as_deref() {
        Some(artist) if !artist.is_empty() => format!("{base} — {artist}"),
        _ => base,
    };
    drop(md);
    let ch = crate::estado::chapters_slot().lock();
    if let Some((_, c)) = ch.at(playback_snapshot().position) {
        if !c.title.is_empty() {
            label = format!("{label}  ·  ▸ {}", c.title);
        }
    }
    label
}
