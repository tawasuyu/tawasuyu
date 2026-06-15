//! open — **apertura de un medio en caliente** (swap de video/audio sobre el
//! mismo motor), para "un proceso, varios medios, uno a la vez".
//!
//! El pipeline de video guarda su fuente en un `Mutex<Box<dyn FrameSource>>`
//! y el sink de audio comparte el `Arc<Mutex<Playlist>>`; abrir otro medio
//! reemplaza **ambos en su lugar** sin reabrir el device GPU ni el device de
//! audio. Reusa exactamente los constructores de fuentes de `pipeline.rs` y
//! `playlist.rs`. Refresca metadata/capítulos/subtítulos/onda del nuevo medio.
//!
//! `open_playlist_index` hace el mismo swap pero **preservando la cola**, así
//! anterior/siguiente y el auto-advance del tick de UI funcionan también con
//! video (no sólo audio nativo).
//!
//! Limitaciones MVP (documentadas): la selección de pistas embebidas
//! (CycleAudioTrack/CycleSubtitleTrack) sigue apuntando a la sesión ffmpeg de
//! arranque.

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

    refresh_media_state(path);
    reset_av_sync_anchor();

    Ok(title_or_path(path))
}

/// Refresca el estado por-medio (metadata/carátula, capítulos, subtítulo
/// sidecar) y resetea la onda para que se recompute en background. Compartido
/// por [`open_media`] (medio único) y [`open_playlist_index`] (cola viva).
fn refresh_media_state(path: &Path) {
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
}

/// Título legible del medio (metadata), con la ruta como respaldo.
fn title_or_path(path: &Path) -> String {
    let title = media_title_string();
    if title.trim().is_empty() {
        path.display().to_string()
    } else {
        title
    }
}

/// Abre la pista `target` de la **cola viva en caliente**, preservando la lista
/// (a diferencia de [`open_media`], que la colapsa a un único medio). Hace el
/// swap completo video+audio, así anterior/siguiente y el auto-advance también
/// funcionan con **video**, no sólo audio nativo. Devuelve el rótulo del OSD.
pub(crate) fn open_playlist_index(target: usize) -> Result<String, String> {
    let pipe = pipeline_slot()
        .get()
        .ok_or_else(|| "el pipeline aún no se inicializó (no hubo frame)".to_string())?;
    let h = playlist_slot()
        .get()
        .and_then(|o| o.as_ref())
        .ok_or_else(|| "no hay motor de audio activo".to_string())?;
    let path = h
        .lock()
        .track_at(target)
        .map(|p| p.to_path_buf())
        .ok_or_else(|| format!("índice de cola fuera de rango: {target}"))?;

    let (video, audio) = build_for(&path);
    let wrapped: Box<dyn FrameSource + Send> = Box::new(TransformVideo::new(
        ColorVideo::new(video, color().clone()),
        transform().clone(),
    ));
    *pipe.source.lock() = wrapped;
    *pipe.last_dim.lock() = (0, 0);

    h.lock().set_track_at(target, audio);
    // La lista no se colapsa: refrescamos los rótulos de TODAS las pistas.
    *playlist_labels_slot().lock() = h.lock().track_labels();

    refresh_media_state(&path);
    reset_av_sync_anchor();
    Ok(title_or_path(&path))
}

/// Carga una lista de medios (audio **o** video, mixta) en la cola viva y abre
/// la primera en caliente. Reemplaza al viejo camino audio-only. Devuelve
/// `(cantidad, ruta_primera)` para relanzar la onda desde el caller.
pub(crate) fn load_playlist_live(
    entries: &[String],
) -> Result<(usize, PathBuf), String> {
    let h = playlist_slot()
        .get()
        .and_then(|o| o.as_ref())
        .ok_or_else(|| "no hay motor de audio activo".to_string())?;
    let tracks: Vec<PathBuf> = entries.iter().map(PathBuf::from).collect();
    if tracks.is_empty() {
        return Err("playlist vacía".into());
    }
    let count = tracks.len();
    let first = tracks[0].clone();
    h.lock().set_list(tracks);
    open_playlist_index(0)?;
    Ok((count, first))
}

/// Resultado del auto-advance del tick de UI.
pub(crate) enum AdvanceOutcome {
    None,
    /// La pista actual se re-arrancó desde cero (loop de video).
    Looped,
    /// Se cambió de pista; el caller debería relanzar la onda de esta ruta.
    Switched(PathBuf),
}

/// Ejecuta el auto-advance de pistas **no nativas** (video/ffmpeg) que el hilo
/// de audio no puede manejar (requiere reconstruir el pipeline). Lo llama el
/// tick de UI. El audio nativo se auto-avanza en su propio hilo.
pub(crate) fn poll_video_advance() -> AdvanceOutcome {
    let Some(h) = playlist_slot().get().and_then(|o| o.as_ref()) else {
        return AdvanceOutcome::None;
    };
    let action = h.lock().tick_advance();
    match action {
        crate::playlist::TickAdvance::None => AdvanceOutcome::None,
        crate::playlist::TickAdvance::Loop => {
            crate::playlist::seek_audio_to_pos(std::time::Duration::ZERO);
            AdvanceOutcome::Looped
        }
        crate::playlist::TickAdvance::Switch(t) => match open_playlist_index(t) {
            Ok(title) => {
                crate::estado::osd_flash(format!("▶ {title}"));
                let path = playlist_slot()
                    .get()
                    .and_then(|o| o.as_ref())
                    .map(|h| h.lock().track_path().to_path_buf())
                    .unwrap_or_default();
                AdvanceOutcome::Switched(path)
            }
            Err(e) => {
                eprintln!("media-app: auto-advance: {e}");
                AdvanceOutcome::None
            }
        },
    }
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
