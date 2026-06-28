//! El lazo de escucha — **puro de hardware**, testeable sin micrófono.
//!
//! Recibe muestras `i16` mono ya a la tasa objetivo (el driver de captura las
//! prepara) y las hace pasar por la cadena de `rimay-voz`:
//!
//! ```text
//!   muestras → [framing] → Vad → (utterance) → STT → Maquina → EventoEscucha
//! ```
//!
//! No abre dispositivos ni corre `tokio`: acumula, segmenta, transcribe y
//! avanza la máquina. El driver (`microfono`) lo alimenta; un test lo alimenta
//! con muestras sintéticas y verifica los eventos por texto.

use std::sync::Arc;

use rimay_voz::{
    ConfigVad, ConfigVoz, DetectorEnergia, EstadoVoz, Evento, Maquina, Reaccion, SalidaVad,
    Transcriptor, Vad,
};

/// Lo que el lazo le reporta a la app por cada bloque de audio procesado. Es la
/// traducción de las reacciones de la [`Maquina`] (+ el borde de inicio del
/// VAD) a algo que la UI dispatcha como `Msg`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventoEscucha {
    /// El VAD detectó arranque de voz (útil para un indicador «escuchando»).
    Escuchando,
    /// Se reconoció el llamado; la escucha quedó abierta.
    Desperto,
    /// Texto a poner en el input (dictado).
    Dictar(String),
    /// Volvió a dormirse por silencio.
    SeDurmio,
}

/// Frame objetivo: 30 ms a 16 kHz = 480 muestras (lo que espera el VAD/whisper).
const HZ_OBJETIVO: u32 = 16_000;
const MUESTRAS_FRAME: usize = 480;

/// El lazo de escucha sobre un [`Transcriptor`] cualquiera (mock, daemon, nube).
pub struct Lazo {
    vad: Vad<DetectorEnergia>,
    maquina: Maquina,
    stt: Arc<dyn Transcriptor>,
    frame_len: usize,
    pendiente: Vec<i16>,
}

impl Lazo {
    /// Lazo con los defaults de la suite (16 kHz, frame de 30 ms, energía).
    pub fn new(stt: Arc<dyn Transcriptor>) -> Self {
        Self::con_config(stt, HZ_OBJETIVO, MUESTRAS_FRAME, ConfigVad::default(), ConfigVoz::default())
    }

    /// Lazo afinable: tasa, largo de frame, y configs de VAD y de la máquina.
    pub fn con_config(
        stt: Arc<dyn Transcriptor>,
        hz: u32,
        frame_len: usize,
        cfg_vad: ConfigVad,
        cfg_voz: ConfigVoz,
    ) -> Self {
        Self {
            vad: Vad::new(DetectorEnergia::default(), cfg_vad, hz),
            maquina: Maquina::new(cfg_voz),
            stt,
            frame_len: frame_len.max(1),
            pendiente: Vec::new(),
        }
    }

    /// Tasa de muestreo que el lazo espera en [`Self::empujar`].
    pub fn hz(&self) -> u32 {
        HZ_OBJETIVO
    }

    /// Estado actual de la escucha.
    pub fn estado(&self) -> EstadoVoz {
        self.maquina.estado()
    }

    /// Empuja muestras `i16` mono. Acumula hasta completar frames, segmenta con
    /// el VAD, transcribe cada utterance cerrada y avanza la máquina. Devuelve
    /// los eventos producidos (puede ser ninguno).
    pub async fn empujar(&mut self, muestras: &[i16]) -> Vec<EventoEscucha> {
        self.pendiente.extend_from_slice(muestras);
        let mut eventos = Vec::new();
        while self.pendiente.len() >= self.frame_len {
            let frame: Vec<i16> = self.pendiente.drain(..self.frame_len).collect();
            match self.vad.empujar(&frame) {
                SalidaVad::Nada => {}
                SalidaVad::Empezo => {
                    self.maquina.avanzar(Evento::VozEmpieza);
                    eventos.push(EventoEscucha::Escuchando);
                }
                SalidaVad::Termino(audio) => {
                    // Sólo ahora corre el STT, sobre el fragmento que aisló el VAD.
                    if let Ok(t) = self.stt.transcribir(&audio).await {
                        let r = self.maquina.avanzar(Evento::Transcript(t.texto));
                        if let Some(ev) = traducir(r) {
                            eventos.push(ev);
                        }
                    }
                    // Un error de STT (red caída, daemon ausente) se traga: la
                    // escucha sigue viva para la próxima utterance.
                }
            }
        }
        eventos
    }

    /// Pulso de reloj para los timeouts de re-dormida. El driver lo llama en un
    /// timer; un test lo llama a mano. Devuelve el evento si la máquina reaccionó.
    pub fn tick(&mut self) -> Option<EventoEscucha> {
        traducir(self.maquina.avanzar(Evento::Tick))
    }
}

/// Mapea una [`Reaccion`] de la máquina a un [`EventoEscucha`] (o nada).
fn traducir(r: Reaccion) -> Option<EventoEscucha> {
    match r {
        Reaccion::Nada => None,
        Reaccion::Desperto => Some(EventoEscucha::Desperto),
        Reaccion::Dictar(t) => Some(EventoEscucha::Dictar(t)),
        Reaccion::SeDurmio => Some(EventoEscucha::SeDurmio),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rimay_voz::TranscriptorMock;

    /// Frames de voz (amplitud alta) y de silencio del largo de un frame.
    fn voz(n_frames: usize) -> Vec<i16> {
        vec![20_000; MUESTRAS_FRAME * n_frames]
    }
    fn silencio(n_frames: usize) -> Vec<i16> {
        vec![0; MUESTRAS_FRAME * n_frames]
    }

    fn lazo_con(texto: &str) -> Lazo {
        // Colgado corto para cerrar utterances rápido en el test.
        let cfg_vad = ConfigVad { umbral: 0.5, arranque: 2, colgado: 3 };
        Lazo::con_config(
            Arc::new(TranscriptorMock::con_texto(texto)),
            HZ_OBJETIVO,
            MUESTRAS_FRAME,
            cfg_vad,
            ConfigVoz::default(),
        )
    }

    #[tokio::test]
    async fn ruido_que_no_es_el_llamado_no_emite_dictado() {
        let mut l = lazo_con("cargo build release");
        let mut evs = l.empujar(&voz(5)).await;
        evs.extend(l.empujar(&silencio(3)).await);
        // Hubo «Escuchando» (arrancó voz) pero ningún Dictar/Desperto.
        assert!(evs.contains(&EventoEscucha::Escuchando));
        assert!(!evs.iter().any(|e| matches!(e, EventoEscucha::Dictar(_) | EventoEscucha::Desperto)));
        assert_eq!(l.estado(), EstadoVoz::Dormido);
    }

    #[tokio::test]
    async fn el_llamado_despierta() {
        let mut l = lazo_con("shuma");
        let mut evs = l.empujar(&voz(4)).await;
        evs.extend(l.empujar(&silencio(3)).await);
        assert!(evs.contains(&EventoEscucha::Desperto));
        assert_eq!(l.estado(), EstadoVoz::Despierto);
    }

    #[tokio::test]
    async fn llamado_con_cola_dicta() {
        let mut l = lazo_con("shuma abrí cosmos");
        let mut evs = l.empujar(&voz(6)).await;
        evs.extend(l.empujar(&silencio(3)).await);
        assert!(evs.contains(&EventoEscucha::Dictar("abrí cosmos".into())));
        assert_eq!(l.estado(), EstadoVoz::Dictando);
    }

    #[tokio::test]
    async fn tick_re_duerme_tras_silencio_prolongado() {
        let mut l = lazo_con("shuma");
        let mut evs = l.empujar(&voz(4)).await;
        evs.extend(l.empujar(&silencio(3)).await);
        assert_eq!(l.estado(), EstadoVoz::Despierto);
        // paciencia_despierto = 4 ticks.
        let mut durmio = false;
        for _ in 0..4 {
            if let Some(EventoEscucha::SeDurmio) = l.tick() {
                durmio = true;
            }
        }
        assert!(durmio);
        assert_eq!(l.estado(), EstadoVoz::Dormido);
    }

    #[tokio::test]
    async fn silencio_puro_no_emite_nada() {
        let mut l = lazo_con("shuma");
        let evs = l.empujar(&silencio(20)).await;
        assert!(evs.is_empty());
    }
}
