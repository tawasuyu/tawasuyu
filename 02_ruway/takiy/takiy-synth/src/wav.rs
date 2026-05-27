//! Escritor WAV PCM 16-bit mono, sin dependencias.
//!
//! Suficiente para escuchar el render en cualquier reproductor. Si más
//! adelante hace falta stereo o 24-bit, se añade aquí mismo o se cambia
//! por `hound`; por ahora menos crates es mejor.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use crate::audio::AudioBuffer;

/// Escribe `buf` a `path` como WAV PCM 16-bit mono. Las muestras se
/// clamplean a `[-1, 1]` antes de cuantizar a `i16`.
pub fn write_wav<P: AsRef<Path>>(buf: &AudioBuffer, path: P) -> io::Result<()> {
    let file = File::create(path)?;
    let mut w = BufWriter::new(file);
    write_wav_to(buf, &mut w)?;
    w.flush()
}

/// Misma lógica que [`write_wav`] pero sobre un `Write` arbitrario —
/// útil para tests o para enviar el WAV por la red.
pub fn write_wav_to<W: Write>(buf: &AudioBuffer, w: &mut W) -> io::Result<()> {
    let n_samples = buf.samples.len() as u32;
    let bytes_per_sample = 2u32;
    let channels = 1u16;
    let data_size = n_samples * bytes_per_sample;
    let riff_size = 36 + data_size;
    let byte_rate = buf.sample_rate * bytes_per_sample;
    let block_align = bytes_per_sample as u16;

    // RIFF header
    w.write_all(b"RIFF")?;
    w.write_all(&riff_size.to_le_bytes())?;
    w.write_all(b"WAVE")?;

    // fmt  chunk (PCM)
    w.write_all(b"fmt ")?;
    w.write_all(&16u32.to_le_bytes())?; // chunk size for PCM
    w.write_all(&1u16.to_le_bytes())?; // format = 1 (PCM)
    w.write_all(&channels.to_le_bytes())?;
    w.write_all(&buf.sample_rate.to_le_bytes())?;
    w.write_all(&byte_rate.to_le_bytes())?;
    w.write_all(&block_align.to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?; // bits per sample

    // data chunk
    w.write_all(b"data")?;
    w.write_all(&data_size.to_le_bytes())?;
    for &s in &buf.samples {
        let q = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        w.write_all(&q.to_le_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_valid_riff_header() {
        let buf = AudioBuffer::silence(44_100, 4);
        let mut out = Vec::new();
        write_wav_to(&buf, &mut out).unwrap();

        assert_eq!(&out[0..4], b"RIFF");
        assert_eq!(&out[8..12], b"WAVE");
        assert_eq!(&out[12..16], b"fmt ");
        assert_eq!(&out[36..40], b"data");

        // sample_rate at bytes 24..28
        let sr = u32::from_le_bytes(out[24..28].try_into().unwrap());
        assert_eq!(sr, 44_100);

        // data size = 4 samples * 2 bytes = 8
        let ds = u32::from_le_bytes(out[40..44].try_into().unwrap());
        assert_eq!(ds, 8);
    }

    #[test]
    fn clamps_out_of_range_samples() {
        let buf = AudioBuffer { sample_rate: 8_000, samples: vec![2.0, -2.0] };
        let mut out = Vec::new();
        write_wav_to(&buf, &mut out).unwrap();
        // Last 4 bytes son las 2 muestras i16.
        let lo = i16::from_le_bytes(out[44..46].try_into().unwrap());
        let hi = i16::from_le_bytes(out[46..48].try_into().unwrap());
        assert_eq!(lo, i16::MAX);
        assert_eq!(hi, -i16::MAX);
    }
}
