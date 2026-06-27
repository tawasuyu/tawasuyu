//! Selector de backend — el «híbrido configurable».
//!
//! El contrato ([`Transcriptor`]/[`Locutor`]) es el mismo para mock, local y
//! nube; esto sólo **elige la implementación** según config, igual que
//! `pluma-llm` elige `ChatClient` con `LlmConfig{kind}`. Por eso el híbrido no
//! es código nuevo: es un factory sobre el trait.
//!
//! STT y TTS se eligen **por separado** (podés dictar con whisper local y leer
//! con una voz de nube, o al revés). Y como en `pluma-llm`, lo no disponible
//! cae a mock con [`construir_stt_o_mock`] para que los demos arranquen igual.

use std::path::PathBuf;
use std::sync::Arc;

use rimay_voz_core::{Locutor, Transcriptor, VozError};
use rimay_voz_daemon::DaemonClient;
use rimay_voz_mock::{LocutorMock, TranscriptorMock};
use rimay_voz_nube::{LocutorNube, TranscriptorNube};

/// De dónde sale un motor de voz. Las tres variantes cumplen el mismo trait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// Determinista, sin modelo (CI/demos).
    Mock,
    /// Modelo local servido por el `voz-daemon` (carga una vez, socket).
    Local,
    /// Servicio en la nube por HTTP. `proveedor` ej. `"deepgram"`, `"openai"`.
    Nube {
        proveedor: String,
        modelo: Option<String>,
    },
}

impl Backend {
    /// Parsea una cadena de config: `"mock"`, `"local"`, `"nube:deepgram"`,
    /// `"nube:openai:whisper-1"`. Lo desconocido cae a [`Backend::Mock`].
    pub fn parse(s: &str) -> Self {
        let s = s.trim().to_lowercase();
        match s.as_str() {
            "" | "mock" => Backend::Mock,
            "local" => Backend::Local,
            otro => match otro.strip_prefix("nube:") {
                Some(resto) => {
                    let mut it = resto.splitn(2, ':');
                    let proveedor = it.next().unwrap_or_default().to_string();
                    let modelo = it.next().filter(|m| !m.is_empty()).map(str::to_string);
                    Backend::Nube { proveedor, modelo }
                }
                None => Backend::Mock,
            },
        }
    }
}

/// Config del híbrido: backend de STT y de TTS, elegibles por separado.
#[derive(Debug, Clone)]
pub struct VozConfig {
    /// Backend de reconocimiento (dictado).
    pub stt: Backend,
    /// Backend de síntesis (lectura).
    pub tts: Backend,
    /// Override del socket del `voz-daemon` (default: [`socket_por_defecto`]).
    pub socket: Option<PathBuf>,
}

impl Default for VozConfig {
    fn default() -> Self {
        Self {
            stt: Backend::Mock,
            tts: Backend::Mock,
            socket: None,
        }
    }
}

impl VozConfig {
    /// Lee la config del entorno (gemelo de `pluma-llm::from_env`):
    /// `RIMAY_VOZ_STT` y `RIMAY_VOZ_TTS`. Ausentes → [`Backend::Mock`].
    pub fn from_env() -> Self {
        Self {
            stt: std::env::var("RIMAY_VOZ_STT")
                .map(|s| Backend::parse(&s))
                .unwrap_or(Backend::Mock),
            tts: std::env::var("RIMAY_VOZ_TTS")
                .map(|s| Backend::parse(&s))
                .unwrap_or(Backend::Mock),
            socket: None,
        }
    }

    /// Ruta del socket del `voz-daemon`: el override de [`Self::socket`] o la
    /// convención de la suite ([`crate::socket_por_defecto`]).
    fn ruta_socket(&self) -> PathBuf {
        self.socket
            .clone()
            .unwrap_or_else(crate::socket_por_defecto)
    }

    /// Construye el transcriptor (STT) según [`Self::stt`]. Ambas ramas reales
    /// están aterrizadas: **local** → [`rimay_voz_daemon::DaemonClient`] (el
    /// brazo local), **nube** → [`rimay_voz_nube`] (HTTP).
    pub async fn construir_stt(&self) -> Result<Arc<dyn Transcriptor>, VozError> {
        match &self.stt {
            Backend::Mock => Ok(Arc::new(TranscriptorMock::default())),
            Backend::Local => {
                let cli = DaemonClient::connect(self.ruta_socket()).await?;
                Ok(Arc::new(cli))
            }
            Backend::Nube { proveedor, modelo } => match proveedor.as_str() {
                "openai" => {
                    let mut b = TranscriptorNube::openai_from_env()?;
                    if let Some(m) = modelo {
                        b = b.con_modelo(m);
                    }
                    Ok(Arc::new(b))
                }
                otro => Err(VozError::Stt(format!(
                    "STT nube: proveedor «{otro}» no soportado (hoy: openai)"
                ))),
            },
        }
    }

    /// Como [`Self::construir_stt`] pero cae a mock si el backend no está
    /// disponible — para que demos y CI corran sin credenciales ni daemon.
    pub async fn construir_stt_o_mock(&self) -> Arc<dyn Transcriptor> {
        self.construir_stt()
            .await
            .unwrap_or_else(|_| Arc::new(TranscriptorMock::default()))
    }

    /// Construye el locutor (TTS) según [`Self::tts`]. Ver [`Self::construir_stt`].
    pub async fn construir_tts(&self) -> Result<Arc<dyn Locutor>, VozError> {
        match &self.tts {
            Backend::Mock => Ok(Arc::new(LocutorMock)),
            Backend::Local => {
                let cli = DaemonClient::connect(self.ruta_socket()).await?;
                Ok(Arc::new(cli))
            }
            Backend::Nube { proveedor, modelo } => match proveedor.as_str() {
                "openai" => {
                    let mut b = LocutorNube::openai_from_env()?;
                    if let Some(m) = modelo {
                        b = b.con_modelo(m);
                    }
                    Ok(Arc::new(b))
                }
                otro => Err(VozError::Tts(format!(
                    "TTS nube: proveedor «{otro}» no soportado (hoy: openai)"
                ))),
            },
        }
    }

    /// Como [`Self::construir_tts`] pero cae a mock si no está disponible.
    pub async fn construir_tts_o_mock(&self) -> Arc<dyn Locutor> {
        self.construir_tts()
            .await
            .unwrap_or_else(|_| Arc::new(LocutorMock))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn parse_reconoce_las_tres_familias() {
        assert_eq!(Backend::parse("mock"), Backend::Mock);
        assert_eq!(Backend::parse(""), Backend::Mock);
        assert_eq!(Backend::parse("LOCAL"), Backend::Local);
        assert_eq!(
            Backend::parse("nube:deepgram"),
            Backend::Nube { proveedor: "deepgram".into(), modelo: None }
        );
        assert_eq!(
            Backend::parse("nube:openai:whisper-1"),
            Backend::Nube { proveedor: "openai".into(), modelo: Some("whisper-1".into()) }
        );
    }

    #[test]
    fn desconocido_cae_a_mock() {
        assert_eq!(Backend::parse("cualquiera"), Backend::Mock);
    }

    #[tokio::test]
    async fn mock_construye_siempre() {
        let cfg = VozConfig::default();
        assert!(cfg.construir_stt().await.is_ok());
        assert!(cfg.construir_tts().await.is_ok());
    }

    #[tokio::test]
    async fn local_sin_daemon_erra_pero_o_mock_no() {
        // Socket inexistente → DaemonClient::connect falla tras el reintento.
        let local = VozConfig {
            stt: Backend::Local,
            tts: Backend::Local,
            socket: Some(std::env::temp_dir().join("voz-no-existe-jamas.sock")),
        };
        assert!(local.construir_stt().await.is_err());
        // pero el _o_mock nunca falla: cae a mock cuando el daemon no está.
        let _ = local.construir_stt_o_mock().await;
    }

    #[tokio::test]
    async fn nube_proveedor_desconocido_erra() {
        let cfg = VozConfig {
            stt: Backend::Nube { proveedor: "marciano".into(), modelo: None },
            tts: Backend::Nube { proveedor: "marciano".into(), modelo: None },
            socket: None,
        };
        assert!(cfg.construir_stt().await.is_err());
        assert!(cfg.construir_tts().await.is_err());
        // el _o_mock sigue cayendo a mock pase lo que pase
        let _ = cfg.construir_tts_o_mock().await;
    }

    #[tokio::test]
    async fn nube_openai_depende_de_la_credencial() {
        let cfg = VozConfig {
            stt: Backend::Nube { proveedor: "openai".into(), modelo: None },
            tts: Backend::Mock,
            socket: None,
        };
        // Con OPENAI_API_KEY presente, construye; sin ella, erra explícito.
        // Ningún caso hace red (el constructor sólo arma el cliente HTTP).
        match std::env::var("OPENAI_API_KEY") {
            Ok(_) => assert!(cfg.construir_stt().await.is_ok()),
            Err(_) => assert!(cfg.construir_stt().await.is_err()),
        }
    }
}
