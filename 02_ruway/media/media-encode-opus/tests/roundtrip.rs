//! Round-trip del audio nativo, sin C ni ffmpeg:
//!
//!   PCM f32 → media-encode-opus → paquetes Opus + OpusHead
//!                                → media-source-opus (opus-wave) → PCM
//!
//! y el ciclo completo del contenedor nativo:
//!
//!   AV1 + Opus → media-mux-webm (.webm) → media-source-webm → AV1 + Opus

use media_core::AudioSource;
use media_encode_opus::{encode_to_opus_track, OpusEncoderConfig};
use media_source_opus::OpusSource;

/// 200 ms de tono A4, mono @ 48 kHz, en [-1, 1].
fn tono(samples: usize) -> Vec<f32> {
    (0..samples)
        .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin() * 0.5)
        .collect()
}

#[test]
fn opus_encode_decode_round_trip_nativo() {
    let pcm = tono(9600); // 200 ms
    let cfg = OpusEncoderConfig {
        sample_rate: 48_000,
        channels: 1,
        ..Default::default()
    };
    let (head, packets, spp) = encode_to_opus_track(cfg, &pcm).unwrap();
    assert_eq!(spp, 960);
    assert_eq!(packets.len(), 10, "200 ms / 20 ms = 10 paquetes");

    // Decode con el decoder nativo (opus-wave detrás de media-source-opus).
    let mut src = OpusSource::from_opus_packets(&head, &packets).unwrap();
    assert_eq!(src.source_channels(), 1);

    let mut out = vec![0f32; 48_000]; // 1 s, sobra
    src.fill(&mut out, 48_000, 1);
    let energetic = out.iter().filter(|s| s.abs() > 0.01).count();
    assert!(
        energetic > 500,
        "el Opus decodificado debe traer el tono de vuelta (fue {energetic})"
    );
}

#[test]
fn webm_av1_mas_opus_propio_round_trip() {
    use media_core::FrameSource;
    use media_encode_av1::{Av1Encoder, Av1EncoderConfig};
    use media_mux_webm::{mux_webm_file, OpusTrack, WebmMuxConfig};
    use media_source_webm::WebmMedia;
    use std::time::Duration;

    const W: u32 = 64;
    const H: u32 = 48;

    // Video AV1 nativo: 6 frames de color sólido.
    let acfg = Av1EncoderConfig {
        width: W,
        height: H,
        fps_num: 10,
        fps_den: 1,
        speed: 10,
        ..Default::default()
    };
    let frame = vec![120u8; (W * H * 4) as usize];
    let mut venc = Av1Encoder::new(acfg).unwrap();
    let mut video = Vec::new();
    for _ in 0..6 {
        for p in venc.encode_rgba(&frame).unwrap() {
            video.push(p.data);
        }
    }
    for p in venc.finish().unwrap() {
        video.push(p.data);
    }

    // Audio Opus nativo: ~600 ms de tono, alineado con el video (6 frames @ 10fps).
    let pcm = tono(28_800);
    let (head, opus_packets, spp) = encode_to_opus_track(
        OpusEncoderConfig {
            sample_rate: 48_000,
            channels: 1,
            ..Default::default()
        },
        &pcm,
    )
    .unwrap();

    let cfg = WebmMuxConfig {
        width: W,
        height: H,
        fps_num: 10,
        fps_den: 1,
    };
    let audio = OpusTrack {
        head,
        sample_rate: 48_000,
        channels: 1,
        samples_per_packet: spp,
        packets: opus_packets,
    };
    let path = std::env::temp_dir().join("media_webm_av1_opus_propio.webm");
    mux_webm_file(&path, &cfg, &video, Some(&audio)).unwrap();

    // Demux + decode nativo de AMBOS tracks: el .webm es 100% tawasuyu.
    let mut media = WebmMedia::open(&path).unwrap();
    assert_eq!((media.width, media.height), (W, H));

    let mut vsrc = media.video.take().expect("track AV1 propio");
    let mut vbuf = Vec::new();
    assert_eq!(
        vsrc.tick(Duration::from_secs(1), &mut vbuf),
        Some((W, H)),
        "el AV1 que muxeamos debe decodificar"
    );

    let mut asrc = media.audio.take().expect("track Opus propio");
    let mut abuf = vec![0f32; 48_000];
    asrc.fill(&mut abuf, 48_000, 1);
    let energetic = abuf.iter().filter(|s| s.abs() > 0.01).count();
    assert!(energetic > 500, "el Opus propio debe traer señal (fue {energetic})");

    let _ = std::fs::remove_file(&path);
}
