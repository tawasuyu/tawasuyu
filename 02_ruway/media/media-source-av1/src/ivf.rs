//! Demuxer IVF puro-Rust.
//!
//! IVF es el contenedor mínimo para streams de video elementales (el que
//! escupen `aomenc`/`SVT-AV1`/`ffmpeg -f ivf`). No tiene patentes, no
//! tiene índice, y su parseo cabe en una pantalla: por eso es el primer
//! contenedor del camino AV1 nativo. Cada "frame" IVF es una *temporal
//! unit* de AV1 — un paquete de OBUs que el decoder consume entero.
//!
//! Formato (little-endian):
//!
//! ```text
//! Cabecera de archivo (32 bytes):
//!   0  "DKIF"            magic
//!   4  u16 version       (0)
//!   6  u16 header_len    (32)
//!   8  FourCC codec      ("AV01" para AV1)
//!  12  u16 width
//!  14  u16 height
//!  16  u32 fps_num       (timebase numerador / framerate)
//!  20  u32 fps_den
//!  24  u32 num_frames
//!  28  u32 unused
//!
//! Por cada frame:
//!   0  u32 size          bytes del payload que siguen
//!   4  u64 timestamp     en unidades de timebase
//!  12  [size] bytes      temporal unit (OBUs)
//! ```

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

/// Metadata de la cabecera IVF. `width`/`height`/`fps` vienen del
/// contenedor — suficiente para dimensionar la textura sin tocar el
/// bitstream AV1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IvfHeader {
    pub codec: [u8; 4],
    pub width: u16,
    pub height: u16,
    pub fps_num: u32,
    pub fps_den: u32,
    pub num_frames: u32,
}

impl IvfHeader {
    /// Framerate en Hz. Cae a 30 si el denominador es 0 (archivo raro).
    pub fn fps(&self) -> f32 {
        if self.fps_den == 0 {
            return 30.0;
        }
        self.fps_num as f32 / self.fps_den as f32
    }

    /// `true` si el FourCC declara AV1 (`"AV01"`).
    pub fn is_av1(&self) -> bool {
        &self.codec == b"AV01"
    }
}

/// Una temporal unit cruda extraída del contenedor.
#[derive(Debug, Clone)]
pub struct TemporalUnit {
    /// Timestamp en unidades de timebase del header.
    pub timestamp: u64,
    /// Bytes del paquete (uno o más OBUs).
    pub data: Vec<u8>,
}

/// Lector IVF sobre cualquier `Read`. Construir valida la cabecera; cada
/// [`next_unit`](IvfReader::next_unit) avanza un paquete.
pub struct IvfReader<R> {
    reader: R,
    header: IvfHeader,
}

impl IvfReader<BufReader<File>> {
    /// Abre un `.ivf` del filesystem.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let f = File::open(path)?;
        Self::new(BufReader::new(f))
    }
}

impl<R: Read> IvfReader<R> {
    /// Lee y valida la cabecera de 32 bytes. Falla con `InvalidData` si
    /// no empieza con `"DKIF"`.
    pub fn new(mut reader: R) -> io::Result<Self> {
        let mut h = [0u8; 32];
        reader.read_exact(&mut h)?;
        if &h[0..4] != b"DKIF" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "no es IVF: falta magic 'DKIF'",
            ));
        }
        let header = IvfHeader {
            codec: [h[8], h[9], h[10], h[11]],
            width: u16::from_le_bytes([h[12], h[13]]),
            height: u16::from_le_bytes([h[14], h[15]]),
            fps_num: u32::from_le_bytes([h[16], h[17], h[18], h[19]]),
            fps_den: u32::from_le_bytes([h[20], h[21], h[22], h[23]]),
            num_frames: u32::from_le_bytes([h[24], h[25], h[26], h[27]]),
        };
        Ok(Self { reader, header })
    }

    pub fn header(&self) -> &IvfHeader {
        &self.header
    }

    /// Lee la próxima temporal unit. `Ok(None)` al final limpio del
    /// archivo (EOF justo donde tocaba la cabecera de frame).
    pub fn next_unit(&mut self) -> io::Result<Option<TemporalUnit>> {
        let mut fh = [0u8; 12];
        match self.reader.read_exact(&mut fh) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let size = u32::from_le_bytes([fh[0], fh[1], fh[2], fh[3]]) as usize;
        let timestamp = u64::from_le_bytes([
            fh[4], fh[5], fh[6], fh[7], fh[8], fh[9], fh[10], fh[11],
        ]);
        let mut data = vec![0u8; size];
        self.reader.read_exact(&mut data)?;
        Ok(Some(TemporalUnit { timestamp, data }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cabecera IVF mínima válida + un frame de 3 bytes.
    fn synthetic() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"DKIF");
        v.extend_from_slice(&0u16.to_le_bytes()); // version
        v.extend_from_slice(&32u16.to_le_bytes()); // header len
        v.extend_from_slice(b"AV01");
        v.extend_from_slice(&320u16.to_le_bytes()); // width
        v.extend_from_slice(&240u16.to_le_bytes()); // height
        v.extend_from_slice(&30u32.to_le_bytes()); // fps_num
        v.extend_from_slice(&1u32.to_le_bytes()); // fps_den
        v.extend_from_slice(&1u32.to_le_bytes()); // num_frames
        v.extend_from_slice(&0u32.to_le_bytes()); // unused
        // frame: size=3, ts=0, data=[1,2,3]
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes());
        v.extend_from_slice(&[1, 2, 3]);
        v
    }

    #[test]
    fn parse_header_and_unit() {
        let bytes = synthetic();
        let mut r = IvfReader::new(&bytes[..]).unwrap();
        let h = *r.header();
        assert!(h.is_av1());
        assert_eq!((h.width, h.height), (320, 240));
        assert_eq!(h.fps(), 30.0);
        assert_eq!(h.num_frames, 1);

        let u = r.next_unit().unwrap().unwrap();
        assert_eq!(u.timestamp, 0);
        assert_eq!(u.data, vec![1, 2, 3]);
        // Segundo next_unit = EOF limpio.
        assert!(r.next_unit().unwrap().is_none());
    }

    #[test]
    fn rejects_non_ivf() {
        let bytes = vec![0u8; 32];
        assert!(IvfReader::new(&bytes[..]).is_err());
    }

    #[test]
    fn real_fixture_header() {
        let bytes = include_bytes!("../tests/fixtures/testsrc_64x48.ivf");
        let r = IvfReader::new(&bytes[..]).unwrap();
        let h = *r.header();
        assert!(h.is_av1());
        assert_eq!((h.width, h.height), (64, 48));
        assert_eq!(h.fps(), 10.0);
        assert_eq!(h.num_frames, 10);
    }
}
