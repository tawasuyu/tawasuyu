//! Efectos de bus master aplicados al `AudioBuffer` post-mezcla.
//!
//! Hoy hay uno solo: un delay simple por feedback. La idea es mantener
//! el procesamiento agnóstico al renderer (osc, SF2, …) y dejar que
//! cada uno llame al efecto antes de normalizar el output.

use takiy_core::{DelayParams, ReverbParams};

use crate::audio::AudioBuffer;

/// Tope superior del feedback para que la cola decaiga. Más de esto
/// acumula amplitud cada ciclo de la línea y termina divergiendo
/// (sobre todo si la mezcla wet también es alta).
const MAX_FEEDBACK: f32 = 0.95;

/// Aplica un delay master in-place al `buf`. `sec_per_beat` viene del
/// tempo del `Score` (`60 / bpm`) — necesario para convertir
/// `params.time_beats` a samples.
///
/// El algoritmo es un *comb feedback* clásico, por canal:
///
/// ```text
/// y[n] = (1-mix)·x[n] + mix·d[n]          # output al buffer
/// d[n+D] = x[n] + feedback·d[n]            # almacenado en la línea
/// ```
///
/// donde `D` son los samples del retardo y `d[]` el ring buffer. Tras
/// la `D` muestras iniciales el primer eco aparece; las repeticiones
/// caen geométricamente según `feedback`. Si `feedback >= 1`, la línea
/// diverge — por eso clampeamos.
///
/// Idempotente sobre delay-of-silence: si el buffer es todo cero,
/// queda todo cero. Si `params.mix == 0`, el buffer queda intacto.
/// Si `delay_frames == 0` (tempo*beats degenerado), no toca nada.
pub fn apply_master_delay(buf: &mut AudioBuffer, sec_per_beat: f32, params: &DelayParams) {
    let channels = buf.channels.max(1) as usize;
    let delay_frames =
        (params.time_beats * sec_per_beat * buf.sample_rate as f32).round() as usize;
    if delay_frames == 0 || params.mix <= 0.0 {
        return;
    }
    let feedback = params.feedback.clamp(0.0, MAX_FEEDBACK);
    let mix = params.mix.clamp(0.0, 1.0);
    let dry = 1.0 - mix;
    let n_frames = buf.frames();

    // Ring buffer por canal. Pre-alocado al delay exacto: el índice
    // avanza módulo `delay_frames`, así que cada canal sólo necesita
    // ese tamaño.
    let mut delay_lines: Vec<Vec<f32>> =
        (0..channels).map(|_| vec![0.0; delay_frames]).collect();
    let mut idx = 0usize;

    for f in 0..n_frames {
        for (c, line) in delay_lines.iter_mut().enumerate().take(channels) {
            let i = f * channels + c;
            let dry_in = buf.samples[i];
            let wet = line[idx];
            buf.samples[i] = dry * dry_in + mix * wet;
            line[idx] = dry_in + feedback * wet;
        }
        idx += 1;
        if idx == delay_frames {
            idx = 0;
        }
    }
}

/// Tamaños de delay en samples a 44.1 kHz para los combs paralelos del
/// reverb. Vienen del catálogo clásico de Freeverb (mutuamente primos
/// para evitar resonancias periódicas obvias). Se reescalan al
/// `sample_rate` real del buffer.
const COMB_DELAYS_44100: [usize; 4] = [1116, 1277, 1422, 1557];

/// Allpasses en serie tras los combs — esparcen los ecos del comb stack
/// para que la cola se sienta difusa y no como una secuencia rítmica.
const ALLPASS_DELAYS_44100: [usize; 2] = [556, 441];

/// Coeficiente del allpass (clásico de Schroeder, fijo en `0.5`).
const ALLPASS_FEEDBACK: f32 = 0.5;

/// Escala el delay clásico de 44.1 kHz al sample-rate del buffer
/// preservando el tamaño proporcional. Mínimo 1 sample para no
/// degenerar el ring buffer.
fn scaled_delay(d: usize, sample_rate: u32) -> usize {
    let scaled = (d as f32 * sample_rate as f32 / 44_100.0).round() as usize;
    scaled.max(1)
}

/// Comb filter con damping (low-pass de un polo en el feedback path).
///
/// La forma: `out = buffer[i]; filterstore = out*(1-d) + filterstore*d;
/// buffer[i] = input + filterstore*feedback`. El damping mete una
/// inercia que suaviza las altas en cada vuelta del feedback — más
/// damping, menos brillo en la cola.
struct CombFilter {
    buffer: Vec<f32>,
    idx: usize,
    feedback: f32,
    damping: f32,
    filterstore: f32,
}

impl CombFilter {
    fn new(size: usize, feedback: f32, damping: f32) -> Self {
        Self {
            buffer: vec![0.0; size],
            idx: 0,
            feedback,
            damping,
            filterstore: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.idx];
        // Low-pass de un polo en el feedback (sin coeficientes en
        // dB — el `damping` lineal alcanza para el rango útil).
        self.filterstore = output * (1.0 - self.damping) + self.filterstore * self.damping;
        self.buffer[self.idx] = input + self.feedback * self.filterstore;
        self.idx += 1;
        if self.idx == self.buffer.len() {
            self.idx = 0;
        }
        output
    }
}

/// Allpass clásico de Schroeder: `out = -input + buffered; buffer[i] =
/// input + buffered * 0.5`. No tiene parámetros — su rol es difundir,
/// no colorear.
struct AllPass {
    buffer: Vec<f32>,
    idx: usize,
}

impl AllPass {
    fn new(size: usize) -> Self {
        Self { buffer: vec![0.0; size], idx: 0 }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buffer[self.idx];
        let out = -input + buffered;
        self.buffer[self.idx] = input + buffered * ALLPASS_FEEDBACK;
        self.idx += 1;
        if self.idx == self.buffer.len() {
            self.idx = 0;
        }
        out
    }
}

/// Aplica un reverb tipo Schroeder in-place al `buf`. Cada canal corre
/// su propia red (4 combs paralelos sumados + 2 allpasses en serie)
/// para preservar la imagen estéreo del input — los allpasses son los
/// mismos en ambos canales (más simple; un reverb más rico usaría
/// pares L/R offseteados).
///
/// `room_size` controla el feedback de los combs: `0.0` → 0.70 (sala
/// pequeña, cola corta), `1.0` → 0.98 (catedral, cola larga). El
/// damping atenúa las altas en el feedback para emular paredes
/// absorbentes. Mix wet/dry final, idéntico al patrón del delay.
///
/// Bypass automático si `params.mix <= 0` (no-op limpio).
pub fn apply_master_reverb(buf: &mut AudioBuffer, params: &ReverbParams) {
    if params.mix <= 0.0 {
        return;
    }
    let channels = buf.channels.max(1) as usize;
    let mix = params.mix.clamp(0.0, 1.0);
    let dry_gain = 1.0 - mix;
    // Mapeo Schroeder clásico: room_size lineal a [0.70, 0.98] de feedback.
    let feedback = 0.70 + params.room_size.clamp(0.0, 1.0) * 0.28;
    let damping = params.damping.clamp(0.0, 1.0);

    // Red por canal: 4 combs + 2 allpasses, todos con sus ring buffers
    // independientes (estado por canal).
    let mut combs_per_ch: Vec<Vec<CombFilter>> = (0..channels)
        .map(|_| {
            COMB_DELAYS_44100
                .iter()
                .map(|&d| CombFilter::new(scaled_delay(d, buf.sample_rate), feedback, damping))
                .collect()
        })
        .collect();
    let mut allpasses_per_ch: Vec<Vec<AllPass>> = (0..channels)
        .map(|_| {
            ALLPASS_DELAYS_44100
                .iter()
                .map(|&d| AllPass::new(scaled_delay(d, buf.sample_rate)))
                .collect()
        })
        .collect();

    let n_frames = buf.frames();
    for f in 0..n_frames {
        for c in 0..channels {
            let i = f * channels + c;
            let dry_in = buf.samples[i];
            // Combs en paralelo: la suma se normaliza por la cantidad
            // (mantiene la energía controlada al cambiar el N).
            let mut wet = 0.0;
            for comb in &mut combs_per_ch[c] {
                wet += comb.process(dry_in);
            }
            wet /= COMB_DELAYS_44100.len() as f32;
            // Allpasses en serie para difundir el comb stack.
            for ap in &mut allpasses_per_ch[c] {
                wet = ap.process(wet);
            }
            buf.samples[i] = dry_gain * dry_in + mix * wet;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse_buf(channels: u16, sample_rate: u32, n_frames: usize) -> AudioBuffer {
        let mut buf = AudioBuffer::silence_with_channels(sample_rate, n_frames, channels);
        // Impulso unitario en el primer frame, todos los canales.
        for c in 0..channels as usize {
            buf.samples[c] = 1.0;
        }
        buf
    }

    #[test]
    fn delay_with_mix_zero_is_noop() {
        let mut buf = impulse_buf(2, 44_100, 1024);
        let before = buf.samples.clone();
        apply_master_delay(
            &mut buf,
            0.5,
            &DelayParams { time_beats: 0.25, feedback: 0.5, mix: 0.0 },
        );
        assert_eq!(buf.samples, before);
    }

    #[test]
    fn delay_inserts_echoes_at_expected_frames() {
        // 1 beat/sec @ 100 Hz → 100 samples por beat. time_beats = 0.25 → 25 frames.
        let sr = 100;
        let mut buf = impulse_buf(1, sr, 200);
        apply_master_delay(
            &mut buf,
            1.0, // sec_per_beat = 1
            &DelayParams { time_beats: 0.25, feedback: 0.5, mix: 1.0 },
        );
        // En t=0 hay un impulso de amplitud 1 (dry=0, mix=1 → out=0 + mix*wet,
        // pero wet de la línea arranca en 0 → out=0). Tras 25 frames aparece
        // el primer eco (= mix * 1.0 = 1.0). A los 50 frames otro eco = 0.5.
        assert!(buf.samples[0].abs() < 1e-6, "frame 0 sin dry");
        assert!((buf.samples[25] - 1.0).abs() < 1e-6, "eco 1 a 25 frames");
        assert!((buf.samples[50] - 0.5).abs() < 1e-6, "eco 2 a 50 frames (fb)");
        assert!((buf.samples[75] - 0.25).abs() < 1e-6, "eco 3 a 75 frames");
    }

    #[test]
    fn delay_decays_under_max_feedback() {
        // Con feedback = MAX la cola debe converger (no diverger). Tomamos
        // un buffer largo y vemos que el peak no crece sin cota.
        let mut buf = impulse_buf(1, 100, 10_000);
        apply_master_delay(
            &mut buf,
            1.0,
            &DelayParams { time_beats: 0.25, feedback: 1.5 /* clamp a 0.95 */, mix: 1.0 },
        );
        // Pico no debe exceder mucho más que 1.0 (algunos pocos % por
        // sumas). Si feedback no estuviera clampeado, divergiría.
        let peak = buf.samples.iter().fold(0.0_f32, |a, b| a.max(b.abs()));
        assert!(peak < 2.0, "el peak {peak} sugiere divergencia");
    }

    #[test]
    fn delay_skips_when_time_too_small() {
        // time_beats * sec_per_beat * sr ≈ 0 → delay_frames = 0 → no-op.
        let mut buf = impulse_buf(2, 44_100, 32);
        let before = buf.samples.clone();
        apply_master_delay(
            &mut buf,
            1.0,
            &DelayParams { time_beats: 1e-9, feedback: 0.5, mix: 0.5 },
        );
        assert_eq!(buf.samples, before);
    }

    #[test]
    fn reverb_with_mix_zero_is_noop() {
        let mut buf = impulse_buf(2, 44_100, 2048);
        let before = buf.samples.clone();
        apply_master_reverb(
            &mut buf,
            &ReverbParams { room_size: 0.5, damping: 0.5, mix: 0.0 },
        );
        assert_eq!(buf.samples, before);
    }

    #[test]
    fn reverb_produces_tail_after_impulse() {
        // Un impulso a t=0 + reverb wet=1: las muestras tras los primeros
        // delays de los combs no pueden ser todas cero — debe haber cola.
        let mut buf = impulse_buf(1, 44_100, 8192);
        apply_master_reverb(
            &mut buf,
            &ReverbParams { room_size: 0.9, damping: 0.2, mix: 1.0 },
        );
        // Saltamos los primeros 1500 frames (~ después del primer comb delay
        // y del primer allpass) y exigimos que aún haya energía.
        let tail_energy: f32 = buf.samples[1500..].iter().map(|x| x * x).sum();
        assert!(tail_energy > 1e-6, "reverb no generó cola");
    }

    #[test]
    fn reverb_tail_decays_over_long_buffer() {
        // room_size=1 → feedback ≈ 0.98 (alto). Buffer largo: la cola
        // debería decaer hasta caer por debajo del peak en al menos
        // 12 dB hacia el final. Si el feedback estuviera mal clampeado,
        // divergiría en lugar de decaer.
        let sr = 44_100;
        let mut buf = AudioBuffer::silence_with_channels(sr, sr as usize * 4, 1);
        for i in 0..100 {
            buf.samples[i] = 0.1;
        }
        apply_master_reverb(
            &mut buf,
            &ReverbParams { room_size: 1.0, damping: 0.5, mix: 1.0 },
        );
        let peak = buf.samples.iter().fold(0.0_f32, |a, b| a.max(b.abs()));
        assert!(peak.is_finite() && peak < 1.0, "reverb diverge o NaN (peak {peak})");
        // Tras 4 segundos, la cola debe ser ≤ 0.25 × peak (≈ -12 dB).
        let tail_start = buf.samples.len() - sr as usize / 2;
        let tail_peak = buf.samples[tail_start..]
            .iter()
            .fold(0.0_f32, |a, b| a.max(b.abs()));
        assert!(
            tail_peak < peak * 0.25,
            "cola no decae (tail {tail_peak} vs peak {peak})"
        );
    }
}
