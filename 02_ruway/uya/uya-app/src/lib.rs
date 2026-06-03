// =============================================================================
//  uya-app — el pegamento de la videollamada.
// -----------------------------------------------------------------------------
//  Re-exporta el modelo de `uya-core` y suma lo que toca al mundo real:
//    · `Enlace`  — transporte TCP punto-a-punto (ver `enlace`).
//    · `EventoUya` — lo que la UI recibe por su canal `std::mpsc`.
//    · `iniciar_camara` — el hilo de captura (TestCard o webcam real).
//
//  Patrón calcado de `ayni-app`: `Enlace::abrir` devuelve `(Enlace, Receiver)`;
//  un hilo de la UI hace `for ev in rx { handle.dispatch(Msg::Red(ev)) }`.
// =============================================================================

mod audio;
mod captura;
mod enlace;
mod lan;
mod video;

pub use audio::{iniciar_microfono, iniciar_reproduccion, MezclaRemota};
pub use captura::iniciar_camara;
pub use enlace::Enlace;
pub use lan::iniciar_baliza_lan;
pub use media_audio_cpal::AudioSink;
pub use uya_core::{
    hex_corto, id_desde_nombre, FormatoCuadro, Paquete, Participante, ParticipanteId, Sala,
};

use std::sync::Arc;

/// Lo que ocurre en la llamada, tal como lo ve la UI. El transporte y la
/// captura empujan estos eventos al canal que la UI drena en su bucle Elm.
#[derive(Clone, Debug)]
pub enum EventoUya {
    /// Un participante (yo o remoto) entró / se presentó.
    Entra {
        id: ParticipanteId,
        nombre: String,
    },
    /// Un participante se fue (cuelgue o desconexión).
    Sale { id: ParticipanteId },
    /// Cambió el estado de medios de un participante.
    Estado {
        id: ParticipanteId,
        camara: bool,
        microfono: bool,
    },
    /// Llegó un cuadro de video de un participante. El RGBA va en `Arc` para
    /// que la UI lo clone barato cada render.
    Cuadro {
        id: ParticipanteId,
        ancho: u16,
        alto: u16,
        rgba: Arc<Vec<u8>>,
    },
    /// Llegó un mensaje de texto de la charla. `nombre` viene resuelto desde el
    /// roster del receptor (el que lo recibió por `Hola`), con respaldo al hex
    /// corto del id si todavía no se conoce.
    Mensaje {
        id: ParticipanteId,
        nombre: String,
        texto: String,
    },
    /// Cambió la actividad de voz de un participante (empezó / dejó de hablar).
    /// Lo emite el detector de voz local (mi micrófono) y el del lector por
    /// cada par; la UI lo usa para resaltar al que habla. Es un flanco, no un
    /// estado continuo.
    Voz { id: ParticipanteId, hablando: bool },
}
