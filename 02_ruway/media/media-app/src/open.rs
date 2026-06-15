//! open — **apertura de un medio en caliente** (swap de video/audio sobre el
//! mismo motor), para "un proceso, varios medios, uno a la vez".
//!
//! El pipeline de video guarda su fuente en un `Mutex<Box<dyn FrameSource>>`
//! y el sink de audio comparte el `Arc<Mutex<Playlist>>`; abrir otro medio
//! reemplaza **ambos en su lugar** sin reabrir el device GPU ni el device de
//! audio. Reusa exactamente los constructores de fuentes de `pipeline.rs` y
//! `playlist.rs`. Refresca metadata/capítulos/subtítulos/onda del nuevo medio.
//!
//! Limitaciones MVP (documentadas): la selección de pistas embebidas
//! (CycleAudioTrack/CycleSubtitleTrack) sigue apuntando a la sesión ffmpeg de
//! arranque; el "siguiente/anterior" entre videos de una playlist no está
//! cableado (cada apertura es un medio único).

use std::path::{Path, PathBuf};

use media_core::color::ColorVideo;
use media_core::transform::TransformVideo;
use media_core::{FrameSource, SubtitleTrack};
use media_source_gif::GifSource;
use media_source_image::ImageSource;
use foreign_av::MediaSession;

use crate::estado::{
    chapters_slot, color, media_metadata_slot, pipeline_slot, playlist_labels_slot, playlist_slot,
    reset_av_sync_anchor, set_video_fps, subtitles_slot, transform, waveform_slot, TESTCARD_FPS,
};
use crate::media_io::{load_chapters, load_media_metadata, media_title_string};
use crate::pipeline::new_testcard;
use crate::playlist::LoadedTrack;

/// Extensiones de audio que el `Playlist` decodifica **nativo** (puro-Rust).
pub(crate) fn is_native_audio(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("wav" | "mp3" | "opus" | "ogg")
    )
}

fn ext_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
}

/// Construye la fuente de video + la pista de audio para `path`. Nunca
/// falla: ante un error cae a testcard (video) y silencio (audio).
fn build_for(path: &Path) -> (Box<dyn FrameSource + Send>, LoadedTrack) {
    // Audio nativo (sin video): testcard + pista nativa.
    if is_native_audio(path) {
        set_video_fps(TESTCARD_FPS);
        let audio = LoadedTrack::from_path(path).unwrap_or(LoadedTrack::Silent);
        return (new_testcard(), audio);
    }
    match ext_of(path).as_deref() {
        Some("gif") => match GifSource::from_path(path) {
            Ok(s) => {
                set_video_fps(TESTCARD_FPS);
                (Box::new(s), LoadedTrack::Silent)
            }
            Err(e) => {
                eprintln!("media-app: GIF {path:?}: {e} — testcard");
                (new_testcard(), LoadedTrack::Silent)
            }
        },
        Some("png" | "jpg" | "jpeg" | "webp" | "bmp" | "tiff") => {
            match ImageSource::from_path(path) {
                Ok(s) => {
                    set_video_fps(TESTCARD_FPS);
                    (Box::new(s), LoadedTrack::Silent)
                }
                Err(e) => {
                    eprintln!("media-app: imagen {path:?}: {e} — testcard");
                    (new_testcard(), LoadedTrack::Silent)
                }
            }
        }
        Some("ivf") => match media_source_av1::Av1VideoSource::open(path) {
            Ok(s) => {
                set_video_fps(s.fps());
                (Box::new(s), LoadedTrack::Silent)
            }
            Err(e) => {
                eprintln!("media-app: AV1 {path:?}: {e} — testcard");
                (new_testcard(), LoadedTrack::Silent)
            }
        },
        // Todo lo demás (mp4/webm/mkv/mov/avi/flv/m4v/ogv + flac/m4a/aac…) por
        // ffmpeg: una sesión, clonada para video y audio.
        _ => match foreign_av::probe(path).and_then(MediaSession::open) {
            Ok(session) => {
                let video: Box<dyn FrameSource + Send> =
                    match foreign_av::FfmpegVideoSource::from_session(session.clone()) {
                        Ok(v) => {
                            set_video_fps(v.fps());
                            Box::new(v)
                        }
                        Err(_) => {
                            // Audio-only (flac/m4a/aac…): testcard de fondo.
                            set_video_fps(TESTCARD_FPS);
                            new_testcard()
                        }
                    };
                let audio = match foreign_av::FfmpegAudioSource::from_session(session) {
                    Ok(a) => LoadedTrack::FfmpegAudio(a),
                    Err(e) => {
                        eprintln!("media-app: ffmpeg audio {path:?}: {e}");
                        LoadedTrack::Silent
                    }
                };
                (video, audio)
            }
            Err(e) => {
                eprintln!("media-app: ffmpeg abrir {path:?}: {e} — testcard");
                (new_testcard(), LoadedTrack::Silent)
            }
        },
    }
}

/// Abre `path` **en caliente**: reemplaza la fuente de video del pipeline y la
/// pista de audio del motor vivo, y refresca el estado del medio. Devuelve un
/// rótulo para el OSD. `Err` sólo si el pipeline aún no se construyó (no se
/// renderizó ningún frame todavía).
pub(crate) fn open_media(path: &Path) -> Result<String, String> {
    eprintln!("media-app: open_media({})", path.display());
    let pipe = pipeline_slot()
        .get()
        .ok_or_else(|| "el pipeline aún no se inicializó (no hubo frame)".to_string())?;

    let (video, audio) = build_for(path);
    eprintln!("media-app: fuente nueva construida, swapeando pipeline…");
    let wrapped: Box<dyn FrameSource + Send> = Box::new(TransformVideo::new(
        ColorVideo::new(video, color().clone()),
        transform().clone(),
    ));
    *pipe.source.lock() = wrapped;
    *pipe.last_dim.lock() = (0, 0);

    if let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) {
        h.lock().set_current_track(path.to_path_buf(), audio);
    }
    let label = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    *playlist_labels_slot().lock() = vec![label];

    // Estado por-medio.
    if path.is_file() {
        *media_metadata_slot().lock() = load_media_metadata(path);
        *chapters_slot().lock() = load_chapters(path);
        *subtitles_slot().lock() = SubtitleTrack::find_sidecar(path)
            .and_then(|c| SubtitleTrack::load(&c).ok());
    } else {
        *media_metadata_slot().lock() = Default::default();
        *chapters_slot().lock() = Default::default();
        *subtitles_slot().lock() = None;
    }
    // La onda se recomputa en background; mientras tanto la timeline cae a
    // la barra lisa.
    *waveform_slot().lock() = None;
    reset_av_sync_anchor();

    let title = media_title_string();
    Ok(if title.trim().is_empty() {
        path.display().to_string()
    } else {
        title
    })
}

/// Lanza el escaneo de onda (Audacity-like) de `path` en background, sin
/// prioridad, y alimenta la timeline al terminar. No-op si no es archivo.
pub(crate) fn spawn_waveform_scan(handle: &llimphi_ui::Handle<crate::tipos::Msg>, path: PathBuf) {
    if !path.is_file() {
        return;
    }
    handle.spawn(move || {
        match foreign_av::decode_peaks(&path, 1600) {
            Ok(w) => *waveform_slot().lock() = Some(w),
            Err(e) => eprintln!("media-app: escaneo de onda: {e}"),
        }
        crate::tipos::Msg::WaveformReady
    });
}
