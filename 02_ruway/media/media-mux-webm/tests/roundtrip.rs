//! Round-trip real del camino nativo, sin ffmpeg en ningún extremo:
//!
//!   frames RGBA → media-encode-av1 → media-mux-webm (.webm)
//!                                   → media-source-webm (demux) → rav1d → RGBA
//!
//! Verifica que el `.webm` que produce tawasuyu lo lee tawasuyu: dimensiones,
//! número de frames y que el primer frame decodifica al tamaño correcto.

use std::io::BufReader;
use std::time::Duration;

use matroska_demuxer::{MatroskaFile, TrackType};
use media_core::FrameSource;
use media_encode_av1::{Av1Encoder, Av1EncoderConfig};
use media_mux_webm::{mux_webm_file, OpusTrack, WebmMuxConfig};
use media_source_webm::WebmMedia;

const W: u32 = 64;
const H: u32 = 48;
const N: usize = 6;

/// Encodea N frames de color sólido y devuelve los paquetes AV1 crudos.
fn encode_video() -> Vec<Vec<u8>> {
    let cfg = Av1EncoderConfig {
        width: W,
        height: H,
        fps_num: 10,
        fps_den: 1,
        speed: 10,
        ..Default::default()
    };
    let frame = vec![100u8; (W * H * 4) as usize];
    let mut enc = Av1Encoder::new(cfg).unwrap();
    let mut packets = Vec::new();
    for _ in 0..N {
        for p in enc.encode_rgba(&frame).unwrap() {
            packets.push(p.data);
        }
    }
    for p in enc.finish().unwrap() {
        packets.push(p.data);
    }
    assert_eq!(packets.len(), N, "N frames in → N paquetes out");
    packets
}

#[test]
fn webm_solo_video_round_trip_nativo() {
    let video = encode_video();
    let cfg = WebmMuxConfig {
        width: W,
        height: H,
        fps_num: 10,
        fps_den: 1,
    };
    let path = std::env::temp_dir().join("media_mux_webm_video.webm");
    mux_webm_file(&path, &cfg, &video, None).unwrap();

    // Lo lee el demuxer nativo completo (matroska-demuxer + decoders).
    let mut media = WebmMedia::open(&path).unwrap();
    assert_eq!((media.width, media.height), (W, H));
    assert!(media.audio.is_none(), "no muxeamos audio");

    let mut vsrc = media.video.take().expect("track AV1 presente");
    let mut buf = Vec::new();
    // El FrameSource emite un frame cada 1/fps; un tick de 1s sobra para
    // sacar el primero (igual que el test del fixture en media-source-webm).
    let dims = vsrc.tick(Duration::from_secs(1), &mut buf);
    assert_eq!(dims, Some((W, H)), "el primer frame debe decodificar");
    assert_eq!(buf.len(), (W * H * 4) as usize);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn webm_con_audio_declara_track_opus() {
    // No encodeamos Opus de verdad (no hay encoder nativo todavía); basta con
    // verificar que el muxer escribe el track A_OPUS con su CodecPrivate y que
    // el demuxer lo lee de vuelta intacto.
    let video = encode_video();
    let cfg = WebmMuxConfig {
        width: W,
        height: H,
        fps_num: 10,
        fps_den: 1,
    };
    // OpusHead mínimo válido (19 bytes): "OpusHead" + versión + canales + ...
    let head = {
        let mut h = b"OpusHead".to_vec();
        h.push(1); // versión
        h.push(2); // canales
        h.extend_from_slice(&3840u16.to_le_bytes()); // pre-skip
        h.extend_from_slice(&48_000u32.to_le_bytes()); // sample rate original
        h.extend_from_slice(&0i16.to_le_bytes()); // output gain
        h.push(0); // channel mapping family
        h
    };
    let audio = OpusTrack {
        head: head.clone(),
        sample_rate: 48_000,
        channels: 2,
        samples_per_packet: 960,
        packets: vec![vec![0xFCu8; 4], vec![0xFCu8; 4], vec![0xFCu8; 4]],
    };
    let path = std::env::temp_dir().join("media_mux_webm_av.webm");
    mux_webm_file(&path, &cfg, &video, Some(&audio)).unwrap();

    // Demux crudo: confirmamos los dos tracks y el codec_private del audio.
    let file = BufReader::new(std::fs::File::open(&path).unwrap());
    let mkv = MatroskaFile::open(file).unwrap();
    let mut vio = false;
    let mut aud = false;
    for t in mkv.tracks() {
        match (t.track_type(), t.codec_id()) {
            (TrackType::Video, "V_AV1") => {
                vio = true;
                let v = t.video().unwrap();
                assert_eq!(v.pixel_width().get() as u32, W);
                assert_eq!(v.pixel_height().get() as u32, H);
            }
            (TrackType::Audio, "A_OPUS") => {
                aud = true;
                assert_eq!(
                    t.codec_private().expect("OpusHead presente"),
                    head.as_slice(),
                    "el CodecPrivate debe volver intacto"
                );
            }
            _ => {}
        }
    }
    assert!(vio && aud, "ambos tracks deben estar presentes");

    let _ = std::fs::remove_file(&path);
}
