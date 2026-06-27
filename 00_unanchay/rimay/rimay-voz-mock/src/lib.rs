//! `voz-mock` — backends deterministas de STT/TTS, sin modelos ni red.
//!
//! Para CI y demos: los consumidores cablean el contrato real ([`Transcriptor`]
//! / [`Locutor`]) contra estos mocks y todo corre sin descargar nada. Gemelo de
//! `rimay-verbo-mock`.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use rimay_voz_core::{Audio, Locutor, Transcriptor, Transcripcion, VozError};

/// STT mock: devuelve siempre un texto fijo, configurable. Default `"shuma"`,
/// así un demo de escucha manos-libres despierta sin audio real.
#[derive(Debug, Clone)]
pub struct TranscriptorMock {
    texto: String,
}

impl Default for TranscriptorMock {
    fn default() -> Self {
        Self { texto: "shuma".to_string() }
    }
}

impl TranscriptorMock {
    /// Mock que transcribe todo fragmento como `texto` (útil para simular
    /// «shuma, abrí cosmos» en un test).
    pub fn con_texto(texto: impl Into<String>) -> Self {
        Self { texto: texto.into() }
    }
}

#[async_trait]
impl Transcriptor for TranscriptorMock {
    fn modelo(&self) -> &str {
        "mock-stt"
    }

    async fn transcribir(&self, _audio: &Audio) -> Result<Transcripcion, VozError> {
        Ok(Transcripcion {
            texto: self.texto.clone(),
            confianza: Some(1.0),
        })
    }
}

/// TTS mock: sintetiza **silencio** de duración proporcional al largo del texto
/// (≈ 60 ms por carácter a 22 050 Hz). Determinista — para verificar el
/// cableado sin escuchar nada.
#[derive(Debug, Clone, Default)]
pub struct LocutorMock;

#[async_trait]
impl Locutor for LocutorMock {
    fn modelo(&self) -> &str {
        "mock-tts"
    }

    async fn sintetizar(&self, texto: &str) -> Result<Audio, VozError> {
        const HZ: u32 = 22_050;
        const MS_POR_CHAR: usize = 60;
        let n = texto.chars().count() * MS_POR_CHAR * HZ as usize / 1000;
        Ok(Audio::new(vec![0; n], HZ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stt_mock_devuelve_el_llamado_por_default() {
        let t = TranscriptorMock::default();
        let r = t.transcribir(&Audio::new(vec![0; 100], 16_000)).await.unwrap();
        assert_eq!(r.texto, "shuma");
    }

    #[tokio::test]
    async fn stt_mock_configurable() {
        let t = TranscriptorMock::con_texto("shuma abrí cosmos");
        let r = t.transcribir(&Audio::new(vec![], 16_000)).await.unwrap();
        assert_eq!(r.texto, "shuma abrí cosmos");
    }

    #[tokio::test]
    async fn tts_mock_dura_proporcional_al_texto() {
        let l = LocutorMock;
        let corto = l.sintetizar("hola").await.unwrap();
        let largo = l.sintetizar("hola mundo largo").await.unwrap();
        assert!(largo.duracion_s() > corto.duracion_s());
        assert_eq!(corto.hz, 22_050);
    }
}
