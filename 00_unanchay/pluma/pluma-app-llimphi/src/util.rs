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
        BackendKind::ClaudeCli => "claude-cli",
    }
}

/// Etiqueta legible de la intención de un cuerpo (sólo para mostrar — derivada
/// del enum, nunca persistida ni comparada como string).
pub(crate) fn etiqueta_intencion(i: &Intencion) -> String {
    use rimay_localize::{t, t_args};
    match i {
        Intencion::Original => t("pluma-app-intent-original"),
        Intencion::Traduccion => t("pluma-app-intent-translation"),
        Intencion::Tono { etiqueta } => {
            t_args("pluma-app-intent-tone", &[("t", etiqueta.clone().into())])
        }
        Intencion::Resumen {
            palabras_objetivo: Some(n),
        } => t_args("pluma-app-summary-n", &[("n", n.to_string().into())]),
        Intencion::Resumen {
            palabras_objetivo: None,
        } => t("pluma-app-intent-summary"),
        Intencion::Reescritura { .. } => t("pluma-app-intent-rewrite"),
        Intencion::Anotacion => t("pluma-app-intent-annotation"),
        Intencion::Custom { kind } => kind.clone(),
    }
}

/// Etiqueta legible del tipo de transformación (sólo para mostrar).
pub(crate) fn etiqueta_tipo(t: &TipoTransformacion) -> String {
    use rimay_localize::{t as tr, t_args};
    match t {
        TipoTransformacion::Identidad => tr("pluma-app-type-identity"),
        TipoTransformacion::Traducir { lengua_destino } => {
            t_args("pluma-app-type-translate", &[("l", lengua_destino.clone().into())])
        }
        TipoTransformacion::Tono { etiqueta } => {
            t_args("pluma-app-intent-tone", &[("t", etiqueta.clone().into())])
        }
        TipoTransformacion::Resumir {
            palabras_objetivo: Some(n),
        } => t_args("pluma-app-summarize-n", &[("n", n.to_string().into())]),
        TipoTransformacion::Resumir {
            palabras_objetivo: None,
        } => tr("pluma-app-type-summarize"),
        TipoTransformacion::Reescribir { .. } => tr("pluma-app-type-rewrite"),
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
    base.join("tawasuyu").join("pluma-app-llimphi").join("pluma.sled")
}

/// Ruta del archivo de presets (prompts reutilizables del diente Derivar-IA),
/// junto al sled: `<...>/pluma-app-llimphi/presets.txt` — un prompt por línea.
pub(crate) fn ruta_presets() -> PathBuf {
    ruta_sled().with_file_name("presets.txt")
}

/// Carga los presets persistidos (un prompt por línea, ignorando vacíos).
pub(crate) fn cargar_presets() -> Vec<String> {
    std::fs::read_to_string(ruta_presets())
        .map(|s| {
            s.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Persiste los presets (best-effort; un fallo de IO no es fatal).
pub(crate) fn guardar_presets(presets: &[String]) {
    let _ = std::fs::write(ruta_presets(), presets.join("\n"));
}

/// Ruta del listado de proyectos recientes (rutas `.pluma`, una por línea).
pub(crate) fn ruta_recientes() -> PathBuf {
    ruta_sled().with_file_name("proyectos.txt")
}

/// Carga las rutas de proyectos recientes (existan o no en disco).
pub(crate) fn cargar_recientes() -> Vec<PathBuf> {
    std::fs::read_to_string(ruta_recientes())
        .map(|s| {
            s.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Persiste las rutas recientes (best-effort).
pub(crate) fn guardar_recientes(rutas: &[PathBuf]) {
    let texto = rutas
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(ruta_recientes(), texto);
}

/// Ruta del listado de proyectos ABIERTOS (para reabrirlos al iniciar).
pub(crate) fn ruta_abiertos() -> PathBuf {
    ruta_sled().with_file_name("proyectos_abiertos.txt")
}

/// Carga las rutas de proyectos que estaban abiertos.
pub(crate) fn cargar_abiertos() -> Vec<PathBuf> {
    std::fs::read_to_string(ruta_abiertos())
        .map(|s| {
            s.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Persiste las rutas de proyectos abiertos (best-effort).
pub(crate) fn guardar_abiertos(rutas: &[PathBuf]) {
    let texto = rutas
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(ruta_abiertos(), texto);
}

pub(crate) fn ahora_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
