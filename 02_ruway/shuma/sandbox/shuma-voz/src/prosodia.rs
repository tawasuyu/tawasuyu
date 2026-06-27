//! Clasificador prosódico determinista (capa barata de entonación).
//!
//! `VOZ.md` parte la entonación en dos capas. Ésta es la **(a)**: de los rasgos
//! de f0 que el host extrae del fragmento, saca una *pista* de intención sin
//! red ni modelo. Es una pista para desambiguar (¿pregunta o orden?), **no** un
//! veredicto emocional — eso es la capa (b), opt-in, vía modelo.

/// Rasgos prosódicos crudos de un fragmento con voz. Los extrae el host del
/// audio (pitch tracking + RMS); este crate sólo los interpreta.
#[derive(Debug, Clone, Copy)]
pub struct Rasgos {
    /// f0 medio del fragmento (Hz).
    pub f0_media: f32,
    /// f0 del tramo final (Hz).
    pub f0_final: f32,
    /// Energía RMS normalizada al rango `[0, 1]`.
    pub energia: f32,
}

/// Pista de intención derivada de la entonación. Determinista.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intencion {
    /// Subida final de f0 → probablemente una pregunta.
    Pregunta,
    /// Caída final de f0 → probablemente una orden/afirmación.
    Orden,
    /// Energía alta → probablemente urgencia (gana sobre el contorno).
    Urgencia,
    /// Sin señal clara.
    Neutral,
}

/// Umbral relativo de variación de f0 (15 %) para llamar subida/caída.
const UMBRAL_F0: f32 = 0.15;
/// Energía por encima de la cual prima la urgencia.
const UMBRAL_ENERGIA: f32 = 0.8;

/// Clasifica la entonación de un fragmento. La urgencia (energía) tiene
/// prioridad sobre el contorno; luego subida → pregunta, caída → orden.
pub fn clasificar(r: Rasgos) -> Intencion {
    if r.energia >= UMBRAL_ENERGIA {
        return Intencion::Urgencia;
    }
    if r.f0_media <= 0.0 {
        return Intencion::Neutral;
    }
    let delta = (r.f0_final - r.f0_media) / r.f0_media;
    if delta > UMBRAL_F0 {
        Intencion::Pregunta
    } else if delta < -UMBRAL_F0 {
        Intencion::Orden
    } else {
        Intencion::Neutral
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subida_final_es_pregunta() {
        let r = Rasgos { f0_media: 120.0, f0_final: 160.0, energia: 0.4 };
        assert_eq!(clasificar(r), Intencion::Pregunta);
    }

    #[test]
    fn caida_final_es_orden() {
        let r = Rasgos { f0_media: 120.0, f0_final: 90.0, energia: 0.4 };
        assert_eq!(clasificar(r), Intencion::Orden);
    }

    #[test]
    fn energia_alta_gana_como_urgencia() {
        // contorno de pregunta pero energía alta → urgencia
        let r = Rasgos { f0_media: 120.0, f0_final: 170.0, energia: 0.95 };
        assert_eq!(clasificar(r), Intencion::Urgencia);
    }

    #[test]
    fn contorno_plano_es_neutral() {
        let r = Rasgos { f0_media: 120.0, f0_final: 122.0, energia: 0.3 };
        assert_eq!(clasificar(r), Intencion::Neutral);
    }

    #[test]
    fn f0_invalido_es_neutral() {
        let r = Rasgos { f0_media: 0.0, f0_final: 0.0, energia: 0.2 };
        assert_eq!(clasificar(r), Intencion::Neutral);
    }
}
