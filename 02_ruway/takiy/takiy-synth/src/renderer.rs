//! `Renderer` — convierte un `Score` en `AudioBuffer`.
//!
//! La interfaz es lo importante: hoy hay un solo backend
//! ([`OscRenderer`], osciladores feos), mañana entrarán SoundFonts y
//! render neural sin que la app de composición tenga que enterarse.

use takiy_core::{AutomationLane, Score, ScoreNote};

use crate::audio::AudioBuffer;
use crate::envelope::Adsr;
use crate::waveform::Waveform;

/// Cualquier cosa capaz de convertir un `Score` en audio mono.
pub trait Renderer {
    fn render(&self, score: &Score) -> AudioBuffer;
}

/// Renderer trivial: una sola onda y una sola envolvente para todas las
/// pistas. Es el MVP feo — sirve para escuchar lo que se compuso.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OscRenderer {
    pub sample_rate: u32,
    pub waveform: Waveform,
    pub envelope: Adsr,
}

impl Default for OscRenderer {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            waveform: Waveform::Sine,
            envelope: Adsr::DEFAULT,
        }
    }
}

impl Renderer for OscRenderer {
    fn render(&self, score: &Score) -> AudioBuffer {
        let bpm = score.tempo_bpm.max(1e-6);
        let sec_per_beat = 60.0 / bpm;
        // Colita extra para que las últimas notas terminen su release.
        let total_seconds =
            score.duration_beats() * sec_per_beat + self.envelope.release + 0.05;
        let n_frames = (total_seconds * self.sample_rate as f32).ceil() as usize;
        let mut buf = AudioBuffer::silence_with_channels(self.sample_rate, n_frames, 2);

        for (idx, track) in score.tracks().iter().enumerate() {
            if !score.track_is_audible(idx) {
                continue;
            }
            // Si la pista no tiene automación, usamos el path "estático"
            // (mismo cálculo que pre-F9 — ese mismo path es el que toca
            // el `wav_determinism` test). Si tiene, evaluamos sample-
            // accurate dentro del loop interno; sweeps de varios beats
            // bajo una nota larga se oyen.
            let vol_lane = track
                .volume_automation
                .as_ref()
                .filter(|l| !l.is_empty());
            let pan_lane = track
                .pan_automation
                .as_ref()
                .filter(|l| !l.is_empty());
            let static_gain = track.volume.max(0.0);
            let (static_gl, static_gr) = track.pan_gains();
            for note in track.notes() {
                self.mix_note_stereo(
                    note,
                    sec_per_beat,
                    &mut buf.samples,
                    static_gain,
                    static_gl,
                    static_gr,
                    vol_lane,
                    pan_lane,
                );
            }
        }

        if let Some(delay) = score.master_delay.as_ref() {
            crate::effects::apply_master_delay(&mut buf, sec_per_beat, delay);
        }
        if let Some(reverb) = score.master_reverb.as_ref() {
            crate::effects::apply_master_reverb(&mut buf, reverb);
        }
        buf.normalize_if_clipping();
        buf
    }
}

impl OscRenderer {
    #[allow(clippy::too_many_arguments)]
    fn mix_note_stereo(
        &self,
        note: &ScoreNote,
        sec_per_beat: f32,
        buf: &mut [f32],
        static_gain: f32,
        static_gl: f32,
        static_gr: f32,
        vol_lane: Option<&AutomationLane>,
        pan_lane: Option<&AutomationLane>,
    ) {
        let freq = note.pitch.frequency();
        let start_sec = note.start * sec_per_beat;
        let dur_sec = note.duration * sec_per_beat;
        let end_sec = start_sec + dur_sec + self.envelope.release;
        let static_amp = note.velocity as f32 / 127.0;

        let sr = self.sample_rate as f32;
        let n_frames = buf.len() / 2;
        let start_frame = (start_sec * sr) as usize;
        let end_frame = ((end_sec * sr).ceil() as usize).min(n_frames);
        if start_frame >= end_frame {
            return;
        }

        let inv_sr = 1.0 / sr;
        let has_automation = vol_lane.is_some() || pan_lane.is_some();

        if !has_automation {
            // Path estático — *bit-exact* equivalente a la versión
            // anterior. El test `wav_determinism` corre por este branch.
            let amp = static_amp * static_gain;
            for f in start_frame..end_frame {
                let t = (f - start_frame) as f32 * inv_sr;
                let phase = t * freq;
                let env = self.envelope.level(t, dur_sec);
                let s = self.waveform.sample(phase) * env * amp;
                buf[f * 2] += s * static_gl;
                buf[f * 2 + 1] += s * static_gr;
            }
            return;
        }

        // Path con automación: re-evaluamos vol/pan **por sample**
        // según el beat global del frame. El lookup en la lane es
        // O(log n) por punto-de-curva — los típicos N < 100 hacen que
        // esto sea barato comparado con el waveform/envelope.
        let inv_spb = 1.0 / sec_per_beat;
        for f in start_frame..end_frame {
            let t = (f - start_frame) as f32 * inv_sr;
            let phase = t * freq;
            let env = self.envelope.level(t, dur_sec);
            let beat = (f as f32) * inv_sr * inv_spb;
            let gain = vol_lane
                .map(|l| l.value_at(beat, static_gain).max(0.0))
                .unwrap_or(static_gain);
            let (gl, gr) = if let Some(pl) = pan_lane {
                let pan = pl.value_at(beat, 0.0).clamp(-1.0, 1.0);
                let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
                (theta.cos(), theta.sin())
            } else {
                (static_gl, static_gr)
            };
            let amp = static_amp * gain;
            let s = self.waveform.sample(phase) * env * amp;
            buf[f * 2] += s * gl;
            buf[f * 2 + 1] += s * gr;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use takiy_core::{Pitch, ScoreNote, Track};

    #[test]
    fn empty_score_renders_short_silence() {
        let score = Score::new(120.0);
        let buf = OscRenderer::default().render(&score);
        // No hay notas, pero queda la colita del release.
        assert!(buf.samples.iter().all(|&s| s == 0.0));
        assert!(buf.duration_seconds() < 1.0);
    }

    #[test]
    fn single_note_produces_signal() {
        let mut score = Score::new(60.0); // 1 beat por segundo
        let mut track = Track::new("a");
        track.add(ScoreNote::new(Pitch::A4, 0.0, 1.0, 127));
        score.add_track(track);

        let buf = OscRenderer::default().render(&score);
        // Debe haber al menos una muestra audible — el seno no es plano.
        assert!(buf.peak() > 0.3);
        // Y debe durar cerca de 1 segundo + release.
        assert!(buf.duration_seconds() > 1.0);
        assert!(buf.duration_seconds() < 1.3);
    }

    #[test]
    fn louder_velocity_produces_louder_audio() {
        let make = |vel: u8| {
            let mut s = Score::new(60.0);
            let mut t = Track::new("a");
            t.add(ScoreNote::new(Pitch::A4, 0.0, 1.0, vel));
            s.add_track(t);
            OscRenderer::default().render(&s).peak()
        };
        assert!(make(120) > make(40));
    }

    #[test]
    fn automation_volume_sweep_changes_amplitude_within_note() {
        // Nota larga (4 beats @ 60 bpm = 4 segundos) con un sweep de
        // volumen desde 0.1 al principio a 1.2 al final. La amplitud
        // del primer cuarto debe ser claramente menor que la del último
        // — sin sample-accurate, ambas serían iguales (se evaluaba sólo
        // en note.start).
        let sr = 44_100;
        let mut score = Score::new(60.0);
        let mut t = Track::new("sweep");
        t.add(ScoreNote::new(Pitch::A4, 0.0, 4.0, 127));
        let mut lane = takiy_core::AutomationLane::default();
        lane.add_point(0.0, 0.1);
        lane.add_point(4.0, 1.2);
        t.volume_automation = Some(lane);
        score.add_track(t);

        let buf = OscRenderer { sample_rate: sr, ..Default::default() }.render(&score);
        // Buffer estéreo interleaved: cada frame = 2 samples.
        let frames = buf.samples.len() / 2;
        let quarter = frames / 4;
        // Peak de cada cuarto (mono — promedio L+R).
        let peak_range = |start: usize, end: usize| {
            buf.samples[start * 2..end * 2]
                .iter()
                .fold(0.0_f32, |a, b| a.max(b.abs()))
        };
        let p0 = peak_range(0, quarter);
        let p3 = peak_range(3 * quarter, frames - 1);
        // El último cuarto debe ser al menos 3× más fuerte que el primero
        // (la curva sube de 0.1 a 1.2 ≈ 12× ganancia, conservador acá
        // contra normalización + envolvente).
        assert!(
            p3 > p0 * 3.0,
            "peak_last={p3} no es significativamente > peak_first={p0}"
        );
    }

    #[test]
    fn rendering_normalizes_when_tracks_clip() {
        // Dos pistas tocando A4 fuerte → suma > 1.0, debe normalizar.
        let mut score = Score::new(60.0);
        for _ in 0..3 {
            let mut t = Track::new("layer");
            t.add(ScoreNote::new(Pitch::A4, 0.0, 1.0, 127));
            score.add_track(t);
        }
        let buf = OscRenderer::default().render(&score);
        assert!(buf.peak() <= 1.0 + 1e-6);
    }
}
