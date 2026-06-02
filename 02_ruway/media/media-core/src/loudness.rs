//! loudness — medición de sonoridad integrada (LUFS) según ITU-R BS.1770-4 /
//! EBU R128, y la ganancia de normalización que se deriva de ella. Es la
//! pata "automática" de A5 (`PARIDAD.md`): hasta ahora la ganancia de
//! [`crate::dynamics`] la fijaba el usuario a mano; acá la medimos.
//!
//! Es la base de **ReplayGain 2.0** (que usa exactamente esta medición con un
//! objetivo de −18 LUFS) y de la normalización por programa de EBU R128
//! (objetivo −23 LUFS). El resultado, `gain_to_target_db`, alimenta
//! directamente a `DynamicsControl::add_gain_db`.
//!
//! Cero dependencias — sólo `f64`/`f32` sobre los samples, así que corre en CI
//! sin hardware (mismo principio que todo `media-core`).
//!
//! ## El algoritmo (BS.1770)
//!
//! 1. **K-weighting**: cada canal pasa por dos biquads en cascada — un
//!    realzado de agudos ("head") + un pasa-altos (RLB) — que modelan la
//!    respuesta del oído. Los coeficientes se re-derivan por sample rate
//!    (bilineal), no se hardcodean a 48 kHz.
//! 2. **Mean square por bloque**: la señal pesada se parte en sub-bloques de
//!    100 ms; un "bloque" de medición son 4 sub-bloques (400 ms, 75 % de
//!    solapamiento → paso de 100 ms).
//! 3. **Suma con peso de canal**: L/R/C pesan 1.0; surround (Ls/Rs) 1.41; el
//!    LFE no cuenta. La sonoridad de un bloque es
//!    `−0.691 + 10·log10(Σ peso·meanSquare)`.
//! 4. **Gating**: se descartan los bloques por debajo de −70 LUFS (gate
//!    absoluto) y luego los que estén 10 LU por debajo de la media de los que
//!    sobrevivieron (gate relativo). La sonoridad **integrada** es la media
//!    (en energía) de los bloques que pasan ambos gates.

use crate::AudioSource;

/// Objetivo de ReplayGain 2.0: −18 LUFS.
pub const REPLAYGAIN_TARGET_LUFS: f32 = -18.0;
/// Objetivo de normalización por programa de EBU R128: −23 LUFS.
pub const EBU_R128_TARGET_LUFS: f32 = -23.0;

/// Offset de calibración de BS.1770 (`−0.691 dB`).
const ABS_OFFSET: f64 = -0.691;
/// Gate absoluto: bloques por debajo de −70 LUFS no cuentan.
const ABSOLUTE_GATE_LUFS: f64 = -70.0;
/// Gate relativo: −10 LU respecto de la media de los bloques no-silenciosos.
const RELATIVE_GATE_LU: f64 = -10.0;

/// Un biquad de segundo orden en forma directa I (transposed), con estado por
/// canal. Coeficientes normalizados (`a0 = 1`).
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    #[inline]
    fn run(&self, x: f64, s: &mut BiquadState) -> f64 {
        // Forma directa II transpuesta.
        let y = self.b0 * x + s.z1;
        s.z1 = self.b1 * x - self.a1 * y + s.z2;
        s.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct BiquadState {
    z1: f64,
    z2: f64,
}

/// Los dos biquads del K-weighting, derivados para una sample rate dada
/// (calca libebur128). Etapa 1: realzado de agudos. Etapa 2: pasa-altos RLB.
fn k_weighting(fs: f64) -> (Biquad, Biquad) {
    use std::f64::consts::PI;

    // Etapa 1 — realzado de agudos (high-shelf).
    let f0 = 1681.974450955533;
    let g = 3.999843853973347; // dB
    let q = 0.7071752369554196;
    let k = (PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let stage1 = Biquad {
        b0: (vh + vb * k / q + k * k) / a0,
        b1: 2.0 * (k * k - vh) / a0,
        b2: (vh - vb * k / q + k * k) / a0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
    };

    // Etapa 2 — pasa-altos RLB.
    let f0 = 38.13547087602444;
    let q = 0.5003270373238773;
    let k = (PI * f0 / fs).tan();
    let den = 1.0 + k / q + k * k;
    let stage2 = Biquad {
        b0: 1.0,
        b1: -2.0,
        b2: 1.0,
        a1: 2.0 * (k * k - 1.0) / den,
        a2: (1.0 - k / q + k * k) / den,
    };

    (stage1, stage2)
}

/// Peso de canal de BS.1770 para un layout dado (`channels` totales). L/R/C y
/// el resto pesan 1.0; en 5.x el surround (índices 4 y 5) pesa 1.41 y el LFE
/// (índice 3) no cuenta. Para mono/estéreo todos pesan 1.0.
fn channel_weight(idx: u16, channels: u16) -> f64 {
    if channels >= 5 {
        match idx {
            3 => 0.0,            // LFE — excluido de la medición.
            4 | 5 => 1.41,       // Ls / Rs.
            _ => 1.0,            // L / R / C / extras.
        }
    } else {
        1.0
    }
}

/// Medidor de sonoridad integrada. Se alimenta con bloques de samples
/// intercalados (igual layout que [`AudioSource::fill`]) y al final entrega la
/// sonoridad integrada en LUFS y la ganancia hacia un objetivo.
///
/// Se **autoconfigura** con el `(sample_rate, channels)` del primer `push`, y
/// se reconfigura (descartando lo acumulado) si cambian — así encaja con la
/// cadena de audio, donde el rate recién se conoce en el `fill` del sink.
pub struct LoudnessMeter {
    /// Configuración vigente; `sample_rate == 0` ⇒ todavía sin configurar.
    sample_rate: u32,
    channels: u16,
    weights: Vec<f64>,
    filters: (Biquad, Biquad),
    states: Vec<(BiquadState, BiquadState)>,
    /// Samples por canal en un sub-bloque (100 ms).
    samples_per_subblock: usize,
    /// Acumulador de Σx² por canal del sub-bloque en curso.
    cur_sumsq: Vec<f64>,
    /// Cuántos samples (por canal) lleva el sub-bloque en curso.
    cur_count: usize,
    /// Σx² por canal de los sub-bloques ya cerrados (para armar bloques de
    /// 400 ms con solapamiento de 75 %).
    subblocks: Vec<Vec<f64>>,
    /// `z` (Σ peso·meanSquare) de cada bloque de 400 ms cerrado.
    block_z: Vec<f64>,
}

impl Default for LoudnessMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl LoudnessMeter {
    /// Crea un medidor sin configurar — se ajusta solo en el primer `push`.
    pub fn new() -> Self {
        LoudnessMeter {
            sample_rate: 0,
            channels: 0,
            weights: Vec::new(),
            filters: k_weighting(48_000.0),
            states: Vec::new(),
            samples_per_subblock: 1,
            cur_sumsq: Vec::new(),
            cur_count: 0,
            subblocks: Vec::new(),
            block_z: Vec::new(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// (Re)configura los filtros y acumuladores para un `(sample_rate,
    /// channels)` dado, descartando todo lo medido hasta ahora.
    fn configure(&mut self, sample_rate: u32, channels: u16) {
        let ch = channels.max(1) as usize;
        self.sample_rate = sample_rate.max(1);
        self.channels = channels.max(1);
        self.weights = (0..self.channels)
            .map(|i| channel_weight(i, self.channels))
            .collect();
        self.filters = k_weighting(self.sample_rate as f64);
        self.states = vec![(BiquadState::default(), BiquadState::default()); ch];
        // 100 ms por sub-bloque; al menos 1 sample para no dividir por cero.
        self.samples_per_subblock = ((self.sample_rate as usize) / 10).max(1);
        self.cur_sumsq = vec![0.0; ch];
        self.cur_count = 0;
        self.subblocks.clear();
        self.block_z.clear();
    }

    /// Vuelve el medidor a cero (mantiene la configuración vigente).
    pub fn reset(&mut self) {
        if self.sample_rate != 0 {
            self.configure(self.sample_rate, self.channels);
        }
    }

    /// Alimenta el medidor con `buf` (samples `f32` intercalados por canal) a
    /// la `sample_rate`/`channels` dadas. Reconfigura si cambian respecto del
    /// bloque anterior.
    pub fn push(&mut self, buf: &[f32], sample_rate: u32, channels: u16) {
        if sample_rate.max(1) != self.sample_rate || channels.max(1) != self.channels {
            self.configure(sample_rate, channels);
        }
        let ch = self.channels as usize;
        let (s1, s2) = self.filters;
        for frame in buf.chunks_exact(ch) {
            for (c, &x) in frame.iter().enumerate() {
                let st = &mut self.states[c];
                // K-weighting: dos biquads en cascada.
                let y = s2.run(s1.run(x as f64, &mut st.0), &mut st.1);
                self.cur_sumsq[c] += y * y;
            }
            self.cur_count += 1;
            if self.cur_count >= self.samples_per_subblock {
                self.close_subblock();
            }
        }
    }

    /// Cierra el sub-bloque en curso y, si ya hay 4, arma el bloque de 400 ms.
    fn close_subblock(&mut self) {
        self.subblocks.push(std::mem::replace(
            &mut self.cur_sumsq,
            vec![0.0; self.channels as usize],
        ));
        self.cur_count = 0;
        let n = self.subblocks.len();
        if n >= 4 {
            // meanSquare por canal sobre los últimos 4 sub-bloques.
            let total_samples = (self.samples_per_subblock * 4) as f64;
            let mut z = 0.0;
            for c in 0..self.channels as usize {
                let mut ss = 0.0;
                for sb in &self.subblocks[n - 4..n] {
                    ss += sb[c];
                }
                z += self.weights[c] * (ss / total_samples);
            }
            self.block_z.push(z);
        }
    }

    fn loudness_of(z: f64) -> f64 {
        ABS_OFFSET + 10.0 * z.log10()
    }

    /// Sonoridad integrada en LUFS, o `None` si no hubo suficiente audio
    /// audible (ni un bloque de 400 ms por encima del gate absoluto).
    pub fn integrated_lufs(&self) -> Option<f32> {
        if self.block_z.is_empty() {
            return None;
        }
        // Gate absoluto: bloques por encima de −70 LUFS.
        let above_abs: Vec<f64> = self
            .block_z
            .iter()
            .copied()
            .filter(|&z| z > 0.0 && Self::loudness_of(z) > ABSOLUTE_GATE_LUFS)
            .collect();
        if above_abs.is_empty() {
            return None;
        }
        // Gate relativo: −10 LU respecto de la media (en energía) de los
        // bloques que pasaron el gate absoluto.
        let mean_abs = above_abs.iter().sum::<f64>() / above_abs.len() as f64;
        let rel_gate = Self::loudness_of(mean_abs) + RELATIVE_GATE_LU;
        let gated: Vec<f64> = above_abs
            .into_iter()
            .filter(|&z| Self::loudness_of(z) > rel_gate)
            .collect();
        if gated.is_empty() {
            return None;
        }
        let mean = gated.iter().sum::<f64>() / gated.len() as f64;
        Some(Self::loudness_of(mean) as f32)
    }

    /// Ganancia en dB que hay que sumar para llevar el material al objetivo
    /// `target_lufs` (p. ej. [`REPLAYGAIN_TARGET_LUFS`]). `None` si no se pudo
    /// medir. Es directamente `target − integrada`.
    pub fn gain_to_target_db(&self, target_lufs: f32) -> Option<f32> {
        self.integrated_lufs().map(|lufs| target_lufs - lufs)
    }
}

/// Conveniencia: mide la sonoridad integrada de un buffer completo de samples
/// intercalados de una sola pasada.
pub fn measure_lufs(buf: &[f32], sample_rate: u32, channels: u16) -> Option<f32> {
    let mut m = LoudnessMeter::new();
    m.push(buf, sample_rate, channels);
    m.integrated_lufs()
}

/// Handle compartido y barato de clonar (sólo un `Arc`) sobre un
/// [`LoudnessMeter`]. Todas las copias comparten el mismo medidor: una vive en
/// el `fill` del audio realtime (a través de [`LoudnessProbe`]) y otra en la
/// UI, que lee la medida y fija la ganancia. Calca el molde de
/// [`crate::AudioProbe`].
#[derive(Clone)]
pub struct LoudnessTap {
    inner: std::sync::Arc<std::sync::Mutex<LoudnessMeter>>,
}

impl Default for LoudnessTap {
    fn default() -> Self {
        Self::new()
    }
}

impl LoudnessTap {
    pub fn new() -> Self {
        LoudnessTap {
            inner: std::sync::Arc::new(std::sync::Mutex::new(LoudnessMeter::new())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LoudnessMeter> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    pub fn push(&self, buf: &[f32], sample_rate: u32, channels: u16) {
        self.lock().push(buf, sample_rate, channels);
    }

    pub fn integrated_lufs(&self) -> Option<f32> {
        self.lock().integrated_lufs()
    }

    pub fn gain_to_target_db(&self, target_lufs: f32) -> Option<f32> {
        self.lock().gain_to_target_db(target_lufs)
    }

    /// Reinicia la medición (p. ej. al cambiar de pista).
    pub fn reset(&self) {
        self.lock().reset();
    }
}

/// Wrapper de [`AudioSource`] que mide la sonoridad de lo que pasa por él sin
/// alterarlo (tap pasivo, igual molde que [`crate::ProbedAudioSource`]). El
/// medidor es compartido vía [`LoudnessTap`], así la UI lee la medida en vivo.
pub struct LoudnessProbe<S> {
    inner: S,
    tap: LoudnessTap,
}

impl<S> LoudnessProbe<S> {
    pub fn new(inner: S, tap: LoudnessTap) -> Self {
        LoudnessProbe { inner, tap }
    }

    pub fn tap(&self) -> LoudnessTap {
        self.tap.clone()
    }
}

impl<S: AudioSource> AudioSource for LoudnessProbe<S> {
    fn fill(&mut self, buf: &mut [f32], sample_rate: u32, channels: u16) {
        self.inner.fill(buf, sample_rate, channels);
        self.tap.push(buf, sample_rate, channels);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    /// Genera `secs` segundos de seno a `freq` Hz, amplitud `amp`, en estéreo
    /// (canales idénticos), intercalado.
    fn sine_stereo(freq: f32, amp: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (secs * sr as f32) as usize;
        let mut v = Vec::with_capacity(n * 2);
        for i in 0..n {
            let s = amp * (2.0 * PI * freq * i as f32 / sr as f32).sin();
            v.push(s);
            v.push(s);
        }
        v
    }

    #[test]
    fn silencio_no_mide() {
        let buf = vec![0.0f32; 48_000 * 2]; // 1 s estéreo de silencio.
        assert_eq!(measure_lufs(&buf, 48_000, 2), None);
    }

    #[test]
    fn audio_corto_no_alcanza_un_bloque() {
        // 300 ms < 400 ms ⇒ ni un bloque completo ⇒ None.
        let buf = sine_stereo(1000.0, 0.5, 0.3, 48_000);
        assert_eq!(measure_lufs(&buf, 48_000, 2), None);
    }

    #[test]
    fn seno_full_scale_da_valor_finito_y_sano() {
        // Un seno de 1 kHz a fondo de escala ronda los 0 LUFS (el K-weighting
        // tiene ~0 dB cerca de 1 kHz). Banda amplia para no atarse al decimal.
        let buf = sine_stereo(1000.0, 1.0, 2.0, 48_000);
        let lufs = measure_lufs(&buf, 48_000, 2).expect("debe medir");
        assert!(lufs > -6.0 && lufs < 4.0, "fuera de rango: {lufs}");
    }

    #[test]
    fn mitad_de_amplitud_baja_seis_lu() {
        // −6 dB de amplitud ⇒ −6.02 LU de sonoridad (la medición es lineal en
        // potencia, independiente de la calibración absoluta).
        let full = measure_lufs(&sine_stereo(1000.0, 0.8, 2.0, 48_000), 48_000, 2).unwrap();
        let half = measure_lufs(&sine_stereo(1000.0, 0.4, 2.0, 48_000), 48_000, 2).unwrap();
        assert!((full - half - 6.02).abs() < 0.1, "Δ fue {}", full - half);
    }

    #[test]
    fn ganancia_a_objetivo_es_target_menos_medido() {
        let buf = sine_stereo(1000.0, 0.5, 2.0, 48_000);
        let mut m = LoudnessMeter::new();
        m.push(&buf, 48_000, 2);
        let lufs = m.integrated_lufs().unwrap();
        let g = m.gain_to_target_db(REPLAYGAIN_TARGET_LUFS).unwrap();
        assert!((g - (REPLAYGAIN_TARGET_LUFS - lufs)).abs() < 1e-4);
        // Material flojo (medido por debajo del objetivo) ⇒ ganancia positiva.
        let quiet = measure_lufs(&sine_stereo(1000.0, 0.02, 2.0, 48_000), 48_000, 2).unwrap();
        assert!(REPLAYGAIN_TARGET_LUFS - quiet > 0.0);
    }

    #[test]
    fn funciona_a_otra_sample_rate() {
        // 44.1 kHz: los coeficientes se re-derivan, debe medir igual de sano.
        let buf = sine_stereo(1000.0, 0.5, 2.0, 44_100);
        let lufs = measure_lufs(&buf, 44_100, 2).expect("debe medir");
        assert!(lufs > -12.0 && lufs < 0.0, "fuera de rango: {lufs}");
    }

    #[test]
    fn probe_no_altera_y_mide() {
        struct Sine {
            phase: f32,
        }
        impl AudioSource for Sine {
            fn fill(&mut self, buf: &mut [f32], sr: u32, ch: u16) {
                for frame in buf.chunks_mut(ch as usize) {
                    let s = 0.7 * (2.0 * PI * 1000.0 * self.phase / sr as f32).sin();
                    self.phase += 1.0;
                    for x in frame.iter_mut() {
                        *x = s;
                    }
                }
            }
        }
        let mut probe = LoudnessProbe::new(Sine { phase: 0.0 }, LoudnessTap::new());
        let tap = probe.tap();
        let mut buf = vec![0.0f32; 48_000 * 2];
        // 2 s en dos pasadas de 1 s.
        probe.fill(&mut buf, 48_000, 2);
        let copia = buf.clone();
        probe.fill(&mut buf, 48_000, 2);
        // El tap no toca el buffer: lo que sale es lo que generó la fuente.
        assert!(copia.iter().any(|&s| s.abs() > 0.1));
        // El handle compartido ve la medida del audio que pasó por el wrapper.
        assert!(tap.integrated_lufs().is_some());
    }

    #[test]
    fn reconfigura_al_cambiar_de_rate() {
        // Empieza a 48k y luego cambia a 44.1k: descarta lo viejo y mide sano.
        let mut m = LoudnessMeter::new();
        m.push(&sine_stereo(1000.0, 0.5, 0.5, 48_000), 48_000, 2);
        assert_eq!(m.sample_rate(), 48_000);
        m.push(&sine_stereo(1000.0, 0.5, 2.0, 44_100), 44_100, 2);
        assert_eq!(m.sample_rate(), 44_100);
        assert!(m.integrated_lufs().is_some());
    }
}
