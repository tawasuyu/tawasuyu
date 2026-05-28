//! `Renderer` — convierte un `Score` en `AudioBuffer`.
//!
//! La interfaz es lo importante: hoy hay un solo backend
//! ([`OscRenderer`], osciladores feos), mañana entrarán SoundFonts y
//! render neural sin que la app de composición tenga que enterarse.

use takiy_core::{Score, ScoreNote};

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
            let gain = track.volume.max(0.0);
            let (gl, gr) = track.pan_gains();
            for note in track.notes() {
                self.mix_note_stereo(note, sec_per_beat, &mut buf.samples, gain, gl, gr);
            }
        }

        buf.normalize_if_clipping();
        buf
    }
}

impl OscRenderer {
    fn mix_note_stereo(
        &self,
        note: &ScoreNote,
        sec_per_beat: f32,
        buf: &mut [f32],
        gain: f32,
        gl: f32,
        gr: f32,
    ) {
        let freq = note.pitch.frequency();
        let start_sec = note.start * sec_per_beat;
        let dur_sec = note.duration * sec_per_beat;
        let end_sec = start_sec + dur_sec + self.envelope.release;
        let amp = (note.velocity as f32 / 127.0) * gain;

        let sr = self.sample_rate as f32;
        // Estéreo interleaved: cada cuadro ocupa 2 samples (L, R).
        let n_frames = buf.len() / 2;
        let start_frame = (start_sec * sr) as usize;
        let end_frame = ((end_sec * sr).ceil() as usize).min(n_frames);
        if start_frame >= end_frame {
            return;
        }

        let inv_sr = 1.0 / sr;
        for f in start_frame..end_frame {
            let t = (f - start_frame) as f32 * inv_sr;
            let phase = t * freq;
            let env = self.envelope.level(t, dur_sec);
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
