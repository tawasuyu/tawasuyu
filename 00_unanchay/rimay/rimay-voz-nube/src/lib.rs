//! `rimay-voz-nube` — la rama **Nube** del híbrido de voz, sobre HTTP.
//!
//! Cumple los mismos traits que el mock y (mañana) el daemon local —
//! [`Transcriptor`] y [`Locutor`] —, así que `VozConfig` los intercambia sin
//! que el consumidor note la diferencia. Es el gemelo de los backends de nube
//! de `pluma-llm`: una credencial por agente, **cero modelos en disco**.
//!
//! Habla la shape OpenAI, que es la que casi todos los servicios de audio
//! imitan:
//!
//! - **STT** → `POST {base}/audio/transcriptions` (Whisper). El audio PCM se
//!   empaqueta como un WAV en memoria y se sube por `multipart/form-data`. La
//!   respuesta es `{"text": "..."}`.
//! - **TTS** → `POST {base}/audio/speech`. Pedimos `response_format: "pcm"`
//!   (16-bit LE mono a 24 kHz), que decodificamos directo a [`Audio`] sin pasar
//!   por mp3/opus.
//!
//! El `base` es configurable, así que cualquier proxy OpenAI-compatible
//! (Groq, un gateway propio) sirve sin tocar el código.
//!
//! ```no_run
//! # use rimay_voz_nube::{TranscriptorNube, LocutorNube};
//! # fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let stt = TranscriptorNube::openai_from_env()?;      // lee OPENAI_API_KEY
//! let tts = LocutorNube::openai_from_env()?.con_voz("nova");
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]

use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};
use rimay_voz_core::{Audio, Locutor, Transcripcion, Transcriptor, VozError};

const TIMEOUT_DEFAULT_SECS: u64 = 60;
const OPENAI_BASE: &str = "https://api.openai.com/v1";
const OPENAI_ENV: &str = "OPENAI_API_KEY";
const STT_MODELO_DEFAULT: &str = "whisper-1";
const TTS_MODELO_DEFAULT: &str = "tts-1";
const TTS_VOZ_DEFAULT: &str = "alloy";
/// OpenAI sirve el `pcm` como 16-bit LE mono a 24 kHz.
const TTS_HZ: u32 = 24_000;

fn cliente_http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_DEFAULT_SECS))
        .build()
        .expect("reqwest client")
}

fn headers_auth(api_key: &str) -> Result<HeaderMap, VozError> {
    let mut h = HeaderMap::new();
    let val = HeaderValue::from_str(&format!("Bearer {api_key}"))
        .map_err(|_| VozError::Stt("api key con bytes inválidos".into()))?;
    h.insert("authorization", val);
    Ok(h)
}

// ----------------------------------------------------------------------------
// STT — transcripción por nube
// ----------------------------------------------------------------------------

/// Backend de **STT** contra `/audio/transcriptions` (Whisper y compatibles).
pub struct TranscriptorNube {
    http: reqwest::Client,
    base: String,
    api_key: String,
    modelo: String,
}

impl TranscriptorNube {
    /// Constructor general: base + credencial + modelo.
    pub fn nuevo(
        base: impl Into<String>,
        api_key: impl Into<String>,
        modelo: impl Into<String>,
    ) -> Self {
        Self {
            http: cliente_http(),
            base: base.into(),
            api_key: api_key.into(),
            modelo: modelo.into(),
        }
    }

    /// Preset OpenAI: lee `OPENAI_API_KEY`, modelo `whisper-1`.
    pub fn openai_from_env() -> Result<Self, VozError> {
        let api_key = std::env::var(OPENAI_ENV).map_err(|_| {
            VozError::Stt(format!("falta la credencial {OPENAI_ENV} para STT de nube"))
        })?;
        Ok(Self::nuevo(OPENAI_BASE, api_key, STT_MODELO_DEFAULT))
    }

    /// Encadenable: cambia el modelo (ej. `"gpt-4o-transcribe"`).
    pub fn con_modelo(mut self, modelo: impl Into<String>) -> Self {
        self.modelo = modelo.into();
        self
    }
}

#[async_trait]
impl Transcriptor for TranscriptorNube {
    fn modelo(&self) -> &str {
        &self.modelo
    }

    async fn transcribir(&self, audio: &Audio) -> Result<Transcripcion, VozError> {
        let wav = wav_de_pcm(audio);
        let parte = reqwest::multipart::Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| VozError::Stt(format!("mime: {e}")))?;
        let form = reqwest::multipart::Form::new()
            .text("model", self.modelo.clone())
            .text("response_format", "json")
            .part("file", parte);

        let resp = self
            .http
            .post(format!("{}/audio/transcriptions", self.base))
            .headers(headers_auth(&self.api_key)?)
            .multipart(form)
            .send()
            .await
            .map_err(|e| VozError::Stt(format!("POST transcriptions: {e}")))?;

        let status = resp.status();
        let body = resp
            .bytes()
            .await
            .map_err(|e| VozError::Stt(format!("leer body: {e}")))?;
        if !status.is_success() {
            return Err(VozError::Stt(format!(
                "HTTP {status}: {}",
                mensaje_error(&body)
            )));
        }

        let parsed: RespuestaStt = serde_json::from_slice(&body)
            .map_err(|e| VozError::Stt(format!("parseo response: {e}")))?;
        Ok(Transcripcion::nueva(parsed.text))
    }
}

// ----------------------------------------------------------------------------
// TTS — síntesis por nube
// ----------------------------------------------------------------------------

/// Backend de **TTS** contra `/audio/speech`. Pide PCM crudo y lo entrega
/// como [`Audio`] a [`TTS_HZ`].
pub struct LocutorNube {
    http: reqwest::Client,
    base: String,
    api_key: String,
    modelo: String,
    voz: String,
}

impl LocutorNube {
    /// Constructor general: base + credencial + modelo + voz.
    pub fn nuevo(
        base: impl Into<String>,
        api_key: impl Into<String>,
        modelo: impl Into<String>,
        voz: impl Into<String>,
    ) -> Self {
        Self {
            http: cliente_http(),
            base: base.into(),
            api_key: api_key.into(),
            modelo: modelo.into(),
            voz: voz.into(),
        }
    }

    /// Preset OpenAI: lee `OPENAI_API_KEY`, modelo `tts-1`, voz `alloy`.
    pub fn openai_from_env() -> Result<Self, VozError> {
        let api_key = std::env::var(OPENAI_ENV).map_err(|_| {
            VozError::Tts(format!("falta la credencial {OPENAI_ENV} para TTS de nube"))
        })?;
        Ok(Self::nuevo(OPENAI_BASE, api_key, TTS_MODELO_DEFAULT, TTS_VOZ_DEFAULT))
    }

    /// Encadenable: cambia el modelo (ej. `"tts-1-hd"`).
    pub fn con_modelo(mut self, modelo: impl Into<String>) -> Self {
        self.modelo = modelo.into();
        self
    }

    /// Encadenable: cambia la voz (ej. `"nova"`, `"onyx"`).
    pub fn con_voz(mut self, voz: impl Into<String>) -> Self {
        self.voz = voz.into();
        self
    }
}

#[async_trait]
impl Locutor for LocutorNube {
    fn modelo(&self) -> &str {
        &self.modelo
    }

    async fn sintetizar(&self, texto: &str) -> Result<Audio, VozError> {
        let payload = serde_json::json!({
            "model": self.modelo,
            "voice": self.voz,
            "input": texto,
            "response_format": "pcm",
        });

        let resp = self
            .http
            .post(format!("{}/audio/speech", self.base))
            .headers(headers_auth(&self.api_key)?)
            .json(&payload)
            .send()
            .await
            .map_err(|e| VozError::Tts(format!("POST speech: {e}")))?;

        let status = resp.status();
        let body = resp
            .bytes()
            .await
            .map_err(|e| VozError::Tts(format!("leer body: {e}")))?;
        if !status.is_success() {
            return Err(VozError::Tts(format!(
                "HTTP {status}: {}",
                mensaje_error(&body)
            )));
        }

        Ok(pcm_de_bytes(&body, TTS_HZ))
    }
}

// ----------------------------------------------------------------------------
// Helpers puros — codec PCM↔WAV, testeable sin red
// ----------------------------------------------------------------------------

/// Empaqueta el PCM mono 16-bit de un [`Audio`] como un WAV en memoria.
/// Whisper acepta WAV directamente, así evitamos depender de un encoder.
fn wav_de_pcm(audio: &Audio) -> Vec<u8> {
    let n_bytes_datos = (audio.muestras.len() * 2) as u32;
    let byte_rate = audio.hz * 2; // mono · 16-bit
    let mut w = Vec::with_capacity(44 + n_bytes_datos as usize);

    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + n_bytes_datos).to_le_bytes());
    w.extend_from_slice(b"WAVE");

    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes()); // tamaño del subchunk fmt
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // canales: mono
    w.extend_from_slice(&audio.hz.to_le_bytes());
    w.extend_from_slice(&byte_rate.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes()); // block align: 2 bytes/muestra
    w.extend_from_slice(&16u16.to_le_bytes()); // bits por muestra

    w.extend_from_slice(b"data");
    w.extend_from_slice(&n_bytes_datos.to_le_bytes());
    for m in &audio.muestras {
        w.extend_from_slice(&m.to_le_bytes());
    }
    w
}

/// Decodifica bytes PCM crudos (16-bit LE mono) a un [`Audio`]. Un byte
/// colgante al final (longitud impar) se descarta.
fn pcm_de_bytes(bytes: &[u8], hz: u32) -> Audio {
    let muestras = bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    Audio::new(muestras, hz)
}

/// Extrae el mensaje de un error HTTP: probamos `{"error":{"message":...}}`
/// (shape OpenAI) y caemos al body como texto plano.
fn mensaje_error(body: &[u8]) -> String {
    #[derive(serde::Deserialize)]
    struct Sobre {
        error: Detalle,
    }
    #[derive(serde::Deserialize)]
    struct Detalle {
        message: String,
    }
    match serde_json::from_slice::<Sobre>(body) {
        Ok(s) => s.error.message,
        Err(_) => String::from_utf8_lossy(body).into_owned(),
    }
}

#[derive(serde::Deserialize)]
struct RespuestaStt {
    text: String,
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn wav_tiene_cabecera_riff_y_largo_correcto() {
        let audio = Audio::new(vec![0, 1, -1, 32_767, -32_768], 16_000);
        let wav = wav_de_pcm(&audio);
        // 44 de cabecera + 5 muestras · 2 bytes.
        assert_eq!(wav.len(), 44 + 10);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        // hz embebido en el subchunk fmt (offset 24).
        assert_eq!(&wav[24..28], &16_000u32.to_le_bytes());
        // tamaño del chunk data (offset 40).
        assert_eq!(&wav[40..44], &10u32.to_le_bytes());
    }

    #[test]
    fn pcm_round_trip_por_los_datos_del_wav() {
        let audio = Audio::new(vec![0, 123, -456, 32_000], 24_000);
        let wav = wav_de_pcm(&audio);
        // Releer sólo la sección de datos (tras los 44 de cabecera).
        let vuelta = pcm_de_bytes(&wav[44..], 24_000);
        assert_eq!(vuelta.muestras, audio.muestras);
        assert_eq!(vuelta.hz, 24_000);
    }

    #[test]
    fn pcm_descarta_byte_colgante() {
        // 3 bytes → 1 muestra (los 2 primeros), el tercero se descarta.
        let a = pcm_de_bytes(&[0x10, 0x20, 0x99], 24_000);
        assert_eq!(a.muestras, vec![i16::from_le_bytes([0x10, 0x20])]);
    }

    #[test]
    fn mensaje_error_lee_shape_openai() {
        let body = r#"{"error":{"message":"clave invalida","type":"auth"}}"#.as_bytes();
        assert_eq!(mensaje_error(body), "clave invalida");
    }

    #[test]
    fn mensaje_error_cae_a_texto_plano() {
        assert_eq!(mensaje_error(b"503 upstream caido"), "503 upstream caido");
    }

    #[test]
    fn from_env_sin_credencial_erra_explicito() {
        // Sin OPENAI_API_KEY en el entorno del test, debe errar (no panic).
        // (Si el entorno la tuviera, el constructor simplemente la toma — no
        //  hace red — así que el test sigue siendo válido en ambos casos.)
        match std::env::var(OPENAI_ENV) {
            Err(_) => {
                assert!(TranscriptorNube::openai_from_env().is_err());
                assert!(LocutorNube::openai_from_env().is_err());
            }
            Ok(_) => {
                assert!(TranscriptorNube::openai_from_env().is_ok());
            }
        }
    }
}
