//! VAD y segmentación de utterances — la primera compuerta del pipeline.
//!
//! `VOZ.md` arranca el lazo manos-libres así:
//!
//! ```text
//!   cpal frames → VAD (local, det.) → [hay voz] → STT del fragmento → máquina
//! ```
//!
//! Este módulo es ese **VAD + segmentador**, en la misma doctrina que el resto
//! de [`crate`]: *determinista primero, modelo opcional después*. Tres piezas,
//! todas puras y sync (sin audio, sin red, sin reloj):
//!
//! - [`DetectorVoz`] — el contrato «¿este frame tiene voz?» → probabilidad. Lo
//!   cumple el detector de energía barato ([`DetectorEnergia`]) **y** un futuro
//!   `rimay-voz-silero` (Silero ONNX), igual que `Transcriptor` lo cumplen mock
//!   y whisper.
//! - [`Segmentador`] — máquina de estados que convierte un flujo de
//!   probabilidades por-frame en eventos de borde de utterance ([`PulsoVad`]),
//!   con *debounce* de arranque y *colgado* (hangover) de cierre. Es el corazón
//!   testeable, desacoplado de cómo se mide la voz.
//! - [`Vad`] — junta detector + segmentador + acumulación del audio: el host
//!   le empuja frames y recibe el fragmento completo ([`Audio`]) cuando la
//!   utterance cierra, listo para mandar al STT.
//!
//! El umbral del segmentador es sobre la **probabilidad**, así Silero (que da
//! prob por frame) entra sin tocar nada: sólo cambia el [`DetectorVoz`].

use crate::traits::Audio;

/// **Contrato VAD** — cuánta voz tiene un frame de audio, en `[0, 1]`.
///
/// Un frame es un bloque corto de PCM mono 16-bit (ej. 20–30 ms). Sync y barato
/// a propósito: corre por cada frame del micrófono. El detector de energía no
/// usa `hz`; un detector de modelo (Silero) asume una tasa fija — el host
/// resamplea antes. Por eso la tasa va aparte, no en el trait.
pub trait DetectorVoz {
    /// Probabilidad `[0,1]` de que `frame` contenga voz.
    fn probabilidad(&self, frame: &[i16]) -> f32;
}

/// Detector determinista por **energía RMS** — el default sin modelo.
///
/// Mapea el RMS del frame (normalizado a `[0,1]` sobre el rango de `i16`) a una
/// pseudo-probabilidad lineal entre `piso` y `techo`: bajo el piso → 0, sobre
/// el techo → 1. No distingue voz de otros sonidos fuertes; es la compuerta
/// barata de F0, a reemplazar por Silero cuando se quiera robustez.
#[derive(Debug, Clone, Copy)]
pub struct DetectorEnergia {
    /// RMS normalizado bajo el cual se considera silencio puro (prob 0).
    pub piso: f32,
    /// RMS normalizado sobre el cual se considera voz segura (prob 1).
    pub techo: f32,
}

impl Default for DetectorEnergia {
    fn default() -> Self {
        // Calibrado para micrófono de escritorio a ganancia típica: ~1% de
        // fondo de sala, ~8% habla normal. Ajustable por el host si hace falta.
        Self { piso: 0.01, techo: 0.08 }
    }
}

impl DetectorEnergia {
    /// RMS del frame, normalizado a `[0,1]` sobre el rango de `i16`.
    pub fn rms_normalizado(frame: &[i16]) -> f32 {
        if frame.is_empty() {
            return 0.0;
        }
        // Acumular en f64 evita perder precisión en frames largos.
        let suma: f64 = frame.iter().map(|&m| (m as f64).powi(2)).sum();
        let rms = (suma / frame.len() as f64).sqrt();
        (rms / i16::MAX as f64) as f32
    }
}

impl DetectorVoz for DetectorEnergia {
    fn probabilidad(&self, frame: &[i16]) -> f32 {
        let rms = Self::rms_normalizado(frame);
        if self.techo <= self.piso {
            return if rms >= self.techo { 1.0 } else { 0.0 };
        }
        ((rms - self.piso) / (self.techo - self.piso)).clamp(0.0, 1.0)
    }
}

/// Config del segmentador — todo en cuentas de frames (el host fija el ms/frame).
#[derive(Debug, Clone, Copy)]
pub struct ConfigVad {
    /// Probabilidad a partir de la cual un frame cuenta como voz.
    pub umbral: f32,
    /// Frames de voz consecutivos para declarar **inicio** (anti-ruido).
    pub arranque: u32,
    /// Frames de silencio consecutivos para declarar **fin** (hangover): evita
    /// cortar en las pausas naturales dentro de una frase.
    pub colgado: u32,
}

impl Default for ConfigVad {
    fn default() -> Self {
        // A ~30 ms/frame: ~60 ms para arrancar, ~300 ms de silencio para cerrar.
        Self { umbral: 0.5, arranque: 2, colgado: 10 }
    }
}

/// Borde de utterance que reporta el [`Segmentador`] por cada frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PulsoVad {
    /// Seguimos en silencio (fuera de una utterance).
    Silencio,
    /// Este frame abre una utterance (cruzó el debounce de arranque).
    Inicio,
    /// Seguimos dentro de la utterance (voz o silencio aún bajo el colgado).
    Sigue,
    /// Este frame cierra la utterance (se cumplió el colgado de silencio).
    Fin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Estado {
    Silencio,
    Voz,
}

/// Convierte un flujo de probabilidades por-frame en eventos de borde de
/// utterance. Puro y determinista: misma secuencia → mismos pulsos.
#[derive(Debug, Clone)]
pub struct Segmentador {
    cfg: ConfigVad,
    estado: Estado,
    voz_seguidos: u32,
    silencio_seguidos: u32,
}

impl Segmentador {
    pub fn new(cfg: ConfigVad) -> Self {
        Self {
            cfg,
            estado: Estado::Silencio,
            voz_seguidos: 0,
            silencio_seguidos: 0,
        }
    }

    /// ¿Estamos dentro de una utterance ahora mismo?
    pub fn en_voz(&self) -> bool {
        self.estado == Estado::Voz
    }

    /// Empuja la probabilidad de un frame y devuelve el borde que produce.
    pub fn empujar(&mut self, prob: f32) -> PulsoVad {
        let es_voz = prob >= self.cfg.umbral;
        match self.estado {
            Estado::Silencio => {
                if es_voz {
                    self.voz_seguidos = self.voz_seguidos.saturating_add(1);
                    if self.voz_seguidos >= self.cfg.arranque {
                        self.estado = Estado::Voz;
                        self.silencio_seguidos = 0;
                        PulsoVad::Inicio
                    } else {
                        PulsoVad::Silencio
                    }
                } else {
                    self.voz_seguidos = 0;
                    PulsoVad::Silencio
                }
            }
            Estado::Voz => {
                if es_voz {
                    self.silencio_seguidos = 0;
                    PulsoVad::Sigue
                } else {
                    self.silencio_seguidos = self.silencio_seguidos.saturating_add(1);
                    if self.silencio_seguidos >= self.cfg.colgado {
                        self.estado = Estado::Silencio;
                        self.voz_seguidos = 0;
                        PulsoVad::Fin
                    } else {
                        PulsoVad::Sigue
                    }
                }
            }
        }
    }
}

/// Lo que devuelve [`Vad::empujar`] tras consumir un frame de audio.
#[derive(Debug, Clone, PartialEq)]
pub enum SalidaVad {
    /// Nada que hacer (silencio, o utterance aún en curso).
    Nada,
    /// Arrancó una utterance — el host puede emitir `Evento::VozEmpieza` a la
    /// [`Maquina`](crate::maquina).
    Empezo,
    /// Cerró la utterance: acá va su audio completo, listo para el STT.
    Termino(Audio),
}

/// VAD completo: detector + segmentador + acumulación del audio.
///
/// El host le empuja frames del micrófono y recibe el fragmento entero cuando
/// la utterance cierra — sin manejar buffers ni umbrales. Genérico sobre el
/// [`DetectorVoz`], así pasar de energía a Silero es cambiar un tipo.
#[derive(Debug, Clone)]
pub struct Vad<D: DetectorVoz> {
    detector: D,
    seg: Segmentador,
    hz: u32,
    umbral: f32,
    buffer: Vec<i16>,
    /// Largo del buffer justo tras el último frame *con voz*. Al cerrar se
    /// recorta acá, así el [`Audio`] emitido no arrastra el silencio del
    /// colgado (mejor para el STT y para el match del wake-word).
    fin_voz: usize,
}

impl<D: DetectorVoz> Vad<D> {
    /// Crea el VAD con un detector, su config y la tasa de muestreo de los
    /// frames (la que tendrá el [`Audio`] emitido).
    pub fn new(detector: D, cfg: ConfigVad, hz: u32) -> Self {
        let umbral = cfg.umbral;
        Self {
            detector,
            seg: Segmentador::new(cfg),
            hz,
            umbral,
            buffer: Vec::new(),
            fin_voz: 0,
        }
    }

    /// ¿Hay una utterance en curso?
    pub fn en_voz(&self) -> bool {
        self.seg.en_voz()
    }

    /// Consume un frame de audio. Acumula mientras dura la utterance y entrega
    /// el [`Audio`] completo al cerrar.
    ///
    /// Nota: el frame de arranque queda incluido; los `arranque - 1` frames de
    /// debounce previos no (clipping de ~un frame al inicio). Si hiciera falta
    /// preservar el ataque exacto, el host puede mantener un pre-roll.
    pub fn empujar(&mut self, frame: &[i16]) -> SalidaVad {
        let prob = self.detector.probabilidad(frame);
        let es_voz = prob >= self.umbral;
        match self.seg.empujar(prob) {
            PulsoVad::Silencio => SalidaVad::Nada,
            PulsoVad::Inicio => {
                self.buffer.clear();
                self.buffer.extend_from_slice(frame);
                self.fin_voz = self.buffer.len(); // el frame de inicio es voz
                SalidaVad::Empezo
            }
            PulsoVad::Sigue => {
                self.buffer.extend_from_slice(frame);
                if es_voz {
                    self.fin_voz = self.buffer.len();
                }
                SalidaVad::Nada
            }
            PulsoVad::Fin => {
                self.buffer.extend_from_slice(frame);
                if es_voz {
                    self.fin_voz = self.buffer.len();
                }
                // Recortar el colgado de silencio: la utterance termina en el
                // último frame con voz, no en el silencio que la cerró.
                self.buffer.truncate(self.fin_voz);
                let muestras = std::mem::take(&mut self.buffer);
                self.fin_voz = 0;
                SalidaVad::Termino(Audio::new(muestras, self.hz))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frame de `n` muestras a una amplitud constante `amp`.
    fn frame(amp: i16, n: usize) -> Vec<i16> {
        vec![amp; n]
    }

    #[test]
    fn energia_silencio_es_cero_voz_es_uno() {
        let d = DetectorEnergia::default();
        assert_eq!(d.probabilidad(&frame(0, 480)), 0.0);
        // amplitud bien sobre el techo (8% de i16::MAX ≈ 2621) → prob 1.
        assert_eq!(d.probabilidad(&frame(20_000, 480)), 1.0);
    }

    #[test]
    fn energia_frame_vacio_no_panica() {
        assert_eq!(DetectorEnergia::default().probabilidad(&[]), 0.0);
    }

    #[test]
    fn segmentador_arranque_necesita_debounce() {
        let mut s = Segmentador::new(ConfigVad { umbral: 0.5, arranque: 2, colgado: 3 });
        // primer frame de voz: aún no arranca (debounce 2).
        assert_eq!(s.empujar(1.0), PulsoVad::Silencio);
        // segundo: arranca.
        assert_eq!(s.empujar(1.0), PulsoVad::Inicio);
        assert!(s.en_voz());
    }

    #[test]
    fn segmentador_ruido_aislado_no_dispara() {
        let mut s = Segmentador::new(ConfigVad { umbral: 0.5, arranque: 3, colgado: 3 });
        // un pico aislado de voz seguido de silencio: nunca llega al debounce.
        assert_eq!(s.empujar(1.0), PulsoVad::Silencio);
        assert_eq!(s.empujar(0.0), PulsoVad::Silencio);
        assert_eq!(s.empujar(1.0), PulsoVad::Silencio);
        assert_eq!(s.empujar(0.0), PulsoVad::Silencio);
        assert!(!s.en_voz());
    }

    #[test]
    fn segmentador_colgado_aguanta_pausas_internas() {
        let mut s = Segmentador::new(ConfigVad { umbral: 0.5, arranque: 1, colgado: 3 });
        assert_eq!(s.empujar(1.0), PulsoVad::Inicio);
        // pausa de 2 < colgado(3): sigue dentro de la utterance.
        assert_eq!(s.empujar(0.0), PulsoVad::Sigue);
        assert_eq!(s.empujar(0.0), PulsoVad::Sigue);
        // vuelve la voz: el contador de silencio se resetea.
        assert_eq!(s.empujar(1.0), PulsoVad::Sigue);
        // ahora 3 silencios seguidos cierran.
        assert_eq!(s.empujar(0.0), PulsoVad::Sigue);
        assert_eq!(s.empujar(0.0), PulsoVad::Sigue);
        assert_eq!(s.empujar(0.0), PulsoVad::Fin);
        assert!(!s.en_voz());
    }

    #[test]
    fn vad_acumula_y_entrega_la_utterance() {
        let cfg = ConfigVad { umbral: 0.5, arranque: 1, colgado: 2 };
        let mut v = Vad::new(DetectorEnergia::default(), cfg, 16_000);
        let voz = frame(20_000, 100); // sobre el techo
        let sil = frame(0, 100);

        assert_eq!(v.empujar(&voz), SalidaVad::Empezo);
        assert_eq!(v.empujar(&voz), SalidaVad::Nada); // Sigue, acumula
        assert_eq!(v.empujar(&sil), SalidaVad::Nada); // 1er silencio < colgado
        match v.empujar(&sil) {
            SalidaVad::Termino(audio) => {
                // 2 frames de voz acumulados; el silencio del colgado (2 frames)
                // se recorta — la utterance termina en el último frame con voz.
                assert_eq!(audio.muestras.len(), 200);
                assert_eq!(audio.hz, 16_000);
            }
            otro => panic!("esperaba Termino, vino {otro:?}"),
        }
        assert!(!v.en_voz());
    }

    #[test]
    fn vad_silencio_puro_no_emite_nada() {
        let mut v = Vad::new(DetectorEnergia::default(), ConfigVad::default(), 16_000);
        for _ in 0..20 {
            assert_eq!(v.empujar(&frame(0, 480)), SalidaVad::Nada);
        }
    }
}
