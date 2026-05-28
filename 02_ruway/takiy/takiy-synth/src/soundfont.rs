//! `SoundFontRenderer` — render del `Score` usando un SoundFont SF2.
//!
//! El paso natural arriba del oscilador: muestras reales de instrumentos
//! (piano, guitarra, cuerdas, …). Usa `rustysynth`, un sintetizador SF2
//! puro Rust, así que sigue sin tocar audio nativo: produce un buffer y
//! ya está. La síntesis tiempo real la añade quien consuma el buffer.
//!
//! Necesita un archivo `.sf2` (FluidR3, GeneralUser GS, TimGM6mb, …).
//! No se bundlea ninguno con el crate; el usuario apunta el suyo.

use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::Arc;

use rustysynth::{SoundFont, SoundFontError, Synthesizer, SynthesizerSettings};
use takiy_core::Score;

use crate::audio::AudioBuffer;
use crate::renderer::Renderer;

/// Errores al cargar un SoundFont.
#[derive(Debug)]
pub enum LoadError {
    Io(io::Error),
    SoundFont(SoundFontError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "no se pudo abrir el SoundFont: {e}"),
            LoadError::SoundFont(e) => write!(f, "SoundFont inválido: {e}"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Io(e) => Some(e),
            LoadError::SoundFont(_) => None,
        }
    }
}

impl From<io::Error> for LoadError {
    fn from(e: io::Error) -> Self {
        LoadError::Io(e)
    }
}

impl From<SoundFontError> for LoadError {
    fn from(e: SoundFontError) -> Self {
        LoadError::SoundFont(e)
    }
}

/// Renderer que usa un SoundFont SF2 para todas las pistas.
///
/// MVP: una sola voz/preset para toda la partitura. Si querés timbres
/// distintos por pista, eso vendrá en una capa superior (un renderer
/// "multi-preset" que envuelve a varios `SoundFontRenderer`).
#[derive(Clone)]
pub struct SoundFontRenderer {
    sound_font: Arc<SoundFont>,
    /// Tasa de muestreo de salida. 44100 por defecto.
    pub sample_rate: u32,
    /// Preset General MIDI `0..=127`. `0` = Acoustic Grand Piano.
    pub program: u8,
    /// Segundos extra al final para que las notas terminen su release.
    pub tail_seconds: f32,
}

impl std::fmt::Debug for SoundFontRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundFontRenderer")
            .field("sample_rate", &self.sample_rate)
            .field("program", &self.program)
            .field("tail_seconds", &self.tail_seconds)
            .finish_non_exhaustive()
    }
}

impl SoundFontRenderer {
    /// Carga un SoundFont desde un archivo `.sf2` y devuelve un renderer
    /// con preset 0 (piano) a 44.1 kHz.
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, LoadError> {
        let mut f = File::open(path)?;
        let sf = SoundFont::new(&mut f)?;
        Ok(Self {
            sound_font: Arc::new(sf),
            sample_rate: 44_100,
            program: 0,
            tail_seconds: 2.0,
        })
    }

    /// Cambia el preset GM. Encadenable.
    pub fn with_program(mut self, program: u8) -> Self {
        self.program = program;
        self
    }

    /// Cambia la tasa de muestreo. Encadenable.
    pub fn with_sample_rate(mut self, sample_rate: u32) -> Self {
        self.sample_rate = sample_rate;
        self
    }
}

#[derive(Debug, Clone, Copy)]
enum Event {
    On { key: i32, velocity: i32 },
    Off { key: i32 },
}

impl Renderer for SoundFontRenderer {
    fn render(&self, score: &Score) -> AudioBuffer {
        let settings = SynthesizerSettings::new(self.sample_rate as i32);
        let mut synth = Synthesizer::new(&self.sound_font, &settings)
            .expect("settings.sample_rate ya está validado");

        // Program change en el canal 0 — todas las notas usan este preset.
        const CHANNEL: i32 = 0;
        const PROGRAM_CHANGE: i32 = 0xC0;
        synth.process_midi_message(CHANNEL, PROGRAM_CHANGE, self.program as i32, 0);

        // Traduce ScoreNote → eventos en samples absolutos.
        let bpm = score.tempo_bpm.max(1e-6);
        let sec_per_beat = 60.0 / bpm;
        let sr = self.sample_rate as f32;

        let mut events: Vec<(usize, Event)> = Vec::new();
        for track in score.tracks() {
            for note in track.notes() {
                let on = (note.start * sec_per_beat * sr) as usize;
                let off = (note.end() * sec_per_beat * sr) as usize;
                let key = note.pitch.midi() as i32;
                events.push((on, Event::On { key, velocity: note.velocity as i32 }));
                events.push((off.max(on + 1), Event::Off { key }));
            }
        }
        // Estable: si dos eventos caen en el mismo sample, mantienen su orden.
        events.sort_by_key(|(s, _)| *s);

        let last_event = events.last().map(|(s, _)| *s).unwrap_or(0);
        let total = last_event + (self.tail_seconds * sr).ceil() as usize;

        let mut left = vec![0.0_f32; total];
        let mut right = vec![0.0_f32; total];

        // Renderiza por tramos: render hasta el siguiente evento, aplica, repite.
        let mut cursor = 0usize;
        for (sample_idx, ev) in events {
            let target = sample_idx.min(total);
            if target > cursor {
                synth.render(&mut left[cursor..target], &mut right[cursor..target]);
                cursor = target;
            }
            match ev {
                Event::On { key, velocity } => synth.note_on(CHANNEL, key, velocity),
                Event::Off { key } => synth.note_off(CHANNEL, key),
            }
        }
        if cursor < total {
            synth.render(&mut left[cursor..total], &mut right[cursor..total]);
        }

        // Estéreo interleaved: [L0, R0, L1, R1, …]. Mantiene la imagen
        // estéreo del SoundFont sin la pérdida del promedio mono.
        let mut samples = Vec::with_capacity(total * 2);
        for i in 0..total {
            samples.push(left[i]);
            samples.push(right[i]);
        }
        let mut buf = AudioBuffer::from_stereo(self.sample_rate, samples);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_io_error() {
        let err = SoundFontRenderer::from_path("/nonexistent/whatever.sf2").unwrap_err();
        assert!(matches!(err, LoadError::Io(_)));
    }

    #[test]
    fn invalid_file_is_soundfont_error() {
        let mut tmp = std::env::temp_dir();
        tmp.push("takiy-not-an-sf2.bin");
        std::fs::write(&tmp, b"definitely not a soundfont").unwrap();
        let err = SoundFontRenderer::from_path(&tmp).unwrap_err();
        let _ = std::fs::remove_file(&tmp);
        assert!(matches!(err, LoadError::SoundFont(_)));
    }

    // Test de integración: sólo corre si TAKIY_SF2 apunta a un .sf2 real.
    // Verifica que un Score se rinda a audio con energía.
    #[test]
    fn renders_with_real_soundfont_if_provided() {
        let Ok(sf_path) = std::env::var("TAKIY_SF2") else {
            eprintln!("(skip) TAKIY_SF2 no está seteado");
            return;
        };
        use takiy_core::{Pitch, ScoreNote, Track};

        let renderer = SoundFontRenderer::from_path(&sf_path).expect("SF2 carga");
        let mut score = Score::new(60.0);
        let mut t = Track::new("a");
        t.add(ScoreNote::new(Pitch::A4, 0.0, 1.0, 100));
        score.add_track(t);

        let buf = renderer.render(&score);
        assert!(buf.peak() > 0.01, "el SoundFont debe producir señal audible");
    }
}
