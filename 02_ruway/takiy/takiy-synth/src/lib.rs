//! `takiy-synth` — síntesis: convierte un `Score` en audio PCM.
//!
//! La pieza más fea del stack, a propósito: osciladores básicos
//! (sine/square/saw/triangle) con envolvente ADSR. Suena horrible, pero
//! permite escuchar lo que escribes sin instalar nada.
//!
//! El trabajo serio lo hará luego otro `Renderer`: SoundFonts (SF2/SFZ)
//! con muestras reales, y eventualmente render neural en `takiy-ai`. La
//! interfaz [`Renderer`] es lo que permite intercambiarlos sin tocar el
//! resto de la app.
//!
//! - [`audio`] — `AudioBuffer` mono `f32` y su tasa de muestreo.
//! - [`waveform`] — formas de onda básicas.
//! - [`envelope`] — `Adsr` en segundos.
//! - [`renderer`] — `Renderer` trait + `OscRenderer`.
//! - [`soundfont`] — `SoundFontRenderer` sobre SF2 (rustysynth).
//! - [`soundfont_multi`] — `MultiProgramRenderer`, preset por pista.
//! - [`effects`] — efectos de bus master (hoy: delay).
//! - [`wav`] — escritor WAV PCM 16-bit, sin dependencias.

#![forbid(unsafe_code)]

pub mod audio;
pub mod effects;
pub mod envelope;
pub mod metronome;
pub mod renderer;
pub mod soundfont;
pub mod soundfont_multi;
pub mod waveform;
pub mod wav;

pub use audio::AudioBuffer;
pub use effects::{apply_master_delay, apply_master_reverb};
pub use envelope::Adsr;
pub use metronome::{count_in_samples, mix_clicks, prepend_count_in, Metronome};
pub use renderer::{OscRenderer, Renderer};
pub use soundfont::{LoadError, SoundFontRenderer};
pub use soundfont_multi::MultiProgramRenderer;
pub use waveform::Waveform;
pub use wav::{write_wav, write_wav_to};
