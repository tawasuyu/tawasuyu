//! Apertura de archivos del navegador con la app que corresponda (Fase 11d).
//!
//! El navegador del sidebar lista archivos miembros de una Mónada (su ruta real
//! en disco, resuelta por nouser). Al abrir uno (right-click), enrutamos a la app
//! adecuada en dos pasos:
//!
//! 1. **Apps nativas de gioser** (`app-bus`): si alguna app declara manejar el
//!    mime del archivo (`handles = ["…"]` en su manifiesto), la lanzamos con la
//!    ruta como argumento (sustitución freedesktop `%f`/`%u` vía
//!    [`app_bus::AppEntry::open`]). Las apps de la suite tienen prioridad.
//! 2. **Fallback del sistema** (`xdg-open`): si ninguna app nativa lo maneja,
//!    delegamos en las asociaciones del escritorio.
//!
//! El mime se deriva de la **extensión** con una tabla acotada (sin leer el
//! archivo — la UI no hace I/O de disco). Lo desconocido cae directo a `xdg-open`,
//! que de todas formas respeta las asociaciones del usuario. (El discernimiento
//! por contenido de `shuma-discern` sería el upgrade, a costa de leer una muestra.)

use app_bus::AppRegistry;

/// Mapea la extensión de `path` (lo que va tras el último punto, en minúsculas) a
/// un mime canónico. `None` si no hay extensión o no está en la tabla — el caller
/// cae a `xdg-open`.
pub fn mime_for_path(path: &str) -> Option<&'static str> {
    // La extensión es el segmento tras el último punto del último componente.
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e)?.to_ascii_lowercase();
    Some(match ext.as_str() {
        // Código y texto estructurado.
        "rs" => "text/x-rust",
        "py" => "text/x-python",
        "js" | "mjs" | "cjs" => "text/javascript",
        "ts" | "tsx" => "text/typescript",
        "c" | "h" => "text/x-c",
        "cpp" | "cc" | "cxx" | "hpp" => "text/x-c++",
        "go" => "text/x-go",
        "java" => "text/x-java",
        "rb" => "text/x-ruby",
        "sh" | "bash" | "zsh" => "application/x-shellscript",
        "toml" => "application/toml",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        // Texto plano y documentos.
        "md" | "markdown" => "text/markdown",
        "rst" => "text/x-rst",
        "txt" | "log" | "ron" => "text/plain",
        "csv" => "text/csv",
        "pdf" => "application/pdf",
        // Imágenes.
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        // Audio.
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        // Video.
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mov" => "video/quicktime",
        // Archivos comprimidos.
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" | "tgz" => "application/gzip",
        "7z" => "application/x-7z-compressed",
        _ => return None,
    })
}

/// Qué hizo [`open_file`], para log/diagnóstico.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// Una app nativa de gioser abrió el archivo (su `label`).
    NativeApp(String),
    /// Se delegó en `xdg-open`.
    SystemDefault,
}

/// Abre `path` con la app del registro que declare su mime; si ninguna, cae a
/// `xdg-open`. No bloquea (spawnea y olvida). Devuelve qué ruta tomó.
pub fn open_file(registry: &AppRegistry, path: &str) -> Opened {
    if let Some(mime) = mime_for_path(path) {
        if let Some(app) = registry.handlers_for(mime).into_iter().next() {
            match app.open(path) {
                Ok(_) => return Opened::NativeApp(app.label.clone()),
                Err(e) => {
                    eprintln!("pata · {} no pudo abrir {path}: {e}; uso xdg-open", app.label);
                }
            }
        }
    }
    crate::spawn_cmd(&format!("xdg-open {}", crate::shell_quote(path)));
    Opened::SystemDefault
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_por_extension_comun() {
        assert_eq!(mime_for_path("/proj/src/lib.rs"), Some("text/x-rust"));
        assert_eq!(mime_for_path("foto.PNG"), Some("image/png")); // case-insensitive
        assert_eq!(mime_for_path("a/b/notas.md"), Some("text/markdown"));
        assert_eq!(mime_for_path("clip.mp4"), Some("video/mp4"));
        assert_eq!(mime_for_path("data.json"), Some("application/json"));
    }

    #[test]
    fn sin_extension_o_desconocida_es_none() {
        assert_eq!(mime_for_path("README"), None);
        assert_eq!(mime_for_path("/etc/hosts"), None);
        assert_eq!(mime_for_path("archivo.xyzqux"), None);
    }

    #[test]
    fn extension_de_un_punto_en_directorio_no_confunde() {
        // El punto está en un componente de directorio, no en el archivo.
        assert_eq!(mime_for_path("/home/.config/Makefile"), None);
        // Pero un dotfile con extensión real sí.
        assert_eq!(mime_for_path("/home/.bashrc.md"), Some("text/markdown"));
    }
}
