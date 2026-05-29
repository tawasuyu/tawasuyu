//! media-source-webm — demux **Matroska/WebM nativo** que une los
//! decoders nativos de gioser.
//!
//! Cierra el último eslabón del camino nativo (PLAN.md §6.quinquies): un
//! `.webm`/`.mkv` con video **AV1** + audio **Opus** se reproduce 100%
//! puro-Rust, sin tocar ffmpeg. El demux EBML lo hace `matroska-demuxer`;
//! los paquetes del track `V_AV1` van a [`media_source_av1::Av1VideoSource`]
//! y los del track `A_OPUS` a [`media_source_opus::OpusSource`].
//!
//! Estrategia: demuxea el archivo entero una vez, separa los paquetes por
//! track y construye ambas fuentes desde memoria. Mismo trade-off de RAM
//! que el resto del dominio (los paquetes comprimidos, no los frames
//! decodificados). Codecs ajenos (H.264/AAC en MKV) no entran acá — para
//! eso está `shared/foreign-av`.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use matroska_demuxer::{Frame, MatroskaFile, TrackType};
use media_source_av1::Av1VideoSource;
use media_source_opus::OpusSource;

#[derive(Debug)]
pub enum WebmError {
    Io(std::io::Error),
    Demux(String),
    /// No hay ningún track AV1 ni Opus que podamos decodificar nativo.
    SinTracksNativos,
    Av1(String),
    Opus(String),
}

impl std::fmt::Display for WebmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Demux(e) => write!(f, "demux mkv/webm: {e}"),
            Self::SinTracksNativos => write!(
                f,
                "el archivo no tiene track V_AV1 ni A_OPUS (¿códec ajeno? usá foreign-av)"
            ),
            Self::Av1(e) => write!(f, "av1: {e}"),
            Self::Opus(e) => write!(f, "opus: {e}"),
        }
    }
}

impl std::error::Error for WebmError {}

impl From<std::io::Error> for WebmError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Resultado del demux: las fuentes nativas listas + metadata del
/// contenedor. `video`/`audio` son `None` si el track correspondiente no
/// existe o no es AV1/Opus.
pub struct WebmMedia {
    pub video: Option<Av1VideoSource>,
    pub audio: Option<OpusSource>,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub duration: Option<Duration>,
}

impl WebmMedia {
    /// Abre y demuxea un `.webm`/`.mkv`, construyendo las fuentes nativas
    /// para los tracks AV1 y Opus que encuentre.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WebmError> {
        let file = BufReader::new(File::open(path.as_ref())?);
        let mut mkv = MatroskaFile::open(file).map_err(|e| WebmError::Demux(format!("{e:?}")))?;

        // Duración global: duration (en ticks) · timestamp_scale (ns/tick).
        let info = mkv.info();
        let ts_scale = info.timestamp_scale().get() as f64; // ns por tick
        let duration = info
            .duration()
            .map(|d| Duration::from_secs_f64(d * ts_scale / 1e9));

        // Identificar tracks AV1 (video) y Opus (audio).
        let mut video_track: Option<u64> = None;
        let mut audio_track: Option<u64> = None;
        let mut width = 0u32;
        let mut height = 0u32;
        let mut fps = 0f32;
        let mut opus_head: Option<Vec<u8>> = None;

        for t in mkv.tracks() {
            match (t.track_type(), t.codec_id()) {
                (TrackType::Video, "V_AV1") if video_track.is_none() => {
                    video_track = Some(t.track_number().get());
                    if let Some(v) = t.video() {
                        width = v.pixel_width().get() as u32;
                        height = v.pixel_height().get() as u32;
                    }
                    // default_duration = ns por frame → fps.
                    if let Some(dd) = t.default_duration() {
                        let ns = dd.get() as f64;
                        if ns > 0.0 {
                            fps = (1e9 / ns) as f32;
                        }
                    }
                }
                (TrackType::Audio, "A_OPUS") if audio_track.is_none() => {
                    audio_track = Some(t.track_number().get());
                    opus_head = t.codec_private().map(|b| b.to_vec());
                }
                _ => {}
            }
        }

        if video_track.is_none() && audio_track.is_none() {
            return Err(WebmError::SinTracksNativos);
        }

        // Recolectar paquetes por track en una sola pasada.
        let mut video_packets: Vec<Vec<u8>> = Vec::new();
        let mut audio_packets: Vec<Vec<u8>> = Vec::new();
        let mut frame = Frame::default();
        loop {
            match mkv.next_frame(&mut frame) {
                Ok(true) => {
                    if Some(frame.track) == video_track {
                        video_packets.push(std::mem::take(&mut frame.data));
                    } else if Some(frame.track) == audio_track {
                        audio_packets.push(std::mem::take(&mut frame.data));
                    }
                }
                Ok(false) => break,
                Err(e) => return Err(WebmError::Demux(format!("{e:?}"))),
            }
        }

        // Si el contenedor no declaró fps, estimarlo de frames/duración.
        if fps <= 0.0 {
            fps = match duration {
                Some(d) if d.as_secs_f32() > 0.0 && !video_packets.is_empty() => {
                    video_packets.len() as f32 / d.as_secs_f32()
                }
                _ => 30.0,
            };
        }

        let video = if video_track.is_some() && !video_packets.is_empty() {
            let n = video_packets.len() as u32;
            Some(
                Av1VideoSource::from_av1_packets(video_packets, width, height, fps, n)
                    .map_err(WebmError::Av1)?,
            )
        } else {
            None
        };

        let audio = match (audio_track, &opus_head) {
            (Some(_), Some(head)) if !audio_packets.is_empty() => Some(
                OpusSource::from_opus_packets(head, &audio_packets)
                    .map_err(|e| WebmError::Opus(e.to_string()))?,
            ),
            _ => None,
        };

        Ok(WebmMedia {
            video,
            audio,
            width,
            height,
            fps,
            duration,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_core::{AudioSource, FrameSource};

    fn fixture() -> std::path::PathBuf {
        let bytes = include_bytes!("../tests/fixtures/clip_av1_opus.webm");
        let path = std::env::temp_dir().join("media_webm_test_clip.webm");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn demuxea_y_decodifica_av1_y_opus_nativo() {
        let path = fixture();
        let mut media = WebmMedia::open(&path).unwrap();
        assert_eq!((media.width, media.height), (64, 48));

        // Video AV1 nativo: el primer tick produce un frame 64×48.
        let mut vsrc = media.video.take().expect("track AV1 presente");
        let mut buf = Vec::new();
        let dims = vsrc.tick(Duration::from_secs(1), &mut buf);
        assert_eq!(dims, Some((64, 48)), "AV1 del webm debería decodificar");
        assert_eq!(buf.len(), 64 * 48 * 4);

        // Audio Opus nativo: fill trae señal del tono.
        let mut asrc = media.audio.take().expect("track Opus presente");
        let mut abuf = vec![0f32; 2 * 1024];
        asrc.fill(&mut abuf, 48_000, 2);
        let energetic = abuf.iter().filter(|s| s.abs() > 0.01).count();
        assert!(energetic > 50, "el audio Opus debería traer señal");

        let _ = std::fs::remove_file(&path);
    }
}
