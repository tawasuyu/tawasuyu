//! Iconos por tipo de archivo — un glifo emoji que precede al nombre en
//! el output decorado para que un `ls` se lea como un explorador de
//! archivos en vez de una lista de tokens.
//!
//! Agnóstico de UI: devuelve un `&'static str` (emoji o símbolo) que el
//! frontend pinta tal cual antes del nombre clickeable. La elección es
//! por **tipo** primero (dir/symlink/ejecutable) y, para archivos
//! regulares, por **extensión** — el mismo criterio que un file manager.
//!
//! Espíritu del repo: no inventamos un set de iconos propio cuando el
//! `lens` de `shuma-discern` ya clasifica por familia (gallery/audio/
//! video/...). Acá cubrimos el caso del shell, donde sólo tenemos el
//! path en disco (sin samplear bytes), así que vamos por extensión.

use std::path::Path;

/// Icono para una entrada del filesystem ya stat-eada. El orden de
/// decisión importa: symlink y directorio mandan sobre la extensión
/// (un `fotos/` sigue siendo carpeta aunque termine en algo raro).
pub fn file_icon(path: &Path, is_dir: bool, is_executable: bool, is_symlink: bool) -> &'static str {
    if is_symlink {
        return "🔗";
    }
    if is_dir {
        return "📁";
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if let Some(ext) = ext.as_deref() {
        if let Some(icon) = icon_for_ext(ext) {
            return icon;
        }
    }
    // Sin extensión reconocida: un binario ejecutable se distingue de un
    // archivo de texto plano.
    if is_executable {
        "⚙️"
    } else {
        "📄"
    }
}

/// Icono por extensión (ya en minúsculas). `None` = no reconocida, el
/// caller cae al genérico archivo/ejecutable.
fn icon_for_ext(ext: &str) -> Option<&'static str> {
    let icon = match ext {
        // Imágenes
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "tiff" | "avif" => "🖼️",
        // Audio
        "mp3" | "wav" | "flac" | "ogg" | "opus" | "m4a" | "aac" | "mka" => "🎵",
        // Video
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "ivf" | "m4v" => "🎬",
        // Archivos comprimidos / paquetes
        "zip" | "tar" | "gz" | "xz" | "zst" | "bz2" | "7z" | "rar" | "tgz" => "📦",
        // Documentos
        "pdf" => "📕",
        "doc" | "docx" | "odt" | "rtf" => "📘",
        "xls" | "xlsx" | "ods" | "csv" | "tsv" => "📊",
        "ppt" | "pptx" | "odp" => "📙",
        // Texto / markup
        "md" | "markdown" | "txt" | "rst" | "adoc" => "📝",
        // Código
        "rs" => "🦀",
        "py" => "🐍",
        "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" => "📜",
        "c" | "h" | "cpp" | "hpp" | "cc" | "go" | "java" | "kt" | "rb" | "php" | "lua"
        | "sh" | "bash" | "zsh" | "fish" | "swift" | "zig" | "hs" | "ml" => "📜",
        // Datos / config
        "json" | "toml" | "yaml" | "yml" | "xml" | "ini" | "conf" | "lock" => "🛠️",
        // Fuentes
        "ttf" | "otf" | "woff" | "woff2" => "🔤",
        // Binarios / objetos
        "wasm" | "so" | "a" | "o" | "dll" | "dylib" | "elf" | "bin" => "⚙️",
        _ => return None,
    };
    Some(icon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_and_symlink_win_over_extension() {
        // Una carpeta llamada "algo.rs" sigue siendo carpeta.
        assert_eq!(file_icon(Path::new("algo.rs"), true, false, false), "📁");
        // Un symlink manda sobre todo.
        assert_eq!(file_icon(Path::new("link.png"), false, false, true), "🔗");
    }

    #[test]
    fn known_extensions_get_their_icon() {
        assert_eq!(file_icon(Path::new("foto.PNG"), false, false, false), "🖼️");
        assert_eq!(file_icon(Path::new("main.rs"), false, false, false), "🦀");
        assert_eq!(file_icon(Path::new("notas.md"), false, false, false), "📝");
        assert_eq!(file_icon(Path::new("data.tar.gz"), false, false, false), "📦");
    }

    #[test]
    fn unknown_extension_falls_back_by_exec_bit() {
        assert_eq!(file_icon(Path::new("raro.qwerty"), false, false, false), "📄");
        assert_eq!(file_icon(Path::new("run"), false, true, false), "⚙️");
        // Sin extensión, no ejecutable → archivo genérico.
        assert_eq!(file_icon(Path::new("LICENSE"), false, false, false), "📄");
    }
}
