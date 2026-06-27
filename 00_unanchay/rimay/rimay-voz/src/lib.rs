//! `rimay-voz` — la fachada que los consumidores de voz importan cuando no
//! quieren ensamblar a mano los crates plugin ni reinventar la convención del
//! socket. Gemela de [`rimay-verbo`](../rimay-verbo): si verbo es *entender*
//! texto, voz es *oír y decir*.
//!
//! Re-exporta todo el contrato y la lógica general de [`rimay-voz-core`]
//! (traits [`Transcriptor`]/[`Locutor`], la [`Maquina`] de escucha
//! manos-libres, [`prosodia`](rimay_voz_core::prosodia), la política de
//! [`lectura`](rimay_voz_core::lectura)) más los backends mock — así un
//! consumidor depende de **un** crate:
//!
//! ```no_run
//! use std::sync::Arc;
//! use rimay_voz::{Maquina, ConfigVoz, Evento, Reaccion, Transcriptor};
//!
//! # async fn ejemplo() -> Result<(), Box<dyn std::error::Error>> {
//! let stt = rimay_voz::stt_mock();           // luego: daemon real
//! let mut escucha = Maquina::new(ConfigVoz::default());
//!
//! // El host transcribe un fragmento y se lo pasa a la máquina.
//! let audio = rimay_voz::Audio::new(vec![0; 16_000], 16_000);
//! let t = stt.transcribir(&audio).await?;
//! if let Reaccion::Desperto = escucha.avanzar(Evento::Transcript(t.texto)) {
//!     // se reconoció el llamado: abrir el dictado
//! }
//! # Ok(()) }
//! ```
//!
//! El daemon (`rimay-voz-daemon`, pendiente) cargará el modelo una vez y lo
//! servirá por socket — igual que `verbo-daemon`. La convención de ese socket
//! ya vive acá en [`socket_por_defecto`] para que cuando el daemon llegue sea
//! la misma decisión partida en dos archivos, no dos.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;

mod config;
pub use config::{Backend, VozConfig};

pub use rimay_voz_core::{
    clasificar, debe_leer, detectar_llamado, lectura, maquina, prosodia, Audio, ConfigVoz,
    EstadoVoz, Evento, Intencion, Locutor, Maquina, Politica, Rasgos, Reaccion, TipoBloque,
    Transcripcion, Transcriptor, VozError,
};
pub use rimay_voz_mock::{LocutorMock, TranscriptorMock};
pub use rimay_voz_nube::{LocutorNube, TranscriptorNube};

/// Un transcriptor mock determinista, listo para envolver en la `Maquina` sin
/// daemon ni modelo. Por default reconoce el llamado `"shuma"`.
pub fn stt_mock() -> Arc<dyn Transcriptor> {
    Arc::new(TranscriptorMock::default())
}

/// Un locutor mock determinista (sintetiza silencio proporcional al texto).
pub fn tts_mock() -> Arc<dyn Locutor> {
    Arc::new(LocutorMock)
}

/// Ruta del socket Unix donde escuchará el futuro `voz-daemon`.
///
/// Convención de la suite (idéntica a la de `verbo`, sólo cambia el nombre):
/// 1. `$XDG_RUNTIME_DIR/voz.sock` si la variable está.
/// 2. Fallback: `/tmp/voz-{uid}.sock`, prefijado por UID.
pub fn socket_por_defecto() -> PathBuf {
    socket_desde(std::env::var("XDG_RUNTIME_DIR").ok().as_deref(), uid_actual())
}

/// Lógica pura de la convención, separada del acceso a env para testearla sin
/// tocar variables globales.
fn socket_desde(xdg_runtime_dir: Option<&str>, uid: u32) -> PathBuf {
    if let Some(xdg) = xdg_runtime_dir {
        return PathBuf::from(xdg).join("voz.sock");
    }
    PathBuf::from(format!("/tmp/voz-{uid}.sock"))
}

/// UID vía `/proc/self/loginuid`; 1000 si falla (misma heurística que verbo).
fn uid_actual() -> u32 {
    std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&u| u != u32::MAX)
        .unwrap_or(1000)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn socket_usa_xdg_runtime_dir_cuando_existe() {
        assert_eq!(
            socket_desde(Some("/run/user/1234"), 1234),
            PathBuf::from("/run/user/1234/voz.sock")
        );
    }

    #[test]
    fn socket_cae_a_tmp_por_uid_sin_xdg() {
        assert_eq!(socket_desde(None, 42), PathBuf::from("/tmp/voz-42.sock"));
    }

    #[tokio::test]
    async fn stt_mock_reconoce_el_llamado() {
        let stt = stt_mock();
        let t = stt
            .transcribir(&Audio::new(vec![0; 100], 16_000))
            .await
            .unwrap();
        assert_eq!(t.texto, "shuma");
    }
}
