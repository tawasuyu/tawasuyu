//! El loop estrella del crate, sin hardware: un productor en vivo
//! (aquí frames sintéticos vía [`LiveSink`], en producción el hilo de
//! la cámara v4l2) alimenta a `media-recorder-webm` y produce un
//! `.webm` AV1+Opus nativo. Verifica que `LiveSource` cumple el
//! contrato de `FrameSource` que el recorder espera.

use std::time::Duration;

use media_core::FrameSource;
use media_recorder_webm::{RecordedFrameSource, WebmRecorder, WebmRecorderSettings};
use media_source_capture::live_channel;

#[test]
fn live_source_alimenta_recorder_webm() {
    let (sink, source) = live_channel();
    let rec = WebmRecorder::with_settings(WebmRecorderSettings {
        // speed máximo / pocas dimensiones → encode rápido en CI.
        speed: 10,
        ..Default::default()
    });
    let mut recorded = RecordedFrameSource::new(source, rec.clone());
    let mut buf = Vec::new();

    let (w, h) = (48u32, 32u32);
    let frame = |v: u8| {
        let mut f = vec![0u8; (w * h * 4) as usize];
        for px in f.chunks_exact_mut(4) {
            px.copy_from_slice(&[v, 255 - v, v / 2, 255]);
        }
        f
    };

    // Primer frame: ceba las dimensiones del recorder (aún sin grabar).
    sink.push_rgba(w, h, frame(0));
    assert_eq!(
        recorded.tick(Duration::from_millis(33), &mut buf),
        Some((w, h))
    );
    assert_eq!(rec.last_dimensions(), (w, h));

    // Armar y grabar unos frames.
    let dir = std::env::temp_dir();
    let path = dir.join("media-capture-test.webm");
    let _ = std::fs::remove_file(&path);
    rec.start(&path).expect("start");

    for i in 1..=5u8 {
        sink.push_rgba(w, h, frame(i * 40));
        assert_eq!(
            recorded.tick(Duration::from_millis(33), &mut buf),
            Some((w, h))
        );
    }

    let (out, summary) = rec.stop().expect("stop");
    assert_eq!(out, path);
    assert!(summary.video_frames >= 1, "se grabó al menos un frame");
    assert_eq!(summary.audio_packets, 0, "sin audio en este test");

    // El archivo existe y arranca con el magic EBML de Matroska/WebM.
    let bytes = std::fs::read(&path).expect("leer .webm");
    assert!(bytes.len() > 32);
    assert_eq!(&bytes[..4], &[0x1A, 0x45, 0xDF, 0xA3], "header EBML");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn frame_estancado_no_se_re_emite() {
    // Garantía clave para el bucle de render: sin frame nuevo, el
    // recorder no recibe duplicados (no infla video_frames a fps de
    // pantalla cuando la cámara va más lenta).
    let (sink, mut source) = live_channel();
    let mut buf = Vec::new();
    sink.push_rgba(2, 2, vec![7u8; 16]);
    assert_eq!(source.tick(Duration::ZERO, &mut buf), Some((2, 2)));
    // Tres ticks más sin push: ninguno emite.
    for _ in 0..3 {
        assert_eq!(source.tick(Duration::ZERO, &mut buf), None);
    }
}
