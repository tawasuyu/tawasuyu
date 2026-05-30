//! Helpers puros: rutas, expansión de `~`, etiquetas legibles de
//! backend/intención/transformación, y el reloj unix.

use std::path::{Path, PathBuf};

use pluma_cuerpo::Intencion;
use pluma_llm::BackendKind;
use pluma_transform::TipoTransformacion;

pub(crate) fn expandir_ruta(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    } else if raw == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(raw)
}

pub(crate) fn extension_lower(p: &Path) -> Option<String> {
    p.extension().map(|e| e.to_string_lossy().to_lowercase())
}

pub(crate) fn etiqueta_backend(k: BackendKind) -> &'static str {
    match k {
        BackendKind::Mock => "mock",
        BackendKind::Gemini => "gemini",
        BackendKind::Anthropic => "anthropic",
        BackendKind::DeepSeek => "deepseek",
        BackendKind::Cohere => "cohere",
        BackendKind::Ollama => "ollama",
    }
}

pub(crate) fn etiqueta_intencion(i: &Intencion) -> String {
    match i {
        Intencion::Original => "original".into(),
        Intencion::Traduccion => "traducción".into(),
        Intencion::Tono { etiqueta } => format!("tono {etiqueta}"),
        Intencion::Resumen {
            palabras_objetivo: Some(n),
        } => format!("resumen ≈{n}p"),
        Intencion::Resumen {
            palabras_objetivo: None,
        } => "resumen".into(),
        Intencion::Reescritura { .. } => "reescritura".into(),
        Intencion::Anotacion => "anotación".into(),
        Intencion::Custom { kind } => kind.clone(),
    }
}

pub(crate) fn etiqueta_tipo(t: &TipoTransformacion) -> String {
    match t {
        TipoTransformacion::Identidad => "identidad".into(),
        TipoTransformacion::Traducir { lengua_destino } => format!("traducir → {lengua_destino}"),
        TipoTransformacion::Tono { etiqueta } => format!("tono {etiqueta}"),
        TipoTransformacion::Resumir {
            palabras_objetivo: Some(n),
        } => format!("resumir ≈{n}p"),
        TipoTransformacion::Resumir {
            palabras_objetivo: None,
        } => "resumir".into(),
        TipoTransformacion::Reescribir { .. } => "reescribir".into(),
        TipoTransformacion::Custom { kind, .. } => kind.clone(),
    }
}

pub(crate) fn recortar(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut o: String = s.chars().take(max.saturating_sub(1)).collect();
        o.push('…');
        o
    }
}

pub(crate) fn ruta_sled() -> PathBuf {
    if let Ok(p) = std::env::var("PLUMA_APP_SLED") {
        return PathBuf::from(p);
    }
    let base = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".cache"))
                .unwrap_or_else(|_| PathBuf::from(".cache"))
        });
    base.join("gioser").join("pluma-app").join("pluma.sled")
}

pub(crate) fn ahora_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
