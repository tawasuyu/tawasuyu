//! Análisis espectral y medición de niveles de audio.
//!
//! - [`Spectrum`]: banco de filtros Goertzel log-espaciados.
//! - [`Waterfall`]: historial 2D del espectro para visualización.
//! - [`Levels`]: medidor de pico y RMS con suavizado temporal.

// ============================================================
// Spectrum — análisis por bandas log-spaced (Goertzel)
// ============================================================

/// Banco de filtros Goertzel sobre un conjunto fijo de frecuencias
/// centro log-espaciadas. Pensado como motor del visor "barras"
/// (spectrogram instantáneo) sin traer dep de FFT.
///
/// Goertzel cuesta `2·N + 4` adds/mults por banda y por snapshot.
/// Para 32 bandas × 4096 samples ≈ 131k mults — barato a 30 fps.
///
/// El uso típico:
///
/// ```ignore
/// let mut spec = Spectrum::log_bands(32, 40.0, 16_000.0);
/// // ... más tarde, por frame:
/// spec.analyze(&snapshot, channels, sample_rate);
/// for (f, a) in spec.bands().iter().zip(spec.magnitudes()) { ... }
/// ```
pub struct Spectrum {
    centers: Vec<f32>,
    /// Magnitudes con suavizado temporal (attack/release simple).
    mags: Vec<f32>,
    /// Factor de release (0..1). Más cerca de 1 = decae más lento.
    release: f32,
}

impl Spectrum {
    /// Construye `n` bandas log-espaciadas entre `fmin` y `fmax`.
    /// Falla silenciosamente con `n == 0` (mags queda vacío y
    /// `analyze` no hace nada).
    pub fn log_bands(n: usize, fmin: f32, fmax: f32) -> Self {
        let fmin = fmin.max(1.0);
        let fmax = fmax.max(fmin * 2.0);
        let lo = fmin.ln();
        let hi = fmax.ln();
        let denom = (n.saturating_sub(1)).max(1) as f32;
        let centers: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / denom;
                (lo + (hi - lo) * t).exp()
            })
            .collect();
        Self {
            mags: vec![0.0; centers.len()],
            centers,
            release: 0.78,
        }
    }

    /// Factor de "release" del suavizado temporal: cuán rápido baja
    /// una banda cuando ya no hay señal. 0 = sin memoria; 0.95 = muy
    /// suave. Default 0.78 (≈ medio segundo a 30 fps).
    pub fn set_release(&mut self, release: f32) {
        self.release = release.clamp(0.0, 0.99);
    }

    pub fn bands(&self) -> &[f32] {
        &self.centers
    }

    pub fn magnitudes(&self) -> &[f32] {
        &self.mags
    }

    /// Corre Goertzel sobre `samples` (intercalados) plegando a mono y
    /// actualiza las magnitudes con attack inmediato + release
    /// exponencial. `sample_rate` y `channels` provienen del snapshot
    /// del probe.
    pub fn analyze(&mut self, samples: &[f32], channels: u16, sample_rate: u32) {
        if self.mags.is_empty() || samples.is_empty() || sample_rate == 0 {
            return;
        }
        let ch = channels.max(1) as usize;
        let frames = samples.len() / ch;
        if frames < 4 {
            return;
        }
        // Mono fold reusable. Lo construimos una vez por análisis para
        // que Goertzel itere sobre f32 contiguos.
        let mut mono: Vec<f32> = Vec::with_capacity(frames);
        let inv_ch = 1.0 / ch as f32;
        for f in 0..frames {
            let mut s = 0.0_f32;
            for c in 0..ch {
                s += samples[f * ch + c];
            }
            mono.push(s * inv_ch);
        }

        let n = frames as f32;
        let sr = sample_rate as f32;
        let nyquist = sr * 0.5;
        for (i, &freq) in self.centers.iter().enumerate() {
            if freq >= nyquist {
                // Sobre Nyquist no hay nada que medir; sólo decae.
                self.mags[i] *= self.release;
                continue;
            }
            // k continuo (no entero) sigue siendo válido para
            // visualización — distorsión leve cerca de bordes.
            let k = n * freq / sr;
            let w = std::f32::consts::TAU * k / n;
            let coeff = 2.0 * w.cos();
            let mut q1 = 0.0_f32;
            let mut q2 = 0.0_f32;
            for &s in &mono {
                let q0 = coeff * q1 - q2 + s;
                q2 = q1;
                q1 = q0;
            }
            // |X(k)|² = q1² + q2² - q1·q2·coeff
            let mag2 = (q1 * q1 + q2 * q2 - q1 * q2 * coeff).max(0.0);
            let mag = (mag2.sqrt() * 2.0 / n).min(1.0);
            // Attack inmediato, release suave.
            let prev = self.mags[i] * self.release;
            self.mags[i] = if mag > prev { mag } else { prev };
        }
    }
}

// ============================================================
// Waterfall — historial 2D del espectro
// ============================================================

/// Historial rotativo de magnitudes del [`Spectrum`]. Cada `analyze`
/// corre el Goertzel sobre el snapshot recibido y guarda la fila
/// resultante en un ring buffer de `rows` filas × `bands` columnas
/// — el visor (spectrogram waterfall) pinta el ring en orden
/// newest-first para que la onda nueva entre por arriba y empuje a
/// la vieja hacia abajo.
///
/// Las filas anteriores a la primera escritura quedan en 0.0
/// (silencio). El consumidor puede leer con [`Waterfall::snapshot`]
/// en orden cronológico inverso (fila 0 = más nueva).
pub struct Waterfall {
    spectrum: Spectrum,
    /// Buffer plano `rows × bands` (fila i, banda j en `[i*bands + j]`).
    grid: Vec<f32>,
    bands: usize,
    rows: usize,
    /// Índice de la fila a sobrescribir en el próximo analyze.
    head: usize,
    /// Cuántas filas se escribieron históricamente (clampada a rows).
    written: usize,
}

impl Waterfall {
    /// Crea un waterfall sobre `bands` bandas log-espaciadas y `rows`
    /// filas de historial. `bands == 0` o `rows == 0` se clampean a 1.
    pub fn new(bands: usize, rows: usize, fmin: f32, fmax: f32) -> Self {
        let bands = bands.max(1);
        let rows = rows.max(1);
        Self {
            spectrum: Spectrum::log_bands(bands, fmin, fmax),
            grid: vec![0.0; bands * rows],
            bands,
            rows,
            head: 0,
            written: 0,
        }
    }

    pub fn bands(&self) -> usize {
        self.bands
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Frecuencias centro de cada banda — espejo de [`Spectrum::bands`].
    pub fn band_freqs(&self) -> &[f32] {
        self.spectrum.bands()
    }

    /// Corre el spectrum sobre `samples` y agrega la fila resultante
    /// al ring. La fila vieja en `head` queda sobrescrita.
    pub fn analyze(&mut self, samples: &[f32], channels: u16, sample_rate: u32) {
        self.spectrum.analyze(samples, channels, sample_rate);
        let mags = self.spectrum.magnitudes();
        let bands = self.bands;
        let start = self.head * bands;
        // Copia la fila — `mags.len()` ya es == bands por construcción.
        self.grid[start..start + bands].copy_from_slice(mags);
        self.head = (self.head + 1) % self.rows;
        self.written = (self.written + 1).min(self.rows);
    }

    /// Copia el grid a `out` en orden newest-first: la fila 0 de
    /// `out` es la última analizada, fila `rows-1` la más vieja.
    /// `out` se redimensiona a `rows * bands`. Devuelve `(rows, bands)`.
    pub fn snapshot(&self, out: &mut Vec<f32>) -> (usize, usize) {
        let total = self.rows * self.bands;
        if out.len() != total {
            out.resize(total, 0.0);
        }
        if self.written == 0 {
            for v in out.iter_mut() {
                *v = 0.0;
            }
            return (self.rows, self.bands);
        }
        // newest = (head + rows - 1) % rows.
        let newest = (self.head + self.rows - 1) % self.rows;
        for i in 0..self.rows {
            // out[i] = grid[(newest - i) mod rows]
            let src_row = (newest + self.rows - i) % self.rows;
            let src_off = src_row * self.bands;
            let dst_off = i * self.bands;
            out[dst_off..dst_off + self.bands]
                .copy_from_slice(&self.grid[src_off..src_off + self.bands]);
        }
        (self.rows, self.bands)
    }
}

// ============================================================
// Levels — medidor peak + RMS sobre snapshots de audio
// ============================================================

/// Niveles instantáneos del stream: pico absoluto y RMS, ambos
/// normalizados a [0, 1] sobre el mono fold del snapshot. Mantiene
/// suavizado attack-inmediato + release-exponencial entre llamadas
/// (igual filosofía que [`Spectrum`]) para que las barras del visor
/// no titilen.
#[derive(Clone, Copy)]
pub struct Levels {
    peak: f32,
    rms: f32,
    release: f32,
}

impl Default for Levels {
    fn default() -> Self {
        Self::new()
    }
}

impl Levels {
    pub fn new() -> Self {
        Self {
            peak: 0.0,
            rms: 0.0,
            release: 0.82,
        }
    }

    pub fn set_release(&mut self, release: f32) {
        self.release = release.clamp(0.0, 0.99);
    }

    pub fn peak(&self) -> f32 {
        self.peak
    }

    pub fn rms(&self) -> f32 {
        self.rms
    }

    /// Procesa un snapshot intercalado y actualiza los niveles. El
    /// mono fold es promedio simple de canales; el RMS es sqrt(media
    /// de cuadrados) sobre los frames mono. Con `samples` vacío sólo
    /// aplica el release.
    pub fn analyze(&mut self, samples: &[f32], channels: u16) {
        let ch = channels.max(1) as usize;
        let frames = samples.len() / ch;
        if frames == 0 {
            self.peak *= self.release;
            self.rms *= self.release;
            return;
        }
        let inv_ch = 1.0 / ch as f32;
        let mut peak_inst = 0.0_f32;
        let mut sq_acc = 0.0_f32;
        for f in 0..frames {
            let mut s = 0.0_f32;
            for c in 0..ch {
                s += samples[f * ch + c];
            }
            let mono = s * inv_ch;
            let abs = mono.abs();
            if abs > peak_inst {
                peak_inst = abs;
            }
            sq_acc += mono * mono;
        }
        let rms_inst = (sq_acc / frames as f32).sqrt();

        // Attack inmediato, release exponencial.
        let prev_peak = self.peak * self.release;
        self.peak = if peak_inst > prev_peak {
            peak_inst.min(1.0)
        } else {
            prev_peak
        };
        let prev_rms = self.rms * self.release;
        self.rms = if rms_inst > prev_rms {
            rms_inst.min(1.0)
        } else {
            prev_rms
        };
    }
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests_waterfall {
    use super::*;

    fn synthetic_block(freq: f32, frames: usize, sr: u32) -> Vec<f32> {
        let mut v = Vec::with_capacity(frames);
        let dphi = std::f32::consts::TAU * freq / sr as f32;
        let mut phi = 0.0_f32;
        for _ in 0..frames {
            v.push(phi.sin() * 0.5);
            phi += dphi;
        }
        v
    }

    #[test]
    fn snapshot_is_newest_first() {
        let mut w = Waterfall::new(8, 4, 100.0, 4_000.0);
        // Primero un análisis con señal fuerte (482 Hz ≈ banda 3),
        // después uno con silencio. El release del Spectrum hace que
        // la fila más nueva tenga ENERGÍA MENOR que la fila anterior
        // (que vio la señal fresca).
        let hot = synthetic_block(482.0, 4096, 48_000);
        let silence = vec![0.0_f32; 4096];
        w.analyze(&hot, 1, 48_000);
        w.analyze(&silence, 1, 48_000);

        let mut snap = Vec::new();
        let (rows, bands) = w.snapshot(&mut snap);
        assert_eq!(rows, 4);
        assert_eq!(bands, 8);

        let row0_sum: f32 = snap[0..8].iter().sum();
        let row1_sum: f32 = snap[8..16].iter().sum();
        assert!(row1_sum > 0.0, "row1 debería capturar la señal");
        assert!(
            row1_sum > row0_sum,
            "row1 (señal fresca, {row1_sum}) debería superar a row0 (post-silencio, {row0_sum})"
        );
    }

    #[test]
    fn empty_snapshot_is_zero() {
        let w = Waterfall::new(4, 4, 100.0, 1_000.0);
        let mut snap = Vec::new();
        let (rows, bands) = w.snapshot(&mut snap);
        assert_eq!((rows, bands), (4, 4));
        assert!(snap.iter().all(|&v| v == 0.0));
    }
}

#[cfg(test)]
mod tests_audio_primitives {
    use super::*;
    use crate::audio::AudioProbe;

    fn sine(freq: f32, frames: usize, sr: u32, amp: f32) -> Vec<f32> {
        let mut v = Vec::with_capacity(frames);
        let dphi = std::f32::consts::TAU * freq / sr as f32;
        let mut phi = 0.0_f32;
        for _ in 0..frames {
            v.push(phi.sin() * amp);
            phi += dphi;
        }
        v
    }

    // ---------- Spectrum ----------

    #[test]
    fn spectrum_peaks_at_dominant_band() {
        // Senoide alineada exactamente al centro de banda 2.
        // Goertzel resuena → esa banda gana sin ambigüedad.
        let mut spec = Spectrum::log_bands(4, 100.0, 4_000.0);
        spec.set_release(0.0); // sin smoothing — análisis puro.
        let target = spec.bands()[2];
        let sig = sine(target, 4096, 48_000, 0.5);
        spec.analyze(&sig, 1, 48_000);
        let mags = spec.magnitudes();
        let argmax = mags
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert_eq!(argmax, 2, "esperaba banda 2, mags={mags:?}");
        assert!(mags[2] > 0.2, "magnitud banda dominante = {}", mags[2]);
    }

    #[test]
    fn spectrum_silence_decays() {
        let mut spec = Spectrum::log_bands(8, 40.0, 16_000.0);
        // Cargo energía y después silencio: el release debe bajar.
        let sig = sine(440.0, 4096, 48_000, 0.5);
        spec.analyze(&sig, 1, 48_000);
        let after_hot = spec.magnitudes().iter().sum::<f32>();
        spec.analyze(&[0.0; 4096], 1, 48_000);
        let after_silence = spec.magnitudes().iter().sum::<f32>();
        assert!(
            after_silence < after_hot,
            "silencio ({after_silence}) debería ser menor que hot ({after_hot})"
        );
    }

    // ---------- Levels ----------

    #[test]
    fn levels_peak_matches_signal_amplitude() {
        let mut lv = Levels::new();
        lv.set_release(0.0);
        // Senoide de amplitud 0.4 — pico debería estar cerca de 0.4.
        let sig = sine(440.0, 4096, 48_000, 0.4);
        lv.analyze(&sig, 1);
        assert!(
            (lv.peak() - 0.4).abs() < 0.02,
            "peak = {}, esperaba ≈ 0.4",
            lv.peak()
        );
        // RMS senoide = amp / sqrt(2) ≈ 0.283 para amp=0.4.
        let expected_rms = 0.4_f32 / std::f32::consts::SQRT_2;
        assert!(
            (lv.rms() - expected_rms).abs() < 0.02,
            "rms = {}, esperaba ≈ {expected_rms}",
            lv.rms()
        );
    }

    #[test]
    fn levels_silence_zeros_with_no_release() {
        let mut lv = Levels::new();
        lv.set_release(0.0);
        lv.analyze(&[0.0; 1024], 1);
        assert_eq!(lv.peak(), 0.0);
        assert_eq!(lv.rms(), 0.0);
    }

    #[test]
    fn levels_mono_fold_averages_channels() {
        let mut lv = Levels::new();
        lv.set_release(0.0);
        // Stereo donde L=+0.5 y R=-0.5: mono fold = 0, peak debería
        // estar cerca de 0 (cancela), no de 0.5.
        let mut sig = Vec::new();
        for _ in 0..1024 {
            sig.push(0.5);
            sig.push(-0.5);
        }
        lv.analyze(&sig, 2);
        assert!(lv.peak() < 1e-4, "peak con cancelación = {}", lv.peak());
    }

    // ---------- AudioProbe ----------

    #[test]
    fn probe_push_then_snapshot_is_chronological() {
        // Capacidad mínima del probe es 64 (ver AudioProbe::new) —
        // los tests trabajan a ese tamaño y validan los slots
        // ocupados al final del snapshot.
        let probe = AudioProbe::new(64);
        let data: Vec<f32> = (1..=6).map(|i| i as f32).collect();
        probe.push(&data, 48_000, 1);
        let mut out = Vec::new();
        let (sr, ch) = probe.snapshot(&mut out);
        assert_eq!(sr, 48_000);
        assert_eq!(ch, 1);
        assert_eq!(out.len(), 64);
        // Los primeros 58 slots quedaron en silencio (no se llenó
        // todavía la vuelta); los últimos 6 son el bloque empujado
        // en orden cronológico.
        assert!(out[..58].iter().all(|&v| v == 0.0));
        assert_eq!(&out[58..64], &data[..]);
    }

    #[test]
    fn probe_wrap_overwrites_oldest() {
        let probe = AudioProbe::new(64);
        // Empuja 70 valores en un ring de cap=64: los 6 primeros se
        // sobrescriben, el snapshot trae [7..70] en orden cronológico.
        let data: Vec<f32> = (1..=70).map(|i| i as f32).collect();
        probe.push(&data, 44_100, 2);
        let mut out = Vec::new();
        let (sr, ch) = probe.snapshot(&mut out);
        assert_eq!(sr, 44_100);
        assert_eq!(ch, 2);
        let expected: Vec<f32> = (7..=70).map(|i| i as f32).collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn probed_audio_source_is_transparent_and_caches() {
        use crate::audio::{AudioSource, ProbedAudioSource};

        struct Const(f32);
        impl AudioSource for Const {
            fn fill(&mut self, buf: &mut [f32], _: u32, _: u16) {
                for s in buf.iter_mut() {
                    *s = self.0;
                }
            }
        }
        let probe = AudioProbe::new(16);
        let mut probed = ProbedAudioSource::new(Const(0.3), probe.clone());
        let mut buf = vec![0.0_f32; 8];
        probed.fill(&mut buf, 48_000, 1);
        // El sink ve el mismo flujo que el inner.
        assert!(buf.iter().all(|&v| (v - 0.3).abs() < 1e-6));
        // El probe vio el bloque entero.
        let mut snap = Vec::new();
        probe.snapshot(&mut snap);
        let tail: Vec<f32> = snap.iter().rev().take(8).cloned().collect();
        assert!(tail.iter().all(|&v| (v - 0.3).abs() < 1e-6));
    }
}
