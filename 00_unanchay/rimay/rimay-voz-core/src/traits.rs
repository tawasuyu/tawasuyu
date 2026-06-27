//! El contrato STT/TTS — model-agnostic, como el `Provider` de verbo.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Audio PCM mono de 16 bits + su frecuencia de muestreo. Es el formato que
/// cruza la frontera entre el host (cpal) y los backends de voz.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Audio {
    /// Muestras PCM mono, 16-bit con signo.
    pub muestras: Vec<i16>,
    /// Frecuencia de muestreo en Hz (ej. 16_000 para STT, 22_050 para TTS).
    pub hz: u32,
}

impl Audio {
    pub fn new(muestras: Vec<i16>, hz: u32) -> Self {
        Self { muestras, hz }
    }

    /// Duración en segundos (0 si `hz` es 0).
    pub fn duracion_s(&self) -> f32 {
        if self.hz == 0 {
            0.0
        } else {
            self.muestras.len() as f32 / self.hz as f32
        }
    }
}

/// Resultado de transcribir un fragmento de audio.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcripcion {
    /// Texto reconocido (vacío si el fragmento no tenía habla).
    pub texto: String,
    /// Confianza `[0,1]` si el backend la reporta; `None` si no.
    pub confianza: Option<f32>,
}

impl Transcripcion {
    pub fn nueva(texto: impl Into<String>) -> Self {
        Self { texto: texto.into(), confianza: None }
    }
}

/// Falla de una operación de voz.
#[derive(Debug, thiserror::Error)]
pub enum VozError {
    #[error("backend de STT: {0}")]
    Stt(String),
    #[error("backend de TTS: {0}")]
    Tts(String),
    #[error("audio inválido: {0}")]
    Audio(String),
}

/// **STT** — convierte audio en texto. Cada backend (whisper local, nube, mock)
/// lo cumple; el consumidor es indistinguible del backend que tenga detrás.
#[async_trait]
pub trait Transcriptor: Send + Sync {
    /// Etiqueta del modelo/backend, para rotular en la UI.
    fn modelo(&self) -> &str;

    /// Transcribe un fragmento de audio.
    async fn transcribir(&self, audio: &Audio) -> Result<Transcripcion, VozError>;
}

/// **TTS** — convierte texto en audio. Cada backend (piper local, nube, mock)
/// lo cumple.
#[async_trait]
pub trait Locutor: Send + Sync {
    /// Etiqueta del modelo/voz, para rotular en la UI.
    fn modelo(&self) -> &str;

    /// Sintetiza voz para `texto`.
    async fn sintetizar(&self, texto: &str) -> Result<Audio, VozError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duracion_de_un_segundo() {
        let a = Audio::new(vec![0; 16_000], 16_000);
        assert!((a.duracion_s() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hz_cero_no_divide_por_cero() {
        assert_eq!(Audio::new(vec![1, 2, 3], 0).duracion_s(), 0.0);
    }
}
