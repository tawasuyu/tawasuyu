//! media-mux-webm — muxer **WebM/Matroska nativo** que empaqueta los
//! formatos nativos de tawasuyu en un solo contenedor.
//!
//! La **contraparte** de [`media_source_webm`]: ese crate *demuxea* un
//! `.webm` AV1+Opus en sus tracks; este lo *produce*. Cierra el ciclo de
//! producción del camino nativo (PLAN.md §6.quinquies) **sin tocar
//! ffmpeg**: tawasuyu encodea AV1 ([`media_encode_av1`]), muxea acá, y el
//! mismo `.webm` se reproduce 100% puro-Rust por el demuxer nativo.
//!
//! El contenedor WebM es un subconjunto acotado de EBML (Matroska); como
//! con el muxer IVF de `media-encode-av1`, lo escribimos byte a byte sin
//! depender de ninguna librería de mux. Estrategia: cada elemento se
//! serializa a un `Vec<u8>` y el padre lo envuelve con su tamaño ya
//! conocido (sin "unknown size") — el archivo queda seekable y honesto.
//!
//! ```no_run
//! use media_mux_webm::{WebmMuxConfig, mux_webm_file};
//!
//! let cfg = WebmMuxConfig { width: 320, height: 240, fps_num: 30, fps_den: 1 };
//! let video_packets: Vec<Vec<u8>> = vec![/* cada uno un frame AV1 (OBUs) */];
//! mux_webm_file("salida.webm", &cfg, &video_packets, None)?;
//! # Ok::<(), std::io::Error>(())
//! ```

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

// ─── IDs de elementos EBML/Matroska ────────────────────────────────────────
//
// Cada ID se escribe tal cual (ya lleva su propio marcador de longitud en el
// byte alto). Valores canónicos del spec Matroska/WebM.

// Cabecera EBML.
const ID_EBML: u32 = 0x1A45_DFA3;
const ID_EBML_VERSION: u32 = 0x4286;
const ID_EBML_READ_VERSION: u32 = 0x42F7;
const ID_EBML_MAX_ID_LENGTH: u32 = 0x42F2;
const ID_EBML_MAX_SIZE_LENGTH: u32 = 0x42F3;
const ID_DOC_TYPE: u32 = 0x4282;
const ID_DOC_TYPE_VERSION: u32 = 0x4287;
const ID_DOC_TYPE_READ_VERSION: u32 = 0x4285;

// Segment y sus hijos de primer nivel.
const ID_SEGMENT: u32 = 0x1853_8067;
const ID_INFO: u32 = 0x1549_A966;
const ID_TIMESTAMP_SCALE: u32 = 0x2AD7_B1;
const ID_DURATION: u32 = 0x4489;
const ID_MUXING_APP: u32 = 0x4D80;
const ID_WRITING_APP: u32 = 0x5741;

const ID_TRACKS: u32 = 0x1654_AE6B;
const ID_TRACK_ENTRY: u32 = 0xAE;
const ID_TRACK_NUMBER: u32 = 0xD7;
const ID_TRACK_UID: u32 = 0x73C5;
const ID_TRACK_TYPE: u32 = 0x83;
const ID_FLAG_LACING: u32 = 0x9C;
const ID_CODEC_ID: u32 = 0x86;
const ID_CODEC_PRIVATE: u32 = 0x63A2;
const ID_DEFAULT_DURATION: u32 = 0x23E383;
const ID_VIDEO: u32 = 0xE0;
const ID_PIXEL_WIDTH: u32 = 0xB0;
const ID_PIXEL_HEIGHT: u32 = 0xBA;
const ID_AUDIO: u32 = 0xE1;
const ID_SAMPLING_FREQUENCY: u32 = 0xB5;
const ID_CHANNELS: u32 = 0x9F;

const ID_CLUSTER: u32 = 0x1F43_B675;
const ID_CLUSTER_TIMESTAMP: u32 = 0xE7;
const ID_SIMPLE_BLOCK: u32 = 0xA3;

// TimestampScale fijo: 1 ms por tick (1_000_000 ns). Simplifica todos los
// timestamps a milisegundos enteros.
const TS_SCALE_NS: u64 = 1_000_000;

// Número de track de cada stream. El video siempre es 1; el audio 2.
const TRACK_VIDEO: u64 = 1;
const TRACK_AUDIO: u64 = 2;

// Un SimpleBlock guarda su timestamp como i16 relativo al cluster (±32767
// ms). Cuando el siguiente bloque excedería ese rango abrimos un cluster
// nuevo con base en su propio timestamp.
const CLUSTER_SPAN_MS: i64 = 30_000;

/// Parámetros del track de video AV1.
#[derive(Debug, Clone)]
pub struct WebmMuxConfig {
    pub width: u32,
    pub height: u32,
    /// Numerador del framerate (30 para 30 fps con `fps_den = 1`).
    pub fps_num: u32,
    /// Denominador del framerate (1, o 1001 para 29.97).
    pub fps_den: u32,
}

/// Track de audio Opus a incluir junto al video.
#[derive(Debug, Clone)]
pub struct OpusTrack {
    /// `OpusHead` que va como `CodecPrivate` del track (lo que el demuxer
    /// lee para reconstruir el decoder — ver [`media_source_webm`]).
    pub head: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u8,
    /// Muestras por paquete (960 = 20 ms @ 48 kHz, el default de Opus).
    /// Define el timestamp de cada paquete sobre el eje común.
    pub samples_per_packet: u32,
    /// Paquetes Opus crudos, en orden de presentación.
    pub packets: Vec<Vec<u8>>,
}

// ─── Serialización EBML de bajo nivel ──────────────────────────────────────

/// Codifica un entero como VINT de tamaño EBML (el campo de longitud que va
/// tras cada ID). Elige la menor cantidad de bytes que lo representa.
fn vint_size(value: u64) -> Vec<u8> {
    let mut length = 1usize;
    // El valor todo-unos de cada longitud está reservado ("unknown size"),
    // por eso `>=` y no `>`.
    while length < 8 && value >= (1u64 << (7 * length)) - 1 {
        length += 1;
    }
    let marker = 1u64 << (7 * length);
    let v = value | marker;
    v.to_be_bytes()[8 - length..].to_vec()
}

/// Escribe el ID (bytes significativos, big-endian) tal cual.
fn push_id(out: &mut Vec<u8>, id: u32) {
    let be = id.to_be_bytes();
    let first = be.iter().position(|&b| b != 0).unwrap_or(3);
    out.extend_from_slice(&be[first..]);
}

/// Elemento maestro o de datos: ID + VINT(tamaño) + payload.
fn elem(id: u32, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    push_id(&mut out, id);
    out.extend_from_slice(&vint_size(data.len() as u64));
    out.extend_from_slice(data);
    out
}

/// Elemento uint con los bytes big-endian mínimos (al menos 1).
fn elem_uint(id: u32, value: u64) -> Vec<u8> {
    let be = value.to_be_bytes();
    let first = be.iter().position(|&b| b != 0).unwrap_or(7);
    elem(id, &be[first..])
}

/// Elemento float de 64 bits (big-endian), como pide Matroska.
fn elem_f64(id: u32, value: f64) -> Vec<u8> {
    elem(id, &value.to_bits().to_be_bytes())
}

// ─── Construcción del Segment ──────────────────────────────────────────────

fn build_ebml_header() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(elem_uint(ID_EBML_VERSION, 1));
    body.extend(elem_uint(ID_EBML_READ_VERSION, 1));
    body.extend(elem_uint(ID_EBML_MAX_ID_LENGTH, 4));
    body.extend(elem_uint(ID_EBML_MAX_SIZE_LENGTH, 8));
    body.extend(elem(ID_DOC_TYPE, b"webm"));
    // WebM acepta AV1 desde DocTypeVersion 4 en la práctica.
    body.extend(elem_uint(ID_DOC_TYPE_VERSION, 4));
    body.extend(elem_uint(ID_DOC_TYPE_READ_VERSION, 2));
    elem(ID_EBML, &body)
}

fn build_info(duration_ms: f64) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(elem_uint(ID_TIMESTAMP_SCALE, TS_SCALE_NS));
    body.extend(elem(ID_MUXING_APP, b"tawasuyu/media-mux-webm"));
    body.extend(elem(ID_WRITING_APP, b"tawasuyu/media-mux-webm"));
    body.extend(elem_f64(ID_DURATION, duration_ms));
    elem(ID_INFO, &body)
}

fn build_tracks(cfg: &WebmMuxConfig, audio: Option<&OpusTrack>) -> Vec<u8> {
    let mut tracks = Vec::new();

    // Track de video AV1.
    let mut v = Vec::new();
    v.extend(elem_uint(ID_TRACK_NUMBER, TRACK_VIDEO));
    v.extend(elem_uint(ID_TRACK_UID, TRACK_VIDEO));
    v.extend(elem_uint(ID_TRACK_TYPE, 1)); // 1 = video
    v.extend(elem_uint(ID_FLAG_LACING, 0));
    v.extend(elem(ID_CODEC_ID, b"V_AV1"));
    // default_duration = ns por frame → el demuxer deriva el fps.
    let ns_per_frame = if cfg.fps_num > 0 {
        (1_000_000_000u64 * cfg.fps_den.max(1) as u64) / cfg.fps_num as u64
    } else {
        0
    };
    if ns_per_frame > 0 {
        v.extend(elem_uint(ID_DEFAULT_DURATION, ns_per_frame));
    }
    let mut vinfo = Vec::new();
    vinfo.extend(elem_uint(ID_PIXEL_WIDTH, cfg.width as u64));
    vinfo.extend(elem_uint(ID_PIXEL_HEIGHT, cfg.height as u64));
    v.extend(elem(ID_VIDEO, &vinfo));
    tracks.extend(elem(ID_TRACK_ENTRY, &v));

    // Track de audio Opus (opcional).
    if let Some(a) = audio {
        let mut au = Vec::new();
        au.extend(elem_uint(ID_TRACK_NUMBER, TRACK_AUDIO));
        au.extend(elem_uint(ID_TRACK_UID, TRACK_AUDIO));
        au.extend(elem_uint(ID_TRACK_TYPE, 2)); // 2 = audio
        au.extend(elem_uint(ID_FLAG_LACING, 0));
        au.extend(elem(ID_CODEC_ID, b"A_OPUS"));
        au.extend(elem(ID_CODEC_PRIVATE, &a.head));
        let mut ainfo = Vec::new();
        ainfo.extend(elem_f64(ID_SAMPLING_FREQUENCY, a.sample_rate as f64));
        ainfo.extend(elem_uint(ID_CHANNELS, a.channels.max(1) as u64));
        au.extend(elem(ID_AUDIO, &ainfo));
        tracks.extend(elem(ID_TRACK_ENTRY, &au));
    }

    elem(ID_TRACKS, &tracks)
}

/// Un bloque del eje común: track, timestamp absoluto en ms y si es keyframe.
struct Block<'a> {
    track: u64,
    ts_ms: i64,
    keyframe: bool,
    data: &'a [u8],
}

/// Cuerpo de un SimpleBlock: VINT(track) + i16(ts relativo) + flags + datos.
fn simple_block(track: u64, rel_ts: i16, keyframe: bool, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(data.len() + 8);
    body.extend(vint_size(track));
    body.extend_from_slice(&rel_ts.to_be_bytes());
    body.push(if keyframe { 0x80 } else { 0x00 });
    body.extend_from_slice(data);
    elem(ID_SIMPLE_BLOCK, &body)
}

/// Agrupa los bloques (ya ordenados por timestamp) en clusters, abriendo uno
/// nuevo cuando el offset relativo excedería el rango i16 seguro.
fn build_clusters(blocks: &[Block]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        let base = blocks[i].ts_ms;
        let mut body = Vec::new();
        body.extend(elem_uint(ID_CLUSTER_TIMESTAMP, base.max(0) as u64));
        while i < blocks.len() && blocks[i].ts_ms - base <= CLUSTER_SPAN_MS {
            let b = &blocks[i];
            let rel = (b.ts_ms - base) as i16;
            body.extend(simple_block(b.track, rel, b.keyframe, b.data));
            i += 1;
        }
        out.extend(elem(ID_CLUSTER, &body));
    }
    out
}

/// Mezcla los paquetes de video y audio en un único eje de timestamps (ms),
/// ordenados estable por tiempo. El video usa el framerate; el audio, las
/// muestras por paquete.
fn merge_timeline<'a>(
    cfg: &WebmMuxConfig,
    video: &'a [Vec<u8>],
    audio: Option<&'a OpusTrack>,
) -> (Vec<Block<'a>>, f64) {
    let mut blocks: Vec<Block> = Vec::with_capacity(video.len());
    let mut end_ms = 0i64;

    let fps = if cfg.fps_den > 0 {
        cfg.fps_num as f64 / cfg.fps_den as f64
    } else {
        0.0
    };
    let frame_ms = if fps > 0.0 { 1000.0 / fps } else { 0.0 };
    for (i, pkt) in video.iter().enumerate() {
        let ts = (i as f64 * frame_ms).round() as i64;
        blocks.push(Block {
            track: TRACK_VIDEO,
            ts_ms: ts,
            // El primer frame AV1 es keyframe; el resto los tratamos como
            // inter (el flag no afecta al decode por OBU, sólo al seek).
            keyframe: i == 0,
            data: pkt,
        });
        end_ms = end_ms.max(ts + frame_ms.round() as i64);
    }

    if let Some(a) = audio {
        let sr = a.sample_rate.max(1) as f64;
        let pkt_ms = a.samples_per_packet as f64 * 1000.0 / sr;
        for (j, pkt) in a.packets.iter().enumerate() {
            let ts = (j as f64 * pkt_ms).round() as i64;
            blocks.push(Block {
                track: TRACK_AUDIO,
                ts_ms: ts,
                keyframe: true, // todo paquete Opus es decodable por sí mismo
                data: pkt,
            });
            end_ms = end_ms.max(ts + pkt_ms.round() as i64);
        }
    }

    // Orden estable por timestamp (mantiene video antes que audio a igual ms).
    blocks.sort_by_key(|b| b.ts_ms);
    (blocks, end_ms as f64)
}

// ─── API pública ───────────────────────────────────────────────────────────

/// Escribe un `.webm` completo a `w` desde paquetes AV1 (orden de
/// presentación) y, opcionalmente, un track Opus.
pub fn mux_webm<W: Write>(
    mut w: W,
    cfg: &WebmMuxConfig,
    video_packets: &[Vec<u8>],
    audio: Option<&OpusTrack>,
) -> io::Result<()> {
    let (blocks, duration_ms) = merge_timeline(cfg, video_packets, audio);

    // El Segment lleva su tamaño ya conocido: serializamos su cuerpo entero
    // y lo envolvemos. Para archivos cortos/medianos es lo más simple y deja
    // el contenedor seekable.
    let mut segment_body = Vec::new();
    segment_body.extend(build_info(duration_ms));
    segment_body.extend(build_tracks(cfg, audio));
    segment_body.extend(build_clusters(&blocks));

    w.write_all(&build_ebml_header())?;
    w.write_all(&elem(ID_SEGMENT, &segment_body))?;
    w.flush()
}

/// Conveniencia: escribe el `.webm` a `path`.
pub fn mux_webm_file(
    path: impl AsRef<Path>,
    cfg: &WebmMuxConfig,
    video_packets: &[Vec<u8>],
    audio: Option<&OpusTrack>,
) -> io::Result<()> {
    let f = BufWriter::new(File::create(path)?);
    mux_webm(f, cfg, video_packets, audio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vint_size_lengths() {
        // 1 byte: el marcador 0x80 más el valor.
        assert_eq!(vint_size(0), vec![0x80]);
        assert_eq!(vint_size(1), vec![0x81]);
        // 126 todavía cabe en 1 byte (127 = todo-unos, reservado).
        assert_eq!(vint_size(126), vec![0xFE]);
        // 127 ya necesita 2 bytes: 0x4000 | 127.
        assert_eq!(vint_size(127), vec![0x40, 0x7F]);
        // 1_000_000 cabe en 3 bytes con marcador 0x20.
        let v = vint_size(1_000_000);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0] & 0xE0, 0x20);
    }

    #[test]
    fn push_id_strips_leading_zeros() {
        let mut out = Vec::new();
        push_id(&mut out, ID_SIMPLE_BLOCK); // 0xA3, un byte
        assert_eq!(out, vec![0xA3]);
        out.clear();
        push_id(&mut out, ID_SEGMENT); // 0x18538067, cuatro bytes
        assert_eq!(out, vec![0x18, 0x53, 0x80, 0x67]);
    }

    #[test]
    fn elem_uint_minimal_bytes() {
        // TrackType=1 → ID 0x83, tamaño 1, dato 0x01.
        assert_eq!(elem_uint(ID_TRACK_TYPE, 1), vec![0x83, 0x81, 0x01]);
    }

    #[test]
    fn header_is_well_formed() {
        let h = build_ebml_header();
        // Arranca con el ID EBML.
        assert_eq!(&h[0..4], &[0x1A, 0x45, 0xDF, 0xA3]);
        // Contiene el DocType "webm".
        let win = h.windows(4).any(|w| w == b"webm");
        assert!(win, "el header debe declarar DocType webm");
    }

    #[test]
    fn timeline_orders_and_durates() {
        let cfg = WebmMuxConfig {
            width: 64,
            height: 48,
            fps_num: 10,
            fps_den: 1,
        };
        // 3 frames de video @ 10fps → ts 0, 100, 200 ms.
        let video = vec![vec![1u8], vec![2u8], vec![3u8]];
        let (blocks, dur) = merge_timeline(&cfg, &video, None);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].ts_ms, 0);
        assert_eq!(blocks[1].ts_ms, 100);
        assert_eq!(blocks[2].ts_ms, 200);
        assert!(blocks[0].keyframe && !blocks[1].keyframe);
        assert_eq!(dur, 300.0); // 200 + 100 ms del último frame
    }
}
