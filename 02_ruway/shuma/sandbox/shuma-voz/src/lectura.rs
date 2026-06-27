//! Política de lectura discriminada (TTS).
//!
//! Doctrina de `VOZ.md`: **la voz no lee todo.** Se vocaliza sólo la prosa del
//! agente; nunca código ni acciones (un bloque de código leído en voz alta es
//! ruido, y una acción de control no se «narra», se aprueba). El host mapea
//! `shuma_agente::BloqueSalida` → [`TipoBloque`] y consulta acá.

/// Tipo de bloque de salida del agente, a efectos de TTS. Espejo mínimo de
/// `shuma_agente::BloqueSalida` para no acoplar este núcleo al de conversación.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoBloque {
    /// Prosa del asistente.
    Texto,
    /// Bloque de código.
    Codigo,
    /// Acción de control propuesta (se aprueba, no se narra).
    Accion,
}

/// ¿Se vocaliza este tipo de bloque? Sólo la prosa.
pub fn debe_leer(tipo: TipoBloque) -> bool {
    matches!(tipo, TipoBloque::Texto)
}

/// Política de lectura por agente: la voz es opt-in.
#[derive(Debug, Clone, Copy, Default)]
pub struct Politica {
    /// Si el agente tiene la voz activada. Default `false` (opt-in).
    pub voz_activa: bool,
}

impl Politica {
    /// Decide si un bloque concreto se lee: requiere voz activa **y** que el
    /// tipo sea vocalizable.
    pub fn lee(&self, tipo: TipoBloque) -> bool {
        self.voz_activa && debe_leer(tipo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_la_prosa_es_vocalizable() {
        assert!(debe_leer(TipoBloque::Texto));
        assert!(!debe_leer(TipoBloque::Codigo));
        assert!(!debe_leer(TipoBloque::Accion));
    }

    #[test]
    fn voz_apagada_no_lee_nada() {
        let p = Politica::default();
        assert!(!p.lee(TipoBloque::Texto));
    }

    #[test]
    fn voz_activa_solo_lee_prosa() {
        let p = Politica { voz_activa: true };
        assert!(p.lee(TipoBloque::Texto));
        assert!(!p.lee(TipoBloque::Codigo));
        assert!(!p.lee(TipoBloque::Accion));
    }
}
