//! Máquina de estados de la escucha manos-libres.
//!
//! El host corre un VAD local siempre-encendido y, cuando hay voz, transcribe
//! el fragmento (STT local o nube). Cada fragmento entra como un [`Evento`];
//! la máquina decide si fue el llamado, si toca dictar, o si hay que volver a
//! dormir por silencio. Es determinista y no asume reloj: el host marca el
//! tiempo con [`Evento::Tick`].

use serde::{Deserialize, Serialize};

/// Estado de la escucha de voz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EstadoVoz {
    /// VAD activo, esperando el llamado. **Nada se transcribe hacia el input.**
    Dormido,
    /// Llamado reconocido; ventana corta esperando que el usuario hable.
    Despierto,
    /// Dictando: el transcript fluye al input.
    Dictando,
}

/// Configuración de la escucha — toda determinista, sin red.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigVoz {
    /// Palabra de activación. Default `"shuma"` (Regla 6: no «Alexa»).
    pub llamado: String,
    /// Ticks de silencio en [`EstadoVoz::Despierto`] antes de re-dormir.
    pub paciencia_despierto: u32,
    /// Ticks de silencio en [`EstadoVoz::Dictando`] antes de cerrar el dictado.
    pub paciencia_dictado: u32,
}

impl Default for ConfigVoz {
    fn default() -> Self {
        Self {
            llamado: "shuma".to_string(),
            paciencia_despierto: 4,
            paciencia_dictado: 8,
        }
    }
}

/// Lo que el host le cuenta a la máquina.
#[derive(Debug, Clone)]
pub enum Evento {
    /// El VAD reporta inicio de voz (resetea el contador de silencio).
    VozEmpieza,
    /// El STT entregó el texto de un fragmento con voz.
    Transcript(String),
    /// Pulso de reloj del host (para los timeouts de re-dormida).
    Tick,
}

/// Lo que la máquina le pide al host tras un [`Evento`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reaccion {
    /// Nada que hacer.
    Nada,
    /// Se reconoció el llamado; abrió la escucha (sin texto aún).
    Desperto,
    /// Poné este texto en el input (dictado).
    Dictar(String),
    /// Volvió a [`EstadoVoz::Dormido`] (timeout de silencio).
    SeDurmio,
}

/// La escucha manos-libres como autómata.
#[derive(Debug, Clone)]
pub struct Maquina {
    estado: EstadoVoz,
    cfg: ConfigVoz,
    silencio: u32,
}

impl Maquina {
    /// Arranca dormida con la config dada.
    pub fn new(cfg: ConfigVoz) -> Self {
        Self {
            estado: EstadoVoz::Dormido,
            cfg,
            silencio: 0,
        }
    }

    /// Estado actual.
    pub fn estado(&self) -> EstadoVoz {
        self.estado
    }

    /// Procesa un evento del host y devuelve qué hacer.
    pub fn avanzar(&mut self, ev: Evento) -> Reaccion {
        match ev {
            Evento::VozEmpieza => {
                self.silencio = 0;
                Reaccion::Nada
            }
            Evento::Tick => self.tick(),
            Evento::Transcript(t) => self.transcript(&t),
        }
    }

    fn tick(&mut self) -> Reaccion {
        self.silencio = self.silencio.saturating_add(1);
        match self.estado {
            EstadoVoz::Dormido => Reaccion::Nada,
            EstadoVoz::Despierto if self.silencio >= self.cfg.paciencia_despierto => {
                self.dormir();
                Reaccion::SeDurmio
            }
            EstadoVoz::Dictando if self.silencio >= self.cfg.paciencia_dictado => {
                self.dormir();
                Reaccion::SeDurmio
            }
            _ => Reaccion::Nada,
        }
    }

    fn transcript(&mut self, t: &str) -> Reaccion {
        self.silencio = 0;
        match self.estado {
            EstadoVoz::Dormido => match detectar_llamado(t, &self.cfg.llamado) {
                // El llamado vino solo → abrimos y esperamos.
                Some(resto) if resto.is_empty() => {
                    self.estado = EstadoVoz::Despierto;
                    Reaccion::Desperto
                }
                // El llamado vino con cola («shuma, abrí cosmos») → dictamos ya.
                Some(resto) => {
                    self.estado = EstadoVoz::Dictando;
                    Reaccion::Dictar(resto)
                }
                // No es el llamado → nada sale de la máquina.
                None => Reaccion::Nada,
            },
            EstadoVoz::Despierto | EstadoVoz::Dictando => {
                self.estado = EstadoVoz::Dictando;
                Reaccion::Dictar(t.trim().to_string())
            }
        }
    }

    fn dormir(&mut self) {
        self.estado = EstadoVoz::Dormido;
        self.silencio = 0;
    }
}

/// Si `transcript` arranca con el `llamado`, devuelve la cola (lo que sigue),
/// ya trimeada. Compara **sólo la primera palabra**, ignorando puntuación de
/// borde y mayúsculas — robusto para *«Shuma, abrí…»* o *«shuma.»*.
///
/// Devuelve `None` si la primera palabra no es el llamado: así, en
/// [`EstadoVoz::Dormido`], el ruido ambiente nunca cruza al input.
pub fn detectar_llamado(transcript: &str, llamado: &str) -> Option<String> {
    let ll = llamado.trim().to_lowercase();
    if ll.is_empty() {
        return None;
    }
    let t = transcript.trim_start();
    let (primera, resto) = match t.split_once(char::is_whitespace) {
        Some((p, r)) => (p, r),
        None => (t, ""),
    };
    let limpia: String = primera
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    if limpia == ll {
        Some(resto.trim().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maq() -> Maquina {
        Maquina::new(ConfigVoz::default())
    }

    #[test]
    fn llamado_solo_despierta_sin_dictar() {
        let mut m = maq();
        assert_eq!(m.avanzar(Evento::Transcript("shuma".into())), Reaccion::Desperto);
        assert_eq!(m.estado(), EstadoVoz::Despierto);
    }

    #[test]
    fn llamado_con_cola_dicta_ya() {
        let mut m = maq();
        let r = m.avanzar(Evento::Transcript("Shuma, abrí cosmos".into()));
        assert_eq!(r, Reaccion::Dictar("abrí cosmos".into()));
        assert_eq!(m.estado(), EstadoVoz::Dictando);
    }

    #[test]
    fn dormido_ignora_lo_que_no_es_llamado() {
        let mut m = maq();
        assert_eq!(
            m.avanzar(Evento::Transcript("cargo build release".into())),
            Reaccion::Nada
        );
        assert_eq!(m.estado(), EstadoVoz::Dormido);
    }

    #[test]
    fn llamado_no_matchea_prefijo_de_palabra() {
        // "shumaqueta" NO debe disparar "shuma".
        let mut m = maq();
        assert_eq!(
            m.avanzar(Evento::Transcript("shumaqueta lo que sea".into())),
            Reaccion::Nada
        );
    }

    #[test]
    fn despierto_dicta_el_siguiente_transcript() {
        let mut m = maq();
        m.avanzar(Evento::Transcript("shuma".into()));
        assert_eq!(
            m.avanzar(Evento::Transcript("listá los archivos".into())),
            Reaccion::Dictar("listá los archivos".into())
        );
        assert_eq!(m.estado(), EstadoVoz::Dictando);
    }

    #[test]
    fn silencio_re_duerme_desde_despierto() {
        let mut m = maq();
        m.avanzar(Evento::Transcript("shuma".into()));
        // paciencia_despierto = 4
        for _ in 0..3 {
            assert_eq!(m.avanzar(Evento::Tick), Reaccion::Nada);
        }
        assert_eq!(m.avanzar(Evento::Tick), Reaccion::SeDurmio);
        assert_eq!(m.estado(), EstadoVoz::Dormido);
    }

    #[test]
    fn voz_resetea_el_silencio() {
        let mut m = maq();
        m.avanzar(Evento::Transcript("shuma".into()));
        m.avanzar(Evento::Tick);
        m.avanzar(Evento::Tick);
        m.avanzar(Evento::VozEmpieza); // resetea
        for _ in 0..3 {
            assert_eq!(m.avanzar(Evento::Tick), Reaccion::Nada);
        }
        // recién al 4° desde el reset se duerme
        assert_eq!(m.avanzar(Evento::Tick), Reaccion::SeDurmio);
    }

    #[test]
    fn detectar_llamado_limpia_puntuacion() {
        assert_eq!(detectar_llamado("shuma.", "shuma"), Some(String::new()));
        assert_eq!(
            detectar_llamado("  Shuma:  hacé algo", "shuma"),
            Some("hacé algo".to_string())
        );
        assert_eq!(detectar_llamado("nada", "shuma"), None);
    }
}
