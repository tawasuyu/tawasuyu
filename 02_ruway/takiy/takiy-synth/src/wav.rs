//! Escritor WAV PCM 16-bit (mono o estéreo), sin dependencias.
//!
//! Lee `AudioBuffer.channels` para escribir el header con los campos
//! correctos. Estéreo interleaved se transfiere directo sin reempaquetar.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use crate::audio::AudioBuffer;

/// Escribe `buf` a `path` como WAV PCM 16-bit. Mono o estéreo según
/// `buf.channels`. Las muestras se clamplean a `[-1, 1]` antes de
/// cuantizar a `i16`.
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
    let channels = buf.channels.max(1);
    let data_size = n_samples * bytes_per_sample;
    let riff_size = 36 + data_size;
    let byte_rate = buf.sample_rate * channels as u32 * bytes_per_sample;
    let block_align = (channels as u32 * bytes_per_sample) as u16;

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

    /// Relee un WAV PCM 16-bit mono escrito por `write_wav_to` y devuelve
    /// `(sample_rate, channels, bits_per_sample, samples_f32)`. Es un
    /// parser feo a propósito (sólo cubre el formato que escribimos): si
    /// el header cambia, este parser falla y los tests se enteran.
    fn parse_wav(bytes: &[u8]) -> (u32, u16, u16, Vec<f32>) {
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        let fmt_size = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        assert_eq!(fmt_size, 16);
        let format = u16::from_le_bytes(bytes[20..22].try_into().unwrap());
        assert_eq!(format, 1, "PCM");
        let channels = u16::from_le_bytes(bytes[22..24].try_into().unwrap());
        let sr = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        let byte_rate = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
        let block_align = u16::from_le_bytes(bytes[32..34].try_into().unwrap());
        let bps = u16::from_le_bytes(bytes[34..36].try_into().unwrap());
        // Coherencia interna del fmt chunk.
        assert_eq!(byte_rate, sr * channels as u32 * (bps as u32 / 8));
        assert_eq!(block_align, channels * (bps / 8));
        assert_eq!(&bytes[36..40], b"data");
        let data_size = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
        let body = &bytes[44..44 + data_size];
        let mut samples = Vec::with_capacity(body.len() / 2);
        for chunk in body.chunks_exact(2) {
            let q = i16::from_le_bytes([chunk[0], chunk[1]]);
            samples.push(q as f32 / i16::MAX as f32);
        }
        (sr, channels, bps, samples)
    }

    #[test]
    fn writes_valid_riff_header() {
        let buf = AudioBuffer::silence(44_100, 4);
        let mut out = Vec::new();
        write_wav_to(&buf, &mut out).unwrap();

        assert_eq!(&out[0..4], b"RIFF");
        assert_eq!(&out[8..12], b"WAVE");
        assert_eq!(&out[12..16], b"fmt ");
        assert_eq!(&out[36..40], b"data");

        let sr = u32::from_le_bytes(out[24..28].try_into().unwrap());
        assert_eq!(sr, 44_100);

        // data size = 4 samples * 2 bytes = 8
        let ds = u32::from_le_bytes(out[40..44].try_into().unwrap());
        assert_eq!(ds, 8);

        // riff_size = 36 + data_size
        let rs = u32::from_le_bytes(out[4..8].try_into().unwrap());
        assert_eq!(rs, 36 + 8);

        // Total file size = riff_size + 8 ("RIFF" + size field).
        assert_eq!(out.len(), 44 + 8);
    }

    #[test]
    fn header_fields_are_internally_consistent() {
        let buf = AudioBuffer::from_mono(48_000, vec![0.0; 1024]);
        let mut out = Vec::new();
        write_wav_to(&buf, &mut out).unwrap();
        let (sr, channels, bps, samples) = parse_wav(&out);
        assert_eq!(sr, 48_000);
        assert_eq!(channels, 1);
        assert_eq!(bps, 16);
        assert_eq!(samples.len(), 1024);
    }

    #[test]
    fn stereo_wav_writes_channels_two_and_correct_byte_rate() {
        // 4 frames stereo = 8 samples interleaved.
        let buf = AudioBuffer::from_stereo(44_100,
            vec![0.5, -0.5, 0.25, -0.25, 0.1, -0.1, 0.0, 0.0]);
        let mut out = Vec::new();
        write_wav_to(&buf, &mut out).unwrap();
        let (sr, channels, bps, samples) = parse_wav(&out);
        assert_eq!(sr, 44_100);
        assert_eq!(channels, 2);
        assert_eq!(bps, 16);
        assert_eq!(samples.len(), 8);
        // byte_rate y block_align se validan dentro de parse_wav.
    }

    #[test]
    fn clamps_out_of_range_samples() {
        let buf = AudioBuffer::from_mono(8_000, vec![2.0, -2.0]);
        let mut out = Vec::new();
        write_wav_to(&buf, &mut out).unwrap();
        let lo = i16::from_le_bytes(out[44..46].try_into().unwrap());
        let hi = i16::from_le_bytes(out[46..48].try_into().unwrap());
        assert_eq!(lo, i16::MAX);
        assert_eq!(hi, -i16::MAX);
    }

    #[test]
    fn roundtrip_preserves_samples_within_quantization_error() {
        // Una rampa de 256 valores en [-1, 1].
        let samples: Vec<f32> = (0..256).map(|i| (i as f32 / 255.0) * 2.0 - 1.0).collect();
        let buf = AudioBuffer::from_mono(22_050, samples.clone());
        let mut out = Vec::new();
        write_wav_to(&buf, &mut out).unwrap();
        let (sr, channels, _bps, decoded) = parse_wav(&out);
        assert_eq!(sr, 22_050);
        assert_eq!(channels, 1);
        assert_eq!(decoded.len(), samples.len());
        // Cuantización a 16-bit: error máximo ≈ 1 / i16::MAX ≈ 3e-5.
        let max_err = decoded
            .iter()
            .zip(samples.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(max_err < 1e-4, "max_err = {max_err}");
    }

    #[test]
    fn empty_buffer_yields_only_header() {
        let buf = AudioBuffer::from_mono(44_100, vec![]);
        let mut out = Vec::new();
        write_wav_to(&buf, &mut out).unwrap();
        assert_eq!(out.len(), 44);
        let ds = u32::from_le_bytes(out[40..44].try_into().unwrap());
        assert_eq!(ds, 0);
        let rs = u32::from_le_bytes(out[4..8].try_into().unwrap());
        assert_eq!(rs, 36);
    }

    #[test]
    fn file_roundtrip_through_tempfile() {
        let buf = AudioBuffer::from_mono(16_000, vec![0.1, -0.2, 0.3, -0.4]);
        let path = std::env::temp_dir().join("takiy-wav-roundtrip.wav");
        write_wav(&buf, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        let (sr, channels, bps, decoded) = parse_wav(&bytes);
        assert_eq!(sr, 16_000);
        assert_eq!(channels, 1);
        assert_eq!(bps, 16);
        assert_eq!(decoded.len(), 4);
    }
}
