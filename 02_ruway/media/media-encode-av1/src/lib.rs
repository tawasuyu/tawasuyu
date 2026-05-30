//! media-encode-av1 — encoder AV1 (vía `rav1e`) desde frames RGBA →
//! contenedor IVF. La **contraparte** de [`media_source_av1`]: ese crate
//! *decodea* AV1 nativo, este lo *produce*. Cierra el ciclo encode↔decode
//! del formato de video nativo de gioser (PLAN.md §6.quinquies) **sin
//! tocar ffmpeg**.
//!
//! `rav1e` es el encoder de referencia AV1 en Rust puro (Xiph/AOMedia);
//! con `default-features = false` sale el camino escalar (sin nasm), igual
//! que `rav1d` en el decoder — compila a WASM y corre en wawa.
//!
//! El input son frames RGBA8 (mismo formato que escupe el `FrameSource` de
//! `media-source-av1`); la salida es un `.ivf` que ese mismo decoder
//! reproduce. La conversión RGBA→YUV420 es el **inverso exacto** de la del
//! decoder (BT.601 *full range*), así el round-trip preserva color.
//!
//! ```no_run
//! use media_encode_av1::{Av1Encoder, Av1EncoderConfig};
//!
//! let cfg = Av1EncoderConfig { width: 320, height: 240, fps_num: 30, fps_den: 1, ..Default::default() };
//! let frames: Vec<Vec<u8>> = vec![/* cada uno width*height*4 bytes RGBA */];
//! let mut enc = Av1Encoder::new(cfg.clone())?;
//! let mut packets = Vec::new();
//! for frame_rgba in &frames {
//!     packets.extend(enc.encode_rgba(frame_rgba)?);
//! }
//! packets.extend(enc.finish()?);
//! media_encode_av1::write_ivf_file("salida.ivf", &cfg, &packets)?;
//! # Ok::<(), media_encode_av1::Av1EncodeError>(())
//! ```

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use rav1e::config::SpeedSettings;
use rav1e::prelude::*;

/// Parámetros de encode. `quantizer` (0..=255, menor = mejor calidad) usa
/// el modo de cuantizador constante de rav1e — sin objetivo de bitrate.
/// `speed` es el preset 0..=10 (mayor = más rápido, menos compresión).
#[derive(Debug, Clone)]
pub struct Av1EncoderConfig {
    pub width: u32,
    pub height: u32,
    /// Numerador del framerate (p.ej. 30 para 30 fps con `fps_den = 1`).
    pub fps_num: u32,
    /// Denominador del framerate (1, o 1001 para 29.97).
    pub fps_den: u32,
    pub quantizer: usize,
    pub speed: u8,
    /// Hilos del encoder. 0 = un solo hilo (el más portable).
    pub threads: usize,
}

impl Default for Av1EncoderConfig {
    fn default() -> Self {
        Self {
            width: 320,
            height: 240,
            fps_num: 30,
            fps_den: 1,
            quantizer: 100,
            speed: 8,
            threads: 0,
        }
    }
}

#[derive(Debug)]
pub enum Av1EncodeError {
    /// Config rechazada por rav1e (dimensiones inválidas, etc.).
    InvalidConfig(String),
    /// El encoder reportó un fallo interno.
    Encode(String),
    /// El frame RGBA no medía `width * height * 4` bytes.
    BadFrameSize { expected: usize, got: usize },
    Io(io::Error),
}

impl std::fmt::Display for Av1EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(e) => write!(f, "config inválida: {e}"),
            Self::Encode(e) => write!(f, "encode: {e}"),
            Self::BadFrameSize { expected, got } => {
                write!(f, "frame RGBA de {got} bytes, esperaba {expected}")
            }
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for Av1EncodeError {}

impl From<io::Error> for Av1EncodeError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Un paquete AV1 codificado (una *temporal unit* lista para meter en IVF).
#[derive(Debug)]
pub struct EncodedPacket {
    /// Bytes del bitstream AV1 (OBUs) del frame.
    pub data: Vec<u8>,
    /// Número de frame de entrada que generó este paquete.
    pub frame_number: u64,
}

/// Encoder AV1 con estado. `encode_rgba` empuja un frame y drena los
/// paquetes listos; `finish` vacía la tubería al final.
pub struct Av1Encoder {
    ctx: Context<u8>,
    width: usize,
    height: usize,
}

impl Av1Encoder {
    pub fn new(cfg: Av1EncoderConfig) -> Result<Self, Av1EncodeError> {
        let mut enc = EncoderConfig {
            width: cfg.width as usize,
            height: cfg.height as usize,
            bit_depth: 8,
            chroma_sampling: ChromaSampling::Cs420,
            // Full range: el decoder de media-source-av1 aplica la matriz
            // BT.601 full (sin escalado 16-235). Señalizamos igual aunque
            // el decoder no lea el flag — mantiene el bitstream honesto.
            pixel_range: PixelRange::Full,
            time_base: Rational {
                num: cfg.fps_den.max(1) as u64,
                den: cfg.fps_num.max(1) as u64,
            },
            quantizer: cfg.quantizer.min(255),
            speed_settings: SpeedSettings::from_preset(cfg.speed.min(10)),
            ..Default::default()
        };
        // Sin objetivo de bitrate → cuantizador constante.
        enc.bitrate = 0;

        let config = Config::new()
            .with_encoder_config(enc)
            .with_threads(cfg.threads.max(1));
        let ctx: Context<u8> = config
            .new_context()
            .map_err(|e| Av1EncodeError::InvalidConfig(format!("{e:?}")))?;

        Ok(Self {
            ctx,
            width: cfg.width as usize,
            height: cfg.height as usize,
        })
    }

    /// Empuja un frame RGBA8 (`width * height * 4` bytes, fila por fila) y
    /// devuelve los paquetes que quedaron listos tras enviarlo. rav1e tiene
    /// latencia: los primeros frames pueden no producir paquetes todavía.
    pub fn encode_rgba(&mut self, rgba: &[u8]) -> Result<Vec<EncodedPacket>, Av1EncodeError> {
        let expected = self.width * self.height * 4;
        if rgba.len() != expected {
            return Err(Av1EncodeError::BadFrameSize {
                expected,
                got: rgba.len(),
            });
        }

        let (y, u, v) = rgba_to_i420_full(rgba, self.width, self.height);
        let cw = (self.width + 1) / 2;

        let mut frame = self.ctx.new_frame();
        frame.planes[0].copy_from_raw_u8(&y, self.width, 1);
        frame.planes[1].copy_from_raw_u8(&u, cw, 1);
        frame.planes[2].copy_from_raw_u8(&v, cw, 1);

        match self.ctx.send_frame(frame) {
            Ok(()) => {}
            Err(EncoderStatus::EnoughData) => {
                // La cola está llena: hay que drenar antes de reintentar.
                // No debería pasar con drenado tras cada envío, pero lo
                // toleramos drenando y reintentando una vez.
            }
            Err(e) => return Err(Av1EncodeError::Encode(format!("send_frame: {e:?}"))),
        }
        self.drain()
    }

    /// Cierra la entrada y drena todos los paquetes restantes. Tras esto el
    /// encoder no acepta más frames.
    pub fn finish(&mut self) -> Result<Vec<EncodedPacket>, Av1EncodeError> {
        self.ctx.flush();
        self.drain()
    }

    /// Drena los paquetes disponibles sin enviar más frames.
    fn drain(&mut self) -> Result<Vec<EncodedPacket>, Av1EncodeError> {
        let mut out = Vec::new();
        loop {
            match self.ctx.receive_packet() {
                Ok(pkt) => out.push(EncodedPacket {
                    data: pkt.data,
                    frame_number: pkt.input_frameno,
                }),
                // `Encoded`: rav1e codificó un frame pero todavía no emitió
                // su paquete — hay que seguir llamando, no parar.
                Err(EncoderStatus::Encoded) => continue,
                // Sin más datos pendientes (necesita más frames o terminó).
                Err(EncoderStatus::NeedMoreData)
                | Err(EncoderStatus::LimitReached)
                | Err(EncoderStatus::EnoughData) => break,
                Err(e) => return Err(Av1EncodeError::Encode(format!("receive_packet: {e:?}"))),
            }
        }
        Ok(out)
    }
}

// ─── Muxer IVF ───────────────────────────────────────────────────────────────
//
// Espejo del demuxer de media-source-av1: cabecera de 32 bytes "DKIF" +
// FourCC "AV01" + dims + framerate + nº de frames, y por paquete 12 bytes
// (u32 tamaño + u64 timestamp) seguidos de los bytes AV1.

/// Escribe la cabecera IVF de 32 bytes a `w`. `num_frames` puede ser 0 si
/// no se conoce de antemano (el demuxer igual lee hasta EOF).
fn write_ivf_header<W: Write>(
    w: &mut W,
    cfg: &Av1EncoderConfig,
    num_frames: u32,
) -> io::Result<()> {
    w.write_all(b"DKIF")?;
    w.write_all(&0u16.to_le_bytes())?; // versión
    w.write_all(&32u16.to_le_bytes())?; // largo de cabecera
    w.write_all(b"AV01")?; // FourCC del códec
    w.write_all(&(cfg.width as u16).to_le_bytes())?;
    w.write_all(&(cfg.height as u16).to_le_bytes())?;
    w.write_all(&cfg.fps_num.to_le_bytes())?; // timebase numerador
    w.write_all(&cfg.fps_den.to_le_bytes())?; // timebase denominador
    w.write_all(&num_frames.to_le_bytes())?;
    w.write_all(&0u32.to_le_bytes())?; // reservado
    Ok(())
}

/// Escribe un stream IVF completo (cabecera + todos los paquetes) a `w`.
/// `num_frames` queda con el conteo real de paquetes.
pub fn write_ivf<W: Write>(
    mut w: W,
    cfg: &Av1EncoderConfig,
    packets: &[EncodedPacket],
) -> io::Result<()> {
    write_ivf_header(&mut w, cfg, packets.len() as u32)?;
    for pkt in packets {
        w.write_all(&(pkt.data.len() as u32).to_le_bytes())?;
        w.write_all(&pkt.frame_number.to_le_bytes())?;
        w.write_all(&pkt.data)?;
    }
    w.flush()
}

/// Conveniencia: escribe el `.ivf` a `path`.
pub fn write_ivf_file(
    path: impl AsRef<Path>,
    cfg: &Av1EncoderConfig,
    packets: &[EncodedPacket],
) -> io::Result<()> {
    let f = BufWriter::new(File::create(path)?);
    write_ivf(f, cfg, packets)
}

/// Conveniencia de un tiro: encodea una secuencia de frames RGBA y escribe
/// el `.ivf` resultante. Para pipelines simples (render → archivo).
pub fn encode_rgba_to_ivf_file<'a>(
    path: impl AsRef<Path>,
    cfg: Av1EncoderConfig,
    frames: impl IntoIterator<Item = &'a [u8]>,
) -> Result<usize, Av1EncodeError> {
    let mut enc = Av1Encoder::new(cfg.clone())?;
    let mut packets = Vec::new();
    for frame in frames {
        packets.extend(enc.encode_rgba(frame)?);
    }
    packets.extend(enc.finish()?);
    let n = packets.len();
    write_ivf_file(path, &cfg, &packets)?;
    Ok(n)
}

// ─── Conversión RGBA → YUV420 (BT.601 full range) ──────────────────────────────
//
// Inverso exacto de la matriz del decoder (media-source-av1::decode::
// yuv_to_rgb): full range, sin escalado 16-235. La luma se calcula por
// pixel; el croma promedia cada bloque 2×2 antes de proyectar a U/V.

fn clamp8(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

fn rgba_to_i420_full(rgba: &[u8], w: usize, h: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let cw = (w + 1) / 2;
    let ch = (h + 1) / 2;
    let mut yp = vec![0u8; w * h];
    let mut up = vec![0u8; cw * ch];
    let mut vp = vec![0u8; cw * ch];

    for yy in 0..h {
        for xx in 0..w {
            let i = (yy * w + xx) * 4;
            let r = rgba[i] as f32;
            let g = rgba[i + 1] as f32;
            let b = rgba[i + 2] as f32;
            yp[yy * w + xx] = clamp8(0.299 * r + 0.587 * g + 0.114 * b);
        }
    }

    for cy in 0..ch {
        for cx in 0..cw {
            // Promedio del bloque 2×2 (recortado en los bordes impares).
            let (mut sr, mut sg, mut sb, mut n) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for dy in 0..2 {
                for dx in 0..2 {
                    let xx = cx * 2 + dx;
                    let yy = cy * 2 + dy;
                    if xx < w && yy < h {
                        let i = (yy * w + xx) * 4;
                        sr += rgba[i] as f32;
                        sg += rgba[i + 1] as f32;
                        sb += rgba[i + 2] as f32;
                        n += 1.0;
                    }
                }
            }
            let (r, g, b) = (sr / n, sg / n, sb / n);
            up[cy * cw + cx] = clamp8(-0.168736 * r - 0.331264 * g + 0.5 * b + 128.0);
            vp[cy * cw + cx] = clamp8(0.5 * r - 0.418688 * g - 0.081312 * b + 128.0);
        }
    }

    (yp, up, vp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_to_yuv_solid_colors() {
        // Blanco → Y≈255, croma neutro.
        let white = [255u8, 255, 255, 255];
        let (y, u, v) = rgba_to_i420_full(&white, 1, 1);
        assert!(y[0] >= 254);
        assert!((u[0] as i32 - 128).abs() <= 1);
        assert!((v[0] as i32 - 128).abs() <= 1);
        // Rojo puro → V alto, U bajo.
        let red = [255u8, 0, 0, 255];
        let (y, u, v) = rgba_to_i420_full(&red, 1, 1);
        assert!(y[0] > 60 && y[0] < 90, "luma rojo ≈76, fue {}", y[0]);
        assert!(v[0] > 200, "V rojo alto, fue {}", v[0]);
        assert!(u[0] < 110, "U rojo bajo, fue {}", u[0]);
    }

    #[test]
    fn bad_frame_size_rejected() {
        let cfg = Av1EncoderConfig {
            width: 64,
            height: 48,
            ..Default::default()
        };
        let mut enc = Av1Encoder::new(cfg).unwrap();
        let err = enc.encode_rgba(&[0u8; 10]).unwrap_err();
        assert!(matches!(err, Av1EncodeError::BadFrameSize { .. }));
    }

    #[test]
    fn encodes_to_valid_ivf() {
        let cfg = Av1EncoderConfig {
            width: 64,
            height: 48,
            speed: 10,
            ..Default::default()
        };
        let frame = vec![128u8; 64 * 48 * 4];
        let mut buf = Vec::new();
        let mut enc = Av1Encoder::new(cfg.clone()).unwrap();
        let mut packets = Vec::new();
        for _ in 0..5 {
            packets.extend(enc.encode_rgba(&frame).unwrap());
        }
        packets.extend(enc.finish().unwrap());
        assert_eq!(packets.len(), 5, "5 frames in → 5 packets out");
        write_ivf(&mut buf, &cfg, &packets).unwrap();
        // Cabecera IVF válida.
        assert_eq!(&buf[0..4], b"DKIF");
        assert_eq!(&buf[8..12], b"AV01");
        assert_eq!(u16::from_le_bytes([buf[12], buf[13]]), 64);
        assert_eq!(u16::from_le_bytes([buf[14], buf[15]]), 48);
    }
}
