//! Política de lectura discriminada (TTS).
//!
//! Doctrina general de voz: **el TTS no lee todo.** Se vocaliza la prosa de una
//! IA conversacional; nunca código ni acciones (un bloque de código leído en
//! voz alta es ruido, y una acción de control no se «narra», se aprueba).
//! Cualquier consumidor mapea su tipo de bloque de salida → [`TipoBloque`] y
//! consulta acá (ej. shuma mapea `shuma_agente::BloqueSalida`).

/// Tipo de bloque de salida de una IA, a efectos de TTS. Taxonomía mínima y
/// general: el consumidor mapea sus propios bloques a estas tres clases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoBloque {
    /// Prosa (lenguaje natural del asistente).
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

/// Política de lectura del consumidor: la voz es opt-in.
#[derive(Debug, Clone, Copy, Default)]
pub struct Politica {
    /// Si la lectura en voz está activada. Default `false` (opt-in).
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
