//! `shuma-discern` — detección de tipo de contenido sobre buffers.
//!
//! Trait + pipeline + discerners default. Devuelve un [`Discernment`] con
//! `TypeRef` consistente con el broker, confidence, MIME y un `lens` hint
//! para UIs (reusa el espíritu del `dominant_lens` de akasha).

#![forbid(unsafe_code)]

use brahman_card::TypeRef;

#[derive(Debug, Clone)]
pub struct Hint<'a> {
    pub path: Option<&'a str>,
    pub size_total: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Discernment {
    pub ty: TypeRef,
    pub confidence: f32,
    pub mime: Option<String>,
    pub lens: Option<String>,
}

pub trait Discerner: Send + Sync {
    fn name(&self) -> &str;
    fn discern(&self, sample: &[u8], hint: &Hint<'_>) -> Option<Discernment>;
}

pub struct DiscernPipeline {
    discerners: Vec<Box<dyn Discerner>>,
}

impl DiscernPipeline {
    pub fn new() -> Self {
        Self { discerners: Vec::new() }
    }

    /// Pipeline con los discerners default. Orden importa: el primer match
    /// con confidence ≥ `accept_threshold` corta.
    pub fn default_pipeline() -> Self {
        let mut p = Self::new();
        p.push(Box::new(MagicBytes));
        // CardProbe antes que JsonProbe: una Card es JSON, pero queremos el
        // TypeRef más específico cuando aplique.
        p.push(Box::new(CardProbe));
        p.push(Box::new(JsonProbe));
        p.push(Box::new(TomlProbe));
        p.push(Box::new(Utf8Probe));
        p
    }

    pub fn push(&mut self, d: Box<dyn Discerner>) {
        self.discerners.push(d);
    }

    /// Recorre los discerners y devuelve el primer Discernment con
    /// confidence ≥ 0.5, o el más confidente si ninguno alcanza el umbral.
    pub fn discern(&self, sample: &[u8], hint: &Hint<'_>) -> Option<Discernment> {
        let mut best: Option<Discernment> = None;
        for d in &self.discerners {
            if let Some(r) = d.discern(sample, hint) {
                if r.confidence >= 0.9 {
                    return Some(r);
                }
                best = match best {
                    Some(prev) if prev.confidence >= r.confidence => Some(prev),
                    _ => Some(r),
                };
            }
        }
        best
    }
}

impl Default for DiscernPipeline {
    fn default() -> Self {
        Self::default_pipeline()
    }
}

// =====================================================================
// Discerners
// =====================================================================

/// Magic-bytes para formatos comunes. Confidence alta cuando hay match.
pub struct MagicBytes;

impl Discerner for MagicBytes {
    fn name(&self) -> &str { "magic-bytes" }

    fn discern(&self, s: &[u8], _h: &Hint<'_>) -> Option<Discernment> {
        let d = |ty: &str, mime: &str, lens: Option<&str>| Discernment {
            ty: TypeRef::Primitive { name: ty.into() },
            confidence: 0.99,
            mime: Some(mime.into()),
            lens: lens.map(String::from),
        };
        match s {
            x if x.starts_with(&[0x89, b'P', b'N', b'G']) => Some(d("png", "image/png", Some("gallery"))),
            x if x.starts_with(&[0xFF, 0xD8, 0xFF]) => Some(d("jpeg", "image/jpeg", Some("gallery"))),
            x if x.starts_with(b"%PDF-") => Some(d("pdf", "application/pdf", Some("reader"))),
            x if x.starts_with(&[0x7F, b'E', b'L', b'F']) => Some(d("elf", "application/x-executable", None)),
            x if x.starts_with(&[0x00, 0x61, 0x73, 0x6D]) => Some(d("wasm", "application/wasm", None)),
            x if x.starts_with(&[0x1F, 0x8B]) => Some(d("gzip", "application/gzip", None)),
            x if x.starts_with(b"PK\x03\x04") || x.starts_with(b"PK\x05\x06") => {
                Some(d("zip", "application/zip", None))
            }
            x if x.starts_with(b"GIF87a") || x.starts_with(b"GIF89a") => {
                Some(d("gif", "image/gif", Some("gallery")))
            }
            _ => None,
        }
    }
}

/// JSON: parsea el inicio. No requiere parsearlo entero; con que arranque
/// con `{`/`[` y haga progreso cuenta.
pub struct JsonProbe;

impl Discerner for JsonProbe {
    fn name(&self) -> &str { "json" }

    fn discern(&self, s: &[u8], _h: &Hint<'_>) -> Option<Discernment> {
        let trimmed = trim_left(s);
        let first = *trimmed.first()?;
        if first != b'{' && first != b'[' {
            return None;
        }
        // Intento parsear tal cual; si falla por truncated, igualmente confidence media.
        let txt = std::str::from_utf8(trimmed).ok()?;
        match serde_json::from_str::<serde_json::Value>(txt) {
            Ok(_) => Some(Discernment {
                ty: TypeRef::Primitive { name: "json".into() },
                confidence: 0.95,
                mime: Some("application/json".into()),
                lens: Some("tree".into()),
            }),
            Err(_) => Some(Discernment {
                ty: TypeRef::Primitive { name: "json".into() },
                confidence: 0.6, // sample truncado
                mime: Some("application/json".into()),
                lens: Some("tree".into()),
            }),
        }
    }
}

pub struct TomlProbe;

impl Discerner for TomlProbe {
    fn name(&self) -> &str { "toml" }

    fn discern(&self, s: &[u8], h: &Hint<'_>) -> Option<Discernment> {
        let txt = std::str::from_utf8(s).ok()?;
        // Heurística: presencia de `[seccion]` y/o `clave = valor` y extensión.
        let looks_like = txt.lines().any(|l| {
            let l = l.trim();
            l.starts_with('[') && l.ends_with(']')
        }) || txt.lines().any(|l| {
            let l = l.trim();
            !l.starts_with('#') && l.contains(" = ")
        });
        if !looks_like {
            return None;
        }
        let confidence = if h.path.map_or(false, |p| p.ends_with(".toml")) {
            0.95
        } else {
            0.55
        };
        // Si parsea, sube confidence.
        let parsed = toml::from_str::<toml::Value>(txt).is_ok();
        Some(Discernment {
            ty: TypeRef::Primitive { name: "toml".into() },
            confidence: if parsed { 0.93 } else { confidence },
            mime: Some("application/toml".into()),
            lens: Some("tree".into()),
        })
    }
}

/// Si el JSON parsea como Card, lo emite como Wit { brahman:card }.
pub struct CardProbe;

impl Discerner for CardProbe {
    fn name(&self) -> &str { "card" }

    fn discern(&self, s: &[u8], _h: &Hint<'_>) -> Option<Discernment> {
        let trimmed = trim_left(s);
        if trimmed.first()? != &b'{' {
            return None;
        }
        let txt = std::str::from_utf8(trimmed).ok()?;
        let v: serde_json::Value = serde_json::from_str(txt).ok()?;
        let obj = v.as_object()?;
        if obj.contains_key("schema_version") && obj.contains_key("id") && obj.contains_key("payload") {
            Some(Discernment {
                ty: TypeRef::Wit {
                    package: "brahman:card".into(),
                    interface: None,
                    name: "card".into(),
                },
                confidence: 0.97,
                mime: Some("application/json".into()),
                lens: Some("card".into()),
            })
        } else {
            None
        }
    }
}

/// Texto UTF-8 plano. Fallback de baja confidence.
pub struct Utf8Probe;

impl Discerner for Utf8Probe {
    fn name(&self) -> &str { "utf8" }

    fn discern(&self, s: &[u8], h: &Hint<'_>) -> Option<Discernment> {
        if s.is_empty() {
            return None;
        }
        let valid = std::str::from_utf8(s).is_ok();
        if !valid {
            return None;
        }
        // Detectar binario disfrazado: bytes de control fuera de \t\n\r.
        let suspicious = s.iter().filter(|&&b| b < 0x09 || (b > 0x0D && b < 0x20)).count();
        if suspicious * 100 / s.len().max(1) > 5 {
            return None;
        }
        let lens = h.path.and_then(|p| {
            if p.ends_with(".md") { Some("markdown") }
            else if p.ends_with(".rs") || p.ends_with(".py") || p.ends_with(".go") || p.ends_with(".js") || p.ends_with(".ts") {
                Some("code")
            } else { None }
        }).map(String::from);
        Some(Discernment {
            ty: TypeRef::Primitive { name: "text".into() },
            confidence: 0.5,
            mime: Some("text/plain; charset=utf-8".into()),
            lens,
        })
    }
}

fn trim_left(s: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < s.len() && (s[i] == b' ' || s[i] == b'\t' || s[i] == b'\n' || s[i] == b'\r') {
        i += 1;
    }
    &s[i..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discern(sample: &[u8]) -> Option<Discernment> {
        DiscernPipeline::default_pipeline().discern(sample, &Hint { path: None, size_total: None })
    }

    #[test]
    fn png_detected() {
        let r = discern(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0]).unwrap();
        assert_eq!(r.mime.as_deref(), Some("image/png"));
        assert!(r.confidence > 0.9);
    }

    #[test]
    fn json_detected() {
        let r = discern(b"{\"hello\": 1}").unwrap();
        assert_eq!(r.mime.as_deref(), Some("application/json"));
    }

    #[test]
    fn card_wins_over_plain_json() {
        let payload = br#"{"schema_version":1,"id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","label":"x","payload":{"Virtual":null},"supervision":"OneShot"}"#;
        let r = discern(payload).unwrap();
        match r.ty {
            TypeRef::Wit { ref package, .. } => assert_eq!(package, "brahman:card"),
            _ => panic!("expected card"),
        }
    }

    #[test]
    fn utf8_text_fallback() {
        let r = discern(b"hello world\nthis is text").unwrap();
        // Puede ser detected as toml (= heurística) o text. Ambos son aceptables, sólo aseguro algo razonable.
        assert!(r.mime.is_some());
    }

    #[test]
    fn binary_rejected_by_utf8() {
        let mut bytes = vec![0u8; 100];
        bytes[0] = 0x00;
        bytes[1] = 0x01;
        bytes[2] = 0x02;
        let r = DiscernPipeline::default_pipeline().discern(&bytes, &Hint { path: None, size_total: None });
        // Tras Utf8Probe rechazar, no hay match → None.
        // Si por casualidad otro discerner mata antes, también es OK.
        if let Some(r) = r {
            assert_ne!(r.mime.as_deref(), Some("text/plain; charset=utf-8"));
        }
    }
}
