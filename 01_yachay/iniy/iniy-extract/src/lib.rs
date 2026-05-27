//! iniy-extract — extracción de aserciones atómicas desde chunks.
//!
//! Convierte un pasaje en un conjunto de proposiciones declarativas mínimas,
//! cada una con su opinión autoral inferida (creencia/descreencia/incertidumbre)
//! a partir de marcadores epistémicos del texto ("creo que", "es evidente",
//! "podría ser", "sin duda", modalidad, hedging).
//!
//! MVP heurístico (este crate): splitting por oraciones + marcadores epistémicos
//! españoles. Futuro: backend LLM o modelo local fine-tuneado vía mismo trait.

use anyhow::Result;
use async_trait::async_trait;
use iniy_core::{Asercion, AsercionId, Opinion};
use iniy_ingest::Chunk;

/// Aserción tal como sale del extractor + opcionalmente el nombre de la
/// fuente *citada* — distinta de la fuente del documento. Ej. el doc puede
/// ser de "Wikipedia" pero contener «Según Aristóteles, …»: la fuente
/// citada es "Aristóteles".
#[derive(Debug, Clone)]
pub struct AsercionExtraida {
    pub asercion: Asercion,
    pub fuente_citada_nombre: Option<String>,
}

#[async_trait]
pub trait Extractor: Send + Sync {
    /// Extracción mínima — sin atribución de citas. Implementadores nuevos
    /// pueden enriquecer vía `extraer_con_atribucion`.
    async fn extraer(&self, chunk: &Chunk) -> Result<Vec<Asercion>>;

    /// Como `extraer` pero detectando además citas inline ("Según X,…",
    /// "Para X,…"). Por defecto, devuelve las aserciones de `extraer`
    /// sin marcar ninguna como cita.
    async fn extraer_con_atribucion(&self, chunk: &Chunk) -> Result<Vec<AsercionExtraida>> {
        Ok(self.extraer(chunk).await?.into_iter()
            .map(|a| AsercionExtraida { asercion: a, fuente_citada_nombre: None })
            .collect())
    }
}

/// Stub que devuelve una lista vacía. Útil para tests del pipeline antes
/// de tener un backend real.
pub struct ExtractorVacio;

#[async_trait]
impl Extractor for ExtractorVacio {
    async fn extraer(&self, _chunk: &Chunk) -> Result<Vec<Asercion>> {
        Ok(Vec::new())
    }
}

/// Extractor heurístico: parte el chunk en oraciones por `. ! ? …`, descarta
/// las muy cortas, y para cada una infiere `opinion_autoral` por marcadores
/// epistémicos (refuerzos / hedges / negación).
pub struct ExtractorHeuristico {
    pub min_caracteres: usize,
}

impl Default for ExtractorHeuristico {
    fn default() -> Self {
        Self { min_caracteres: 15 }
    }
}

#[async_trait]
impl Extractor for ExtractorHeuristico {
    async fn extraer(&self, chunk: &Chunk) -> Result<Vec<Asercion>> {
        Ok(self
            .extraer_con_atribucion(chunk)
            .await?
            .into_iter()
            .map(|a| a.asercion)
            .collect())
    }

    async fn extraer_con_atribucion(&self, chunk: &Chunk) -> Result<Vec<AsercionExtraida>> {
        let mut out = Vec::new();
        for oracion in dividir_en_oraciones(&chunk.texto) {
            let t = oracion.trim();
            if t.chars().count() < self.min_caracteres {
                continue;
            }
            let (fuente_citada_nombre, texto_limpio) = match detectar_cita(t) {
                Some((nombre, resto)) => (Some(nombre), resto),
                None => (None, t.to_string()),
            };
            if texto_limpio.chars().count() < self.min_caracteres {
                continue;
            }
            let asercion = Asercion {
                id: AsercionId::nuevo(),
                doc_id: chunk.doc_id,
                chunk_id: chunk.id,
                texto: texto_limpio.clone(),
                opinion_autoral: inferir_opinion(&texto_limpio),
            };
            out.push(AsercionExtraida { asercion, fuente_citada_nombre });
        }
        Ok(out)
    }
}

/// Detecta atribución inline en español. Dos patrones:
/// 1. Prefijo: "Según X, …" / "Para X, …" / "Según X. …" / "Según X: …".
/// 2. Verbo: "<X> afirma|sostiene|dice|escribió|defiende|… que <oración>".
/// Devuelve `(nombre_fuente_citada, resto_sin_atribución)` o `None`.
pub fn detectar_cita(texto: &str) -> Option<(String, String)> {
    if let Some(r) = detectar_cita_prefijo(texto) {
        return Some(r);
    }
    detectar_cita_verbo(texto)
}

fn detectar_cita_prefijo(texto: &str) -> Option<(String, String)> {
    let trim = texto.trim_start();
    for prefijo in ["Según ", "según ", "Para ", "para "] {
        if let Some(rest) = trim.strip_prefix(prefijo) {
            let mut fin_nombre = None;
            for (i, c) in rest.char_indices() {
                if matches!(c, ',' | ':' | '.') {
                    fin_nombre = Some(i);
                    break;
                }
                if i > 80 {
                    return None;
                }
            }
            let fin = fin_nombre?;
            let nombre = rest[..fin].trim().to_string();
            if nombre.is_empty() || nombre.chars().count() > 60 {
                return None;
            }
            let resto = rest[fin + 1..].trim_start().to_string();
            let resto = capitalizar_inicial(&resto);
            return Some((nombre, resto));
        }
    }
    None
}

const VERBOS_ATRIBUCION: &[&str] = &[
    "afirma", "afirmó", "afirmaba",
    "sostiene", "sostuvo", "sostenía",
    "dice", "dijo", "decía",
    "escribe", "escribió", "escribía",
    "defiende", "defendió", "defendía",
    "argumenta", "argumentó", "argumentaba",
    "postula", "postuló", "postulaba",
    "propone", "propuso", "proponía",
    "declara", "declaró", "declaraba",
    "señala", "señaló", "señalaba",
    "opina", "opinó", "opinaba",
    "considera", "consideró", "consideraba",
    "piensa", "pensó", "pensaba",
    "cree", "creyó", "creía",
    "enseña", "enseñó", "enseñaba",
    "asegura", "aseguró", "aseguraba",
    "explica", "explicó", "explicaba",
];

fn detectar_cita_verbo(texto: &str) -> Option<(String, String)> {
    let lower = texto.to_lowercase();
    for v in VERBOS_ATRIBUCION {
        let needle = format!(" {v} que ");
        let Some(idx) = lower.find(&needle) else { continue; };
        // Nombre = todo lo anterior al verbo.
        let nombre_raw = &texto[..idx];
        let nombre = nombre_raw.trim();
        if nombre.is_empty() || nombre.chars().count() > 60 {
            continue;
        }
        // Filtro heurístico: si el "nombre" contiene puntuación que
        // sugiere que en realidad es una oración, descartamos. Una coma
        // sola al final podría ser cierre de aposición y se acepta;
        // pero punto/punto-y-coma o múltiples comas no.
        if nombre.contains('.') || nombre.contains(';') {
            continue;
        }
        if nombre.matches(',').count() > 1 {
            continue;
        }
        // Si el nombre es demasiado corto (1-2 chars) o todo en minúscula
        // (probablemente no es un nombre propio), descartamos.
        if nombre.chars().count() < 3 {
            continue;
        }
        let primera = nombre.chars().next().unwrap();
        if !primera.is_uppercase() {
            // No es un nombre propio capitalizado. Para MVP feo, lo
            // descartamos — evita falsos positivos como "el autor cree que".
            continue;
        }
        let resto_inicio = idx + needle.len();
        let resto = texto[resto_inicio..].trim();
        if resto.chars().count() < 3 {
            continue;
        }
        return Some((nombre.to_string(), capitalizar_inicial(resto)));
    }
    None
}

fn capitalizar_inicial(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn dividir_en_oraciones(texto: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for c in texto.chars() {
        buf.push(c);
        if matches!(c, '.' | '!' | '?' | '…') {
            out.push(std::mem::take(&mut buf));
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf);
    }
    out
}

const REFUERZOS: &[&str] = &[
    "sin duda", "es evidente", "está claro", "obviamente", "indudablemente",
    "siempre", "nunca", "claramente", "por supuesto", "ciertamente",
];
const HEDGES: &[&str] = &[
    "creo que", "podría", "quizás", "quizá", "tal vez", "supongo",
    "parece", "probablemente", "posiblemente", "se dice", "se cree",
    "es posible", "tal vez", "aparentemente",
];
const NEGADORES: &[&str] = &[" no ", "no es ", "no son ", "no fue ", "jamás ", "nunca "];

pub fn inferir_opinion(texto: &str) -> Opinion {
    let t = format!(" {} ", texto.to_lowercase());
    let tiene_refuerzo = REFUERZOS.iter().any(|m| t.contains(m));
    let tiene_hedge = HEDGES.iter().any(|m| t.contains(m));
    let tiene_negador = NEGADORES.iter().any(|m| t.contains(m));

    // Prioridad: refuerzo > hedge > negador > neutral.
    // (Refuerzo gana incluso si hay "nunca" porque "nunca" también es refuerzo
    // de la polaridad expresada, e.g. "nunca olvidaré" = creencia alta.)
    if tiene_refuerzo {
        return Opinion::nueva(0.85, 0.05, 0.10, 0.5).expect("refuerzo bien formada");
    }
    if tiene_hedge {
        return Opinion::nueva(0.30, 0.10, 0.60, 0.5).expect("hedge bien formada");
    }
    if tiene_negador {
        return Opinion::nueva(0.10, 0.75, 0.15, 0.5).expect("negador bien formada");
    }
    // Default: confianza moderada, algo de incertidumbre — el autor afirma sin marcadores.
    Opinion::nueva(0.60, 0.10, 0.30, 0.5).expect("default bien formada")
}

#[cfg(test)]
mod tests {
    use super::*;
    use iniy_core::{ChunkId, DocId};

    fn chunk_con(texto: &str) -> Chunk {
        Chunk {
            id: ChunkId::nuevo(),
            doc_id: DocId::nuevo(),
            orden: 0,
            texto: texto.to_string(),
        }
    }

    #[test]
    fn divide_por_puntuacion_final() {
        let v = dividir_en_oraciones("Hola mundo. ¿Cómo estás? Bien!");
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn refuerzo_sube_creencia() {
        let op = inferir_opinion("Sin duda el sol sale por el este.");
        assert!(op.creencia > 0.8);
    }

    #[test]
    fn hedge_sube_incertidumbre() {
        let op = inferir_opinion("Quizás llueva mañana.");
        assert!(op.incertidumbre > 0.5);
    }

    #[test]
    fn negador_sube_descreencia() {
        let op = inferir_opinion("El sol no sale por el oeste.");
        assert!(op.descreencia > 0.5);
    }

    #[tokio::test]
    async fn extractor_heuristico_descarta_oraciones_cortas() {
        let c = chunk_con("Sí. Esta oración tiene longitud suficiente para superar el umbral. No.");
        let asercs = ExtractorHeuristico::default().extraer(&c).await.unwrap();
        assert_eq!(asercs.len(), 1);
        assert!(asercs[0].texto.starts_with("Esta oración"));
    }

    #[tokio::test]
    async fn extractor_heuristico_propaga_doc_y_chunk_id() {
        let c = chunk_con("Esta oración mide más de quince caracteres y será una aserción.");
        let asercs = ExtractorHeuristico::default().extraer(&c).await.unwrap();
        assert_eq!(asercs.len(), 1);
        assert_eq!(asercs[0].doc_id, c.doc_id);
        assert_eq!(asercs[0].chunk_id, c.id);
    }

    #[test]
    fn detectar_cita_segun_extrae_nombre_y_limpia() {
        let r = detectar_cita("Según Aristóteles, el sol gira alrededor de la Tierra.");
        assert_eq!(r.as_ref().map(|(n, _)| n.as_str()), Some("Aristóteles"));
        assert!(r.unwrap().1.starts_with("El sol"));
    }

    #[test]
    fn detectar_cita_para_extrae_nombre() {
        let r = detectar_cita("Para Heráclito, todo fluye.");
        assert_eq!(r.as_ref().map(|(n, _)| n.as_str()), Some("Heráclito"));
        assert_eq!(r.unwrap().1, "Todo fluye.");
    }

    #[test]
    fn detectar_cita_sin_prefijo_es_none() {
        assert!(detectar_cita("El sol gira alrededor de la Tierra.").is_none());
    }

    #[test]
    fn detectar_cita_nombre_demasiado_largo_es_none() {
        let largo = "x ".repeat(50);
        assert!(detectar_cita(&format!("Según {largo}, algo.")).is_none());
    }

    #[test]
    fn detectar_cita_verbo_afirma_que() {
        let r = detectar_cita("Aristóteles afirma que el cosmos es eterno.");
        assert_eq!(r.as_ref().map(|(n, _)| n.as_str()), Some("Aristóteles"));
        assert!(r.unwrap().1.starts_with("El cosmos"));
    }

    #[test]
    fn detectar_cita_verbo_sostiene_pasado() {
        let r = detectar_cita("Heráclito sostenía que todo fluye constantemente.");
        assert_eq!(r.as_ref().map(|(n, _)| n.as_str()), Some("Heráclito"));
    }

    #[test]
    fn detectar_cita_verbo_dijo_que() {
        let r = detectar_cita("Sócrates dijo que solo sabía que no sabía nada.");
        assert_eq!(r.as_ref().map(|(n, _)| n.as_str()), Some("Sócrates"));
    }

    #[test]
    fn detectar_cita_verbo_nombre_minuscula_es_none() {
        // "el autor sostiene que ..." no es un nombre propio; descartamos.
        assert!(detectar_cita("el autor sostiene que la teoría es falsa.").is_none());
    }

    #[test]
    fn detectar_cita_verbo_sin_que_no_detecta() {
        assert!(detectar_cita("Aristóteles afirma cosas interesantes.").is_none());
    }

    #[test]
    fn detectar_cita_verbo_oracion_compleja_se_descarta() {
        // Si lo que viene antes del verbo contiene puntuación oracional,
        // probablemente no es un nombre propio sino una oración intermedia.
        assert!(detectar_cita("Esto pasa cada día. Aristóteles. Realmente afirma que algo.").is_none());
    }

    #[tokio::test]
    async fn extractor_heuristico_marca_fuente_citada() {
        let c = chunk_con("Según Aristóteles, el cosmos es eterno y no tuvo comienzo en el tiempo.");
        let asercs = ExtractorHeuristico::default().extraer_con_atribucion(&c).await.unwrap();
        assert_eq!(asercs.len(), 1);
        assert_eq!(asercs[0].fuente_citada_nombre.as_deref(), Some("Aristóteles"));
        assert!(asercs[0].asercion.texto.starts_with("El cosmos"));
    }
}
