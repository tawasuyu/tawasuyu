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

/// Identificador de una fuente (autor, escuela, tradición, observación).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FuenteId(pub Ulid);

impl FuenteId {
    pub fn nuevo() -> Self {
        Self(Ulid::new())
    }
}

/// Una fuente atribuye autoría/proveniencia a un documento. No es solo
/// "quién escribió" — es la *tradición* o *postura* desde la cual se afirma
/// algo. Dos documentos del mismo Aristóteles cuentan como una fuente; dos
/// documentos de tradiciones distintas que repiten el mismo texto, no.
///
/// `kind` es libre: "autor", "escuela", "tradición", "observación", "wiki",
/// "consenso científico", etc. Para MVP no se valida — se respeta lo que
/// el ingestor decida.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fuente {
    pub id: FuenteId,
    pub nombre: String,
    pub kind: Option<String>,
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

    /// Invierte la polaridad: lo que era creencia pasa a descreencia y
    /// viceversa. Útil para propagar opiniones a través de aristas de
    /// contradicción (si creo A y A contradice B, mi opinión sobre B es
    /// la opuesta de mi opinión sobre A).
    pub fn invertir(&self) -> Self {
        Self {
            creencia: self.descreencia,
            descreencia: self.creencia,
            incertidumbre: self.incertidumbre,
            base_rate: 1.0 - self.base_rate,
        }
    }

    /// Trust discounting de Jøsang: degrada b y d hacia u proporcional a
    /// (1 - peso). peso=1.0 → opinión intacta; peso=0.0 → opinión vacua.
    /// Preserva b+d+u=1 exactamente. Usar para propagar opinión a través
    /// de una arista NLI cuyo score (entailment o contradiction) < 1.
    pub fn descontar(&self, peso: f32) -> Self {
        let p = peso.clamp(0.0, 1.0);
        Self {
            creencia: self.creencia * p,
            descreencia: self.descreencia * p,
            incertidumbre: self.incertidumbre * p + (1.0 - p),
            base_rate: self.base_rate,
        }
    }

    /// Cumulative belief fusion de Jøsang: combina dos opiniones sobre la
    /// misma proposición provenientes de fuentes distintas. La opinión más
    /// cierta (u más baja) pesa más. Si ambas son dogmáticas (u=0), cae a
    /// promedio simple.
    pub fn fusionar(a: &Self, b: &Self) -> Self {
        let denom = a.incertidumbre + b.incertidumbre - a.incertidumbre * b.incertidumbre;
        let base_rate = (a.base_rate + b.base_rate) * 0.5;
        if denom < 1e-6 {
            // Ambas dogmáticas: u_a = u_b = 0. Promedio sin perder normalización.
            let creencia = (a.creencia + b.creencia) * 0.5;
            let descreencia = (a.descreencia + b.descreencia) * 0.5;
            let s = creencia + descreencia;
            if s < 1e-6 {
                return Self::vacua(base_rate).expect("base_rate ∈ [0,1]");
            }
            return Self {
                creencia: creencia / s,
                descreencia: descreencia / s,
                incertidumbre: 0.0,
                base_rate,
            };
        }
        let creencia = (a.creencia * b.incertidumbre + b.creencia * a.incertidumbre) / denom;
        let descreencia = (a.descreencia * b.incertidumbre + b.descreencia * a.incertidumbre) / denom;
        let incertidumbre = (a.incertidumbre * b.incertidumbre) / denom;
        let s = creencia + descreencia + incertidumbre;
        // Reescalar si la aritmética flotante saca de norma.
        Self {
            creencia: creencia / s,
            descreencia: descreencia / s,
            incertidumbre: incertidumbre / s,
            base_rate,
        }
    }

    /// Fusiona N opiniones. Si la lista está vacía, devuelve la vacua con
    /// base_rate=0.5. Asociativa y conmutativa por construcción de `fusionar`.
    pub fn fusionar_muchas(ops: &[Self]) -> Self {
        let mut iter = ops.iter();
        let Some(first) = iter.next() else {
            return Self::vacua(0.5).expect("0.5 ∈ [0,1]");
        };
        let mut acc = *first;
        for o in iter {
            acc = Self::fusionar(&acc, o);
        }
        acc
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

    #[test]
    fn invertir_intercambia_creencia_y_descreencia() {
        let a = Opinion::nueva(0.7, 0.1, 0.2, 0.5).unwrap();
        let b = a.invertir();
        assert!((b.creencia - 0.1).abs() < 1e-5);
        assert!((b.descreencia - 0.7).abs() < 1e-5);
        assert!((b.incertidumbre - 0.2).abs() < 1e-5);
        assert!((b.base_rate - 0.5).abs() < 1e-5);
    }

    #[test]
    fn descontar_con_peso_uno_es_identidad() {
        let a = Opinion::nueva(0.7, 0.1, 0.2, 0.5).unwrap();
        let b = a.descontar(1.0);
        assert!((a.creencia - b.creencia).abs() < 1e-5);
        assert!((a.descreencia - b.descreencia).abs() < 1e-5);
        assert!((a.incertidumbre - b.incertidumbre).abs() < 1e-5);
    }

    #[test]
    fn descontar_con_peso_cero_es_vacua() {
        let a = Opinion::nueva(0.7, 0.1, 0.2, 0.5).unwrap();
        let b = a.descontar(0.0);
        assert!((b.incertidumbre - 1.0).abs() < 1e-5);
        assert!((b.creencia).abs() < 1e-5);
        assert!((b.descreencia).abs() < 1e-5);
    }

    #[test]
    fn descontar_preserva_norma() {
        let a = Opinion::nueva(0.6, 0.3, 0.1, 0.5).unwrap();
        let b = a.descontar(0.5);
        let s = b.creencia + b.descreencia + b.incertidumbre;
        assert!((s - 1.0).abs() < 1e-5);
    }

    #[test]
    fn fusionar_dos_opiniones_iguales_es_idempotente_en_p_esperada() {
        let a = Opinion::nueva(0.6, 0.1, 0.3, 0.5).unwrap();
        let f = Opinion::fusionar(&a, &a);
        assert!((f.probabilidad_esperada() - a.probabilidad_esperada()).abs() < 0.05);
    }

    #[test]
    fn fusionar_baja_incertidumbre() {
        let a = Opinion::nueva(0.5, 0.0, 0.5, 0.5).unwrap();
        let b = Opinion::nueva(0.5, 0.0, 0.5, 0.5).unwrap();
        let f = Opinion::fusionar(&a, &b);
        // Dos opiniones independientes confluyentes deben bajar la u.
        assert!(f.incertidumbre < a.incertidumbre - 0.05);
    }

    #[test]
    fn fusionar_dogmaticas_opuestas_se_balancea() {
        let si = Opinion::dogmatica_si();
        let no = Opinion::dogmatica_no();
        let f = Opinion::fusionar(&si, &no);
        assert!((f.creencia - 0.5).abs() < 0.01);
        assert!((f.descreencia - 0.5).abs() < 0.01);
        assert_eq!(f.incertidumbre, 0.0);
    }

    #[test]
    fn fusionar_muchas_lista_vacia_es_vacua() {
        let f = Opinion::fusionar_muchas(&[]);
        assert_eq!(f.incertidumbre, 1.0);
    }

    #[test]
    fn fusionar_muchas_es_consistente_con_fusionar_dos() {
        let a = Opinion::nueva(0.7, 0.1, 0.2, 0.5).unwrap();
        let b = Opinion::nueva(0.6, 0.1, 0.3, 0.5).unwrap();
        let c = Opinion::nueva(0.5, 0.2, 0.3, 0.5).unwrap();
        let chain = Opinion::fusionar(&Opinion::fusionar(&a, &b), &c);
        let muchas = Opinion::fusionar_muchas(&[a, b, c]);
        assert!((chain.creencia - muchas.creencia).abs() < 1e-5);
        assert!((chain.incertidumbre - muchas.incertidumbre).abs() < 1e-5);
    }
}
