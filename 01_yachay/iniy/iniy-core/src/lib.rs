//! iniy-core — tipos núcleo del laboratorio semántico de creencias.
//!
//! Modela el espacio donde viven aserciones, su confianza subjetiva, y las
//! implicaciones que las conectan. No habla de "verdad" — habla de grados
//! de creencia y dirección de la subjetividad. Inspirado en Subjective Logic
//! (Jøsang) y en la idea de un espacio vectorial de posturas.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ulid::Ulid;

/// Error compartido por el dominio.
#[derive(Debug, Error)]
pub enum Error {
    #[error("confianza fuera de rango [0,1]: {0}")]
    ConfianzaFueraDeRango(f32),
    #[error("opinion no normalizada: b+d+u = {0} (esperado 1.0 ± ε)")]
    OpinionNoNormalizada(f32),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Identificador de un documento ingerido (libro, ensayo, transcripción).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocId(pub Ulid);

impl DocId {
    pub fn nuevo() -> Self {
        Self(Ulid::new())
    }
}

/// Identificador de un chunk (pasaje textual contiguo extraído del documento).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkId(pub Ulid);

impl ChunkId {
    pub fn nuevo() -> Self {
        Self(Ulid::new())
    }
}

/// Identificador de una aserción atómica.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AsercionId(pub Ulid);

impl AsercionId {
    pub fn nuevo() -> Self {
        Self(Ulid::new())
    }
}

/// Opinión subjetiva al estilo Jøsang: (creencia, descreencia, incertidumbre, base_rate).
///
/// Invariantes: b + d + u == 1.0 (con tolerancia ε), todos en [0,1].
/// `base_rate` es el prior — qué tan creíble sería esta aserción sin evidencia adicional.
///
/// La gran diferencia respecto a una probabilidad escalar: `u` (incertidumbre)
/// modela explícitamente la ignorancia. Una creencia de 0.5 con u=0 ("estoy 50/50")
/// es muy distinta de una creencia con u=1 ("no tengo idea"), aunque ambas proyecten
/// igual probabilidad esperada.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Opinion {
    pub creencia: f32,
    pub descreencia: f32,
    pub incertidumbre: f32,
    pub base_rate: f32,
}

const EPS: f32 = 1e-4;

impl Opinion {
    pub fn nueva(creencia: f32, descreencia: f32, incertidumbre: f32, base_rate: f32) -> Result<Self> {
        for v in [creencia, descreencia, incertidumbre, base_rate] {
            if !(0.0..=1.0).contains(&v) {
                return Err(Error::ConfianzaFueraDeRango(v));
            }
        }
        let suma = creencia + descreencia + incertidumbre;
        if (suma - 1.0).abs() > EPS {
            return Err(Error::OpinionNoNormalizada(suma));
        }
        Ok(Self { creencia, descreencia, incertidumbre, base_rate })
    }

    /// Probabilidad esperada proyectada: P = b + u·a.
    pub fn probabilidad_esperada(&self) -> f32 {
        self.creencia + self.incertidumbre * self.base_rate
    }

    /// Total ignorancia (vacuous opinion): no se sabe nada, el prior decide.
    pub fn vacua(base_rate: f32) -> Result<Self> {
        Self::nueva(0.0, 0.0, 1.0, base_rate)
    }

    /// Certeza dogmática a favor.
    pub fn dogmatica_si() -> Self {
        Self { creencia: 1.0, descreencia: 0.0, incertidumbre: 0.0, base_rate: 0.5 }
    }

    /// Certeza dogmática en contra.
    pub fn dogmatica_no() -> Self {
        Self { creencia: 0.0, descreencia: 1.0, incertidumbre: 0.0, base_rate: 0.5 }
    }
}

/// Aserción atómica extraída del texto.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asercion {
    pub id: AsercionId,
    pub doc_id: DocId,
    pub chunk_id: ChunkId,
    pub texto: String,
    /// Opinión inferida desde el propio texto (hedging, modalidad, marcadores epistémicos).
    pub opinion_autoral: Opinion,
}

/// Resultado de evaluar la relación entre dos aserciones via NLI.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RelacionNli {
    pub entailment: f32,
    pub contradiction: f32,
    pub neutral: f32,
}

impl RelacionNli {
    pub fn dominante(&self) -> ClaseNli {
        let (mut max, mut clase) = (self.entailment, ClaseNli::Entailment);
        if self.contradiction > max { max = self.contradiction; clase = ClaseNli::Contradiction; }
        if self.neutral > max { clase = ClaseNli::Neutral; }
        clase
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaseNli {
    Entailment,
    Contradiction,
    Neutral,
}

/// Arista del grafo: una aserción premisa implica/contradice a una hipótesis,
/// con qué peso, y cuánto de "ese peso" es subjetivo del autor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Implicacion {
    pub premisa: AsercionId,
    pub hipotesis: AsercionId,
    pub relacion: RelacionNli,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opinion_valida_se_construye() {
        let op = Opinion::nueva(0.7, 0.1, 0.2, 0.5).unwrap();
        assert!((op.probabilidad_esperada() - 0.8).abs() < 1e-5);
    }

    #[test]
    fn opinion_no_normalizada_falla() {
        assert!(Opinion::nueva(0.7, 0.7, 0.7, 0.5).is_err());
    }

    #[test]
    fn opinion_vacua_proyecta_base_rate() {
        let op = Opinion::vacua(0.3).unwrap();
        assert!((op.probabilidad_esperada() - 0.3).abs() < 1e-5);
    }
}
