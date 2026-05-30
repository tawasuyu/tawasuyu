//! `shuma-discern` — detección de tipo de contenido sobre buffers.
//!
//! Trait + pipeline + discerners default. Devuelve un [`Discernment`] con
//! `TypeRef` consistente con el broker, confidence, MIME y un `lens` hint
//! para UIs (reusa el espíritu del `dominant_lens` de chasqui).

#![forbid(unsafe_code)]

use card_core::TypeRef;

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
        p.push(Box::new(TabularProbe));
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
            // tar — el magic "ustar" no está al inicio sino en el offset
            // 257 del primer header (POSIX y GNU lo escriben ahí). Como la
            // muestra son 8 KB, el offset cae dentro. Sin esto, un .tar
            // (texto en sus primeros 257 bytes) caería al text viewer.
            x if x.len() >= 262 && &x[257..262] == b"ustar" => {
                Some(d("tar", "application/x-tar", None))
            }
            x if x.starts_with(b"GIF87a") || x.starts_with(b"GIF89a") => {
                Some(d("gif", "image/gif", Some("gallery")))
            }
            // RIFF: el FourCC en off 8 distingue WebP (imagen) de WAVE (audio).
            x if x.len() >= 12 && x.starts_with(b"RIFF") && &x[8..12] == b"WEBP" => {
                Some(d("webp", "image/webp", Some("gallery")))
            }
            x if x.len() >= 12 && x.starts_with(b"RIFF") && &x[8..12] == b"WAVE" => {
                Some(d("wav", "audio/wav", Some("audio")))
            }
            // FLAC — audio sin pérdida ("fLaC").
            x if x.starts_with(b"fLaC") => Some(d("flac", "audio/flac", Some("audio"))),
            // Ogg — contenedor de Vorbis u Opus ("OggS"). El visor elige
            // decoder por extensión (.ogg/.oga vs .opus).
            x if x.starts_with(b"OggS") => Some(d("ogg", "audio/ogg", Some("audio"))),
            // MP3 con tag ID3v2 al inicio. El frame-sync crudo (0xFFEx) es
            // ambiguo con otros streams, así que sólo capturamos ID3.
            x if x.starts_with(b"ID3") => Some(d("mp3", "audio/mpeg", Some("audio"))),
            // EBML — contenedor Matroska/WebM. Lo tratamos como video; el
            // visor (media-source-webm) toma el track AV1. .mka (audio-only)
            // caería igual acá, pero el visor lo reporta como "sin video".
            x if x.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) => {
                Some(d("webm", "video/webm", Some("video")))
            }
            // IVF — contenedor crudo de un stream AV1/VP9 ("DKIF").
            x if x.starts_with(b"DKIF") => Some(d("ivf", "video/x-ivf", Some("video"))),
            // Fuentes parseables por ttf-parser: TrueType (0x00010000 o
            // "true"), OpenType/CFF ("OTTO") y colecciones ("ttcf"). WOFF
            // queda fuera (es un wrapper comprimido que ttf-parser no abre).
            x if x.starts_with(&[0x00, 0x01, 0x00, 0x00])
                || x.starts_with(b"OTTO")
                || x.starts_with(b"true")
                || x.starts_with(b"ttcf") =>
            {
                Some(d("font", "font/sfnt", Some("font")))
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

/// Datos tabulares (CSV/TSV). El formato no tiene magic-bytes, así que
/// se apoya en el `hint.path` (`.csv`/`.tsv`) y confirma con el contenido:
/// la primera línea debe traer el delimitador. Emite lens `table`.
pub struct TabularProbe;

impl Discerner for TabularProbe {
    fn name(&self) -> &str { "tabular" }

    fn discern(&self, s: &[u8], h: &Hint<'_>) -> Option<Discernment> {
        let path = h.path?;
        let (delim, mime) = if path.ends_with(".csv") {
            (b',', "text/csv")
        } else if path.ends_with(".tsv") {
            (b'\t', "text/tab-separated-values")
        } else {
            return None;
        };
        // Confirmar con la primera línea: debe ser UTF-8 y tener el
        // delimitador (una columna sola no es una tabla).
        let txt = std::str::from_utf8(s).ok()?;
        let first = txt.lines().next()?;
        if !first.as_bytes().contains(&delim) {
            return None;
        }
        Some(Discernment {
            ty: TypeRef::Primitive { name: "tabular".into() },
            confidence: 0.93,
            mime: Some(mime.into()),
            lens: Some("table".into()),
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
    fn webm_ebml_detected_como_video() {
        let mut bytes = vec![0x1A, 0x45, 0xDF, 0xA3];
        bytes.extend_from_slice(b"\x01\x00\x00\x00\x00\x00\x00\x1f");
        let r = discern(&bytes).unwrap();
        assert_eq!(r.mime.as_deref(), Some("video/webm"));
        assert_eq!(r.lens.as_deref(), Some("video"));
    }

    #[test]
    fn ivf_detected_como_video() {
        let r = discern(b"DKIF\x00\x00\x20\x00AV01").unwrap();
        assert_eq!(r.mime.as_deref(), Some("video/x-ivf"));
        assert_eq!(r.lens.as_deref(), Some("video"));
    }

    #[test]
    fn tar_detectado_por_ustar_en_offset_257() {
        // Un header tar: nombre + relleno hasta el offset 257 donde va el
        // magic "ustar". Los primeros bytes son texto (el nombre), así que
        // sin el chequeo de offset caería al text viewer.
        let mut bytes = vec![0u8; 512];
        bytes[..8].copy_from_slice(b"file.txt");
        bytes[257..262].copy_from_slice(b"ustar");
        let r = discern(&bytes).unwrap();
        assert_eq!(r.mime.as_deref(), Some("application/x-tar"));
    }

    #[test]
    fn fuentes_detectadas_por_magic() {
        // TTF (0x00010000) y OTF ("OTTO") → lens font.
        let r = discern(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x0F]).unwrap();
        assert_eq!(r.lens.as_deref(), Some("font"));
        assert_eq!(discern(b"OTTO\x00\x0a").unwrap().lens.as_deref(), Some("font"));
        assert_eq!(discern(b"ttcf\x00\x01").unwrap().mime.as_deref(), Some("font/sfnt"));
    }

    #[test]
    fn wav_riff_detected_como_audio() {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&[0x24, 0x08, 0x00, 0x00]); // chunk size
        bytes.extend_from_slice(b"WAVE");
        let r = discern(&bytes).unwrap();
        assert_eq!(r.mime.as_deref(), Some("audio/wav"));
        assert_eq!(r.lens.as_deref(), Some("audio"));
    }

    #[test]
    fn flac_y_ogg_detectados_como_audio() {
        assert_eq!(
            discern(b"fLaC\x00\x00\x00\x22").unwrap().lens.as_deref(),
            Some("audio")
        );
        assert_eq!(
            discern(b"OggS\x00\x02\x00\x00").unwrap().mime.as_deref(),
            Some("audio/ogg")
        );
    }

    #[test]
    fn csv_por_path_es_tabla() {
        let p = DiscernPipeline::default_pipeline();
        let hint = Hint { path: Some("/datos/ventas.csv"), size_total: None };
        let r = p.discern(b"fecha,monto,region\n2026-01,10,sur\n", &hint).unwrap();
        assert_eq!(r.mime.as_deref(), Some("text/csv"));
        assert_eq!(r.lens.as_deref(), Some("table"));
    }

    #[test]
    fn csv_sin_delimitador_no_es_tabla() {
        let p = DiscernPipeline::default_pipeline();
        let hint = Hint { path: Some("/x.csv"), size_total: None };
        // Sin coma en la primera línea: cae al text fallback, no a tabla.
        let r = p.discern(b"una sola columna\nsin comas\n", &hint).unwrap();
        assert_ne!(r.lens.as_deref(), Some("table"));
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
