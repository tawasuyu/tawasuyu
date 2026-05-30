//! Round-trip del recorder unificado: enchufamos un FrameSource + un
//! AudioSource sintéticos a los wrappers, "grabamos" unos frames y bloques
//! de audio, y al stop() verificamos que el `.webm` resultante demuxea +
//! decodea ambos tracks nativamente — sin ffmpeg.

use std::time::Duration;

use media_core::{AudioSource, FrameSource};
use media_recorder_webm::{
    RecordedAudioSource, RecordedFrameSource, WebmRecorder, WebmRecorderSettings,
};
use media_source_webm::WebmMedia;

const W: u32 = 64;
const H: u32 = 48;
const FPS: u32 = 10;

/// Video: color sólido del tamaño dado.
struct SolidVideo;
impl FrameSource for SolidVideo {
    fn tick(&mut self, _dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        buf.resize((W * H * 4) as usize, 0);
        for px in buf.chunks_exact_mut(4) {
            px.copy_from_slice(&[200, 80, 40, 255]);
        }
        Some((W, H))
    }
}

/// Audio: tono A4 continuo, mono @ 48 kHz, generado por fase incremental.
struct ToneAudio {
    phase: f32,
}
impl AudioSource for ToneAudio {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        let step = 2.0 * std::f32::consts::PI * 440.0 / sample_rate as f32;
        let ch = channels.max(1) as usize;
        for frame in buf.chunks_mut(ch) {
            let s = self.phase.sin() * 0.5;
            for c in frame.iter_mut() {
                *c = s;
            }
            self.phase += step;
        }
    }
}

#[test]
fn graba_webm_av1_opus_y_se_reproduce_nativo() {
    let rec = WebmRecorder::with_settings(WebmRecorderSettings {
        fps_num: FPS,
        fps_den: 1,
        speed: 10,
        ..Default::default()
    });

    let mut vsrc = RecordedFrameSource::new(SolidVideo, rec.clone());
    let mut asrc = RecordedAudioSource::new(ToneAudio { phase: 0.0 }, rec.clone());

    // Un frame debe pasar antes de armar (descubre dimensiones).
    let mut vbuf = Vec::new();
    vsrc.tick(Duration::from_millis(100), &mut vbuf);

    let path = std::env::temp_dir().join("media_recorder_webm_test.webm");
    rec.start(&path).unwrap();
    assert!(rec.is_recording());

    // "Grabar" ~600 ms: 6 frames de video y bloques de audio que suman
    // ~600 ms @ 48 kHz mono (28800 muestras). Bloques de 4096 para forzar
    // el buffering por frames de Opus.
    for _ in 0..6 {
        vsrc.tick(Duration::from_millis(100), &mut vbuf);
    }
    let mut total_audio = 0;
    while total_audio < 28_800 {
        let mut ablk = vec![0f32; 4096];
        asrc.fill(&mut ablk, 48_000, 1);
        total_audio += ablk.len();
    }

    let (out, summary) = rec.stop().unwrap();
    assert_eq!(out, path);
    assert!(!rec.is_recording());
    assert_eq!(summary.video_frames, 6, "6 frames grabados");
    assert!(summary.audio_packets > 0, "se grabaron paquetes Opus");
    assert_eq!(summary.audio_sample_rate, 48_000);
    assert_eq!(summary.audio_channels, 1);

    // Reproducción nativa de ambos tracks.
    let mut media = WebmMedia::open(&path).unwrap();
    assert_eq!((media.width, media.height), (W, H));

    let mut v = media.video.take().expect("track AV1 grabado");
    let mut buf = Vec::new();
    assert_eq!(
        v.tick(Duration::from_secs(1), &mut buf),
        Some((W, H)),
        "el video grabado decodifica"
    );

    let mut a = media.audio.take().expect("track Opus grabado");
    let mut abuf = vec![0f32; 48_000];
    a.fill(&mut abuf, 48_000, 1);
    let energetic = abuf.iter().filter(|s| s.abs() > 0.01).count();
    assert!(energetic > 500, "el audio grabado trae el tono (fue {energetic})");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn audio_con_rate_no_opus_degrada_a_video_solo() {
    let rec = WebmRecorder::with_settings(WebmRecorderSettings {
        fps_num: FPS,
        fps_den: 1,
        speed: 10,
        ..Default::default()
    });
    let mut vsrc = RecordedFrameSource::new(SolidVideo, rec.clone());
    let mut asrc = RecordedAudioSource::new(ToneAudio { phase: 0.0 }, rec.clone());

    let mut vbuf = Vec::new();
    vsrc.tick(Duration::from_millis(100), &mut vbuf);
    let path = std::env::temp_dir().join("media_recorder_webm_degrade.webm");
    rec.start(&path).unwrap();
    for _ in 0..3 {
        vsrc.tick(Duration::from_millis(100), &mut vbuf);
    }
    // 44100 Hz no es un sample-rate Opus → audio se descarta, video sigue.
    let mut ablk = vec![0f32; 4096];
    asrc.fill(&mut ablk, 44_100, 2);

    let (_, summary) = rec.stop().unwrap();
    assert_eq!(summary.video_frames, 3);
    assert_eq!(summary.audio_packets, 0, "rate no-Opus → sin audio");

    let media = WebmMedia::open(&path).unwrap();
    assert!(media.video.is_some());
    assert!(media.audio.is_none(), "el .webm quedó video-solo");

    let _ = std::fs::remove_file(&path);
}
