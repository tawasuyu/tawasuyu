//! Round-trip de grabación: tee de un FrameSource → `.ivf` AV1 → decode
//! con `media-source-av1`. Prueba que el camino de captura de video cierra
//! sin ffmpeg, igual que el round-trip de `media-encode-av1`.

use std::time::Duration;

use media_core::FrameSource;
use media_recorder_av1::{Av1Recorder, Av1RecorderSettings, RecordedFrameSource};
use media_source_av1::Av1VideoSource;

struct SolidSource {
    w: u32,
    h: u32,
    rgb: (u8, u8, u8),
}

impl FrameSource for SolidSource {
    fn tick(&mut self, _dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        buf.resize((self.w * self.h * 4) as usize, 0);
        for px in buf.chunks_exact_mut(4) {
            px[0] = self.rgb.0;
            px[1] = self.rgb.1;
            px[2] = self.rgb.2;
            px[3] = 255;
        }
        Some((self.w, self.h))
    }
}

#[test]
fn record_then_decode_preserves_color() {
    let (w, h) = (80u32, 64u32);
    let (r, g, b) = (40u8, 200u8, 90u8);

    let rec = Av1Recorder::with_settings(Av1RecorderSettings {
        fps_num: 30,
        fps_den: 1,
        quantizer: 20, // alta calidad para que el color sólido sobreviva
        speed: 10,     // rápido para el test
    });
    let mut src = RecordedFrameSource::new(SolidSource { w, h, rgb: (r, g, b) }, rec.clone());

    let dt = Duration::from_millis(33);
    // Un frame para descubrir dimensiones, luego armamos.
    src.tick(dt, &mut Vec::new());
    assert_eq!(rec.last_dimensions(), (w, h));

    let path = std::env::temp_dir().join("media_recorder_av1_roundtrip.ivf");
    let _ = std::fs::remove_file(&path);
    rec.start(&path).expect("armar recorder");
    assert!(rec.is_recording());

    let mut buf = Vec::new();
    for _ in 0..8 {
        src.tick(dt, &mut buf);
    }
    let (closed, n) = rec.stop().expect("stop + escribir IVF");
    assert_eq!(closed, path);
    assert_eq!(n, 8, "8 frames grabados → 8 paquetes");
    assert!(!rec.is_recording());
    assert_eq!(rec.dropped_frames(), 0);

    // Decode con el decoder nativo.
    let mut dec = Av1VideoSource::open(&path).expect("abrir IVF grabado");
    assert_eq!(dec.dimensions(), (w, h));
    let mut out = Vec::new();
    let dims = dec.tick(Duration::from_secs(1), &mut out);
    assert_eq!(dims, Some((w, h)));

    let i = ((h as usize / 2) * w as usize + (w as usize / 2)) * 4;
    let (dr, dg, db) = (out[i] as i32, out[i + 1] as i32, out[i + 2] as i32);
    let tol = 16;
    assert!(
        (dr - r as i32).abs() <= tol
            && (dg - g as i32).abs() <= tol
            && (db - b as i32).abs() <= tol,
        "color grabado dentro de ±{tol}: esperaba ({r},{g},{b}), fue ({dr},{dg},{db})"
    );

    let _ = std::fs::remove_file(&path);
}
