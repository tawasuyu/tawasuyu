//! iniy-nli — Natural Language Inference entre pares de aserciones.
//!
//! Dado (premisa, hipótesis) devuelve probabilidades de entailment / contradiction / neutral.
//!
//! Backends:
//! - `MotorNeutro` — siempre neutral. Para cablear el pipeline.
//! - `MotorNliLexico` — heurístico MVP: Jaccard sobre tokens + detección de
//!   polaridad (negadores). Detecta los casos obvios ("X es bueno" vs "X no es
//!   bueno") y se calla en los ambiguos. Suficiente para un primer pase que
//!   muestre contradicciones reales en `iniy contradictions`.
//! - Futuro: ONNX-RT + DeBERTa-v3-MNLI o XNLI multilingüe.

use anyhow::Result;
use async_trait::async_trait;
use iniy_core::{Asercion, RelacionNli};
use std::collections::HashSet;

#[async_trait]
pub trait MotorNli: Send + Sync {
    async fn evaluar(&self, premisa: &Asercion, hipotesis: &Asercion) -> Result<RelacionNli>;
}

/// Motor neutro: devuelve siempre "neutral 1.0".
pub struct MotorNeutro;

#[async_trait]
impl MotorNli for MotorNeutro {
    async fn evaluar(&self, _p: &Asercion, _h: &Asercion) -> Result<RelacionNli> {
        Ok(RelacionNli { entailment: 0.0, contradiction: 0.0, neutral: 1.0 })
    }
}

/// Motor léxico simétrico: jaccard de tokens de contenido + flip por polaridad opuesta.
pub struct MotorNliLexico {
    pub umbral_overlap: f32,
}

impl Default for MotorNliLexico {
    fn default() -> Self {
        Self { umbral_overlap: 0.30 }
    }
}

#[async_trait]
impl MotorNli for MotorNliLexico {
    async fn evaluar(&self, p: &Asercion, h: &Asercion) -> Result<RelacionNli> {
        Ok(relacion_lexica(&p.texto, &h.texto, self.umbral_overlap))
    }
}

pub fn relacion_lexica(a: &str, b: &str, umbral: f32) -> RelacionNli {
    let ta = tokens_contenido(a);
    let tb = tokens_contenido(b);
    let jacc = jaccard(&ta, &tb);

    if jacc < umbral {
        return RelacionNli { entailment: 0.0, contradiction: 0.0, neutral: 1.0 };
    }
    // Score en [0.4, 0.95] proporcional al overlap por encima del umbral.
    let score = (0.4 + (jacc - umbral) / (1.0 - umbral) * 0.55).clamp(0.4, 0.95);
    // Contradicción por dos vías combinables: (a) flip de negación ("X" vs
    // "no X"), (b) oposición antonímica ("X reduce Y" vs "X aumenta Y"). Si
    // ambas ocurren se cancelan ("no reduce" ≈ "aumenta" → coinciden), por eso
    // es un XOR.
    let neg_flip = tiene_negacion(a) != tiene_negacion(b);
    let anton_flip = oposicion_antonimica(&ta, &tb);
    if neg_flip ^ anton_flip {
        RelacionNli { entailment: 0.0, contradiction: score, neutral: 1.0 - score }
    } else {
        RelacionNli { entailment: score, contradiction: 0.0, neutral: 1.0 - score }
    }
}

/// Pares de antónimos por *stem* (prefijo). Si una aserción contiene un stem de
/// un lado y la otra del lado opuesto, se oponen aunque no haya negación
/// explícita. Los stems son prefijos largos (≥4 chars salvo casos seguros) para
/// acotar colisiones; el matcheo es por `starts_with` sobre tokens de contenido.
/// Heurística MVP — el día que entre un modelo NLI real (DeBERTa/XNLI) esto pasa
/// a ser un fallback.
const ANTONIMOS: &[(&[&str], &[&str])] = &[
    (&["reduc", "disminu", "decrec", "merma"], &["aument", "increment", "eleva"]),
    (&["mejor"], &["empeor", "deterior"]),
    (&["segur", "inocu", "saludable"], &["peligr", "riesgos", "nociv", "tóxic", "toxic", "dañin", "dañ", "perjudic"]),
    (&["benefic", "favorab", "positiv", "ventaj"], &["perjudic", "negativ", "contraproduc", "desventaj"]),
    (&["permit", "autoriz", "habilit"], &["prohib", "impid"]),
    (&["verdad", "cierto", "correct", "válid", "valid"], &["fals", "incorrect", "erróne", "errón", "inválid", "invalid"]),
    (&["acept", "aprob", "respald"], &["rechaz", "refut", "desmient"]),
    (&["crec", "expand"], &["decrec", "contrae", "encog"]),
];

/// ¿Algún token arranca con alguno de los stems?
fn tokens_match_any(toks: &HashSet<String>, stems: &[&str]) -> bool {
    toks.iter().any(|t| stems.iter().any(|s| t.starts_with(s)))
}

/// `true` si A toca un lado de un par de antónimos y B el opuesto.
fn oposicion_antonimica(a: &HashSet<String>, b: &HashSet<String>) -> bool {
    ANTONIMOS.iter().any(|(lado1, lado2)| {
        (tokens_match_any(a, lado1) && tokens_match_any(b, lado2))
            || (tokens_match_any(a, lado2) && tokens_match_any(b, lado1))
    })
}

const STOPWORDS: &[&str] = &[
    "el", "la", "los", "las", "un", "una", "unos", "unas", "y", "o", "u", "ni",
    "de", "del", "en", "a", "al", "es", "son", "fue", "fueron", "ser", "estar",
    "que", "se", "lo", "le", "les", "por", "para", "con", "sin", "su", "sus",
    "mi", "tu", "como", "más", "muy", "este", "esta", "estos", "estas",
    "ese", "esa", "esos", "esas", "aquel", "aquella", "pero", "ya", "sí",
    "también", "porque", "cuando", "donde", "qué", "quién", "ha", "han", "hay",
];

fn tokens_contenido(s: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let lower = s.to_lowercase();
    for raw in lower.split(|c: char| !c.is_alphanumeric()) {
        let t = raw.trim();
        if t.is_empty() || t.chars().count() < 3 {
            continue;
        }
        if STOPWORDS.contains(&t) {
            continue;
        }
        out.insert(t.to_string());
    }
    out
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    inter / union
}

const NEGADORES: &[&str] = &[" no ", "no es", "no son", "no fue", "no fueron", "jamás", "nunca", " sin "];

fn tiene_negacion(s: &str) -> bool {
    let t = format!(" {} ", s.to_lowercase());
    NEGADORES.iter().any(|n| t.contains(n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iniy_core::{AsercionId, ChunkId, DocId, Opinion};

    fn asercion(t: &str) -> Asercion {
        Asercion {
            id: AsercionId::nuevo(),
            doc_id: DocId::nuevo(),
            chunk_id: ChunkId::nuevo(),
            texto: t.into(),
            opinion_autoral: Opinion::nueva(0.6, 0.1, 0.3, 0.5).unwrap(),
        }
    }

    #[tokio::test]
    async fn neutro_devuelve_neutral_uno() {
        let r = MotorNeutro.evaluar(&asercion("a"), &asercion("b")).await.unwrap();
        assert_eq!(r.neutral, 1.0);
    }

    #[test]
    fn polaridad_opuesta_alta_overlap_es_contradiccion() {
        let r = relacion_lexica(
            "El sol siempre sale por el este de la Tierra",
            "El sol no sale por el este de la Tierra",
            0.30,
        );
        assert!(r.contradiction > 0.4, "esperaba contradicción alta, got {:?}", r);
        assert_eq!(r.entailment, 0.0);
    }

    #[test]
    fn misma_polaridad_alta_overlap_es_entailment() {
        let r = relacion_lexica(
            "La memoria humana reconstruye los recuerdos cada vez que los evocas",
            "Cada evocación reescribe los recuerdos de la memoria humana",
            0.30,
        );
        assert!(r.entailment > 0.3, "esperaba entailment moderado, got {:?}", r);
        assert_eq!(r.contradiction, 0.0);
    }

    #[test]
    fn antonimo_alta_overlap_es_contradiccion() {
        // Mismo sujeto/objeto, verbo antónimo (reduce ↔ aumenta), sin negación:
        // el motor viejo lo daba como entailment; ahora es contradicción.
        let r = relacion_lexica(
            "El café reduce el riesgo de enfermedad cardiovascular",
            "El café aumenta el riesgo de enfermedad cardiovascular",
            0.30,
        );
        assert!(r.contradiction > 0.4, "esperaba contradicción por antónimo, got {:?}", r);
        assert_eq!(r.entailment, 0.0);
    }

    #[test]
    fn antonimo_seguro_peligroso_es_contradiccion() {
        let r = relacion_lexica(
            "La cafeína es segura para los adultos sanos",
            "La cafeína es peligrosa para los adultos",
            0.30,
        );
        assert!(r.contradiction > 0.0, "esperaba contradicción seguro/peligroso, got {:?}", r);
    }

    #[test]
    fn negacion_mas_antonimo_se_cancela_a_entailment() {
        // "no reduce" ≈ "aumenta": doble flip → coinciden (XOR).
        let r = relacion_lexica(
            "El café no reduce el riesgo de enfermedad cardiovascular",
            "El café aumenta el riesgo de enfermedad cardiovascular",
            0.30,
        );
        assert!(r.entailment > 0.0, "doble flip debía coincidir, got {:?}", r);
        assert_eq!(r.contradiction, 0.0);
    }

    #[test]
    fn mismo_lado_antonimo_no_es_contradiccion() {
        // Ambas en el lado "aumenta": no hay oposición.
        let r = relacion_lexica(
            "El café aumenta el riesgo de enfermedad cardiovascular",
            "El café incrementa el riesgo de enfermedad cardiovascular",
            0.30,
        );
        assert_eq!(r.contradiction, 0.0, "mismo lado no debe contradecir, got {:?}", r);
        assert!(r.entailment > 0.0);
    }

    #[test]
    fn sin_overlap_es_neutral() {
        let r = relacion_lexica("Los gatos duermen mucho", "Júpiter es un planeta gaseoso", 0.30);
        assert_eq!(r.neutral, 1.0);
    }

    #[tokio::test]
    async fn motor_lexico_implementa_trait() {
        let m = MotorNliLexico::default();
        let r = m
            .evaluar(
                &asercion("El sol siempre sale por el este"),
                &asercion("El sol no sale por el este"),
            )
            .await
            .unwrap();
        assert!(r.contradiction > 0.0);
    }
}
