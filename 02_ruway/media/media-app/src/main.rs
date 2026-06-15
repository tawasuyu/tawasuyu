//! media-app — primer reproductor del dominio.
//!
//! Pipeline video: una fuente [`FrameSource`] genera RGBA, lo empuja
//! a un [`llimphi_surface::ExternalSurface`], y la UI Llimphi lo
//! expone en un canvas central vía `View::gpu_paint_with`. Con
//! argumento es un GIF en disco (loop infinito); sin argumento cae
//! al [`TestCard`] sintético.
//!
//! Pipeline audio: junto al video se abre un sink cpal sobre el
//! default output device, alimentado por un [`ToneSource`] (A4 a
//! -12 dB). Si el sink no puede abrir el device, se loguea y se
//! sigue sólo con video — la falta de audio no aborta la app.
//!
//! Ver el archivo original para el comentario completo del módulo.

mod tipos;
mod estado;
mod config_io;
mod playlist;
mod comandos;
mod media_io;
mod pipeline;
mod modelo;
mod vista;
mod vista_config;
mod dock;
mod audio_source;

use std::path::{Path, PathBuf};

use media_audio_cpal::AudioSink;
use foreign_av::MediaSession;

use crate::tipos::*;
use crate::estado::*;
use crate::config_io::*;
use crate::playlist::*;
use crate::media_io::*;
use crate::pipeline::*;
use crate::audio_source::*;

fn main() {
    rimay_localize::init();
    let _ = rimay_localize::set_locale(&wawa_config::WawaConfig::load().lang);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match args.first() {
        Some(arg) if is_network_url(arg) => {
            let stream = if foreign_ytdlp::is_platform_url(arg) {
                match foreign_ytdlp::resolve_best(arg) {
                    Ok(r) => {
                        if let Some(audio) = &r.audio_url {
                            eprintln!(
                                "media-app: yt-dlp resolvió {arg} → DASH (video+audio separados)"
                            );
                            dash_audio_slot().set(PathBuf::from(audio.clone())).ok();
                        } else {
                            eprintln!("media-app: yt-dlp resolvió {arg} → stream muxeado");
                        }
                        r.stream_url
                    }
                    Err(e) => {
                        eprintln!("media-app: yt-dlp falló ({e}) — pruebo la URL directo");
                        arg.clone()
                    }
                }
            } else {
                arg.clone()
            };
            eprintln!("media-app: stream de red → decoder ffmpeg");
            video_path_slot().set(PathBuf::from(stream)).ok();
            Config {
                label: format!("stream {arg}"),
                kind: VideoKind::Ffmpeg,
            }
        }
        Some(path) => {
            let path = PathBuf::from(path);
            let kind = match path
                .extension()
                .and_then(|s| s.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("gif") => VideoKind::Gif,
                Some("png" | "jpg" | "jpeg" | "webp" | "bmp" | "tiff") => VideoKind::Image,
                Some("ivf") => VideoKind::Av1,
                Some("mp4" | "webm" | "mkv" | "mov" | "avi" | "flv" | "m4v" | "ogv") => {
                    VideoKind::Ffmpeg
                }
                other => {
                    eprintln!(
                        "media-app: extensión {:?} no reconocida — caigo a testcard",
                        other
                    );
                    VideoKind::Testcard
                }
            };
            let label = match kind {
                VideoKind::Gif => format!("gif {}", path.display()),
                VideoKind::Image => format!("img {}", path.display()),
                VideoKind::Ffmpeg => format!("video {}", path.display()),
                VideoKind::Av1 => format!("av1 {}", path.display()),
                VideoKind::Testcard => format!(
                    "testcard {TESTCARD_W}×{TESTCARD_H} @ {TESTCARD_FPS:.0} fps"
                ),
            };
            if !matches!(kind, VideoKind::Testcard) {
                video_path_slot().set(path).ok();
            }
            Config { label, kind }
        }
        None => Config {
            label: format!("testcard {TESTCARD_W}×{TESTCARD_H} @ {TESTCARD_FPS:.0} fps"),
            kind: VideoKind::Testcard,
        },
    };
    config_slot().set(cfg).ok();
    *settings_slot().write().expect("settings lock") = load_settings();

    if let (Some(path), Some(VideoKind::Ffmpeg)) =
        (video_path_slot().get(), config_slot().get().map(|c| c.kind))
    {
        let probed = match dash_audio_slot().get() {
            Some(audio) => foreign_av::probe_dash(path, audio),
            None => foreign_av::probe(path),
        };
        match probed.and_then(MediaSession::open) {
            Ok(session) => {
                eprintln!(
                    "media-app: ffmpeg session abierta ({})",
                    path.display()
                );
                ffmpeg_session_slot().set(Some(session)).ok();
            }
            Err(e) => {
                eprintln!("media-app: ffmpeg session falló: {e}");
                ffmpeg_session_slot().set(None).ok();
            }
        }
    }

    let env_path = std::env::var("MEDIA_SRT")
        .or_else(|_| std::env::var("MEDIA_VTT"))
        .or_else(|_| std::env::var("MEDIA_ASS"))
        .ok();
    let subs = match env_path {
        Some(path) => load_subtitle_file(Path::new(&path)),
        None => auto_load_sidecar_subtitles(),
    };
    *subtitles_slot().lock() = subs;

    let _audio_sink = if std::env::var("MEDIA_MUTE").is_err() {
        let (source, probe) = audio_source_from_env();
        apply_startup_config();
        match AudioSink::open(source) {
            Ok(sink) => {
                eprintln!(
                    "media-app: audio cpal abierto @ {} Hz · {} ch",
                    sink.sample_rate(),
                    sink.channels(),
                );
                audio_probe_slot().set(Some(probe)).ok();
                Some(sink)
            }
            Err(e) => {
                eprintln!("media-app: audio off ({e}) — sigo sin sonido");
                audio_probe_slot().set(None).ok();
                None
            }
        }
    } else {
        apply_startup_config();
        audio_probe_slot().set(None).ok();
        None
    };

    llimphi_ui::run::<modelo::MediaApp>();
}

#[cfg(test)]
mod tests {
    use super::is_network_url;
    use media_core::SubtitleTrack;
    use std::path::{Path, PathBuf};

    #[test]
    fn sidecar_usa_el_nombre_base_del_video() {
        let cands = SubtitleTrack::sidecar_candidates(Path::new("/cine/peli.mp4"));
        assert_eq!(
            cands,
            vec![
                PathBuf::from("/cine/peli.srt"),
                PathBuf::from("/cine/peli.vtt"),
                PathBuf::from("/cine/peli.ass"),
                PathBuf::from("/cine/peli.ssa"),
            ]
        );
        assert_eq!(
            SubtitleTrack::sidecar_candidates(Path::new("clip"))[0],
            PathBuf::from("clip.srt")
        );
    }

    #[test]
    fn reconoce_urls_de_red() {
        for u in [
            "http://host/a.mp4",
            "https://host/stream.m3u8",
            "rtsp://cam.local/live",
            "rtmp://srv/app/key",
            "udp://239.0.0.1:1234",
            "srt://host:9000",
        ] {
            assert!(is_network_url(u), "debería ser URL: {u}");
        }
    }

    #[test]
    fn rechaza_paths_locales_y_file() {
        for p in [
            "/ruta/al/clip.mp4",
            "clip.mkv",
            "./rel/foto.png",
            "C:\\videos\\x.avi",
            "file:///home/u/v.mp4",
            "espacio raro://no",
        ] {
            assert!(!is_network_url(p), "no debería ser URL de red: {p}");
        }
    }
}
