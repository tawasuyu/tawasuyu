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
//!
//! Dos salidas paralelas, ambas UI-agnósticas:
//! - [`file_icon`] → un emoji `&'static str` (granular: 🦀/🐍/…), pensado
//!   para frontends de **terminal** donde el emoji rinde nativo.
//! - [`file_kind`] → un [`FileKind`] semántico (categoría gruesa), que un
//!   frontend gráfico mapea a su propio set de iconos vectoriales (p. ej.
//!   `llimphi-icons` en el shell Llimphi) sin acoplar este crate a la UI.

use std::path::Path;

/// Categoría semántica de una entrada del filesystem, para que un
/// frontend elija un icono. Gruesa a propósito: un set de iconos
/// vectoriales monocromos no distingue pdf-rojo de doc-azul, así que
/// colapsamos los documentos en uno y los lenguajes de código en otro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FileKind {
    Folder,
    Symlink,
    Image,
    Audio,
    Video,
    Archive,
    /// pdf / doc / hoja de cálculo / presentación / texto / markdown.
    Document,
    /// Cualquier lenguaje de programación o script.
    Code,
    /// json / toml / yaml / xml / config.
    Data,
    Font,
    /// Binario ejecutable u objeto (so/o/wasm/elf…).
    Executable,
    /// Archivo regular sin categoría reconocida.
    Generic,
}

/// Clasifica una entrada ya stat-eada en una [`FileKind`]. Mismo orden
/// de decisión que [`file_icon`]: symlink y directorio mandan sobre la
/// extensión.
pub fn file_kind(path: &Path, is_dir: bool, is_executable: bool, is_symlink: bool) -> FileKind {
    if is_symlink {
        return FileKind::Symlink;
    }
    if is_dir {
        return FileKind::Folder;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if let Some(ext) = ext.as_deref() {
        if let Some(kind) = kind_for_ext(ext) {
            return kind;
        }
    }
    if is_executable {
        FileKind::Executable
    } else {
        FileKind::Generic
    }
}

/// Categoría por extensión (ya en minúsculas). `None` = no reconocida.
fn kind_for_ext(ext: &str) -> Option<FileKind> {
    let kind = match ext {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "tiff" | "avif" => {
            FileKind::Image
        }
        "mp3" | "wav" | "flac" | "ogg" | "opus" | "m4a" | "aac" | "mka" => FileKind::Audio,
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "ivf" | "m4v" => FileKind::Video,
        "zip" | "tar" | "gz" | "xz" | "zst" | "bz2" | "7z" | "rar" | "tgz" => FileKind::Archive,
        "pdf" | "doc" | "docx" | "odt" | "rtf" | "xls" | "xlsx" | "ods" | "csv" | "tsv" | "ppt"
        | "pptx" | "odp" | "md" | "markdown" | "txt" | "rst" | "adoc" => FileKind::Document,
        "rs" | "py" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "c" | "h" | "cpp" | "hpp"
        | "cc" | "go" | "java" | "kt" | "rb" | "php" | "lua" | "sh" | "bash" | "zsh" | "fish"
        | "swift" | "zig" | "hs" | "ml" => FileKind::Code,
        "json" | "toml" | "yaml" | "yml" | "xml" | "ini" | "conf" | "lock" => FileKind::Data,
        "ttf" | "otf" | "woff" | "woff2" => FileKind::Font,
        "wasm" | "so" | "a" | "o" | "dll" | "dylib" | "elf" | "bin" => FileKind::Executable,
        _ => return None,
    };
    Some(kind)
}

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

    #[test]
    fn file_kind_classifies_by_type_then_extension() {
        // Tipo manda sobre extensión.
        assert_eq!(file_kind(Path::new("algo.rs"), true, false, false), FileKind::Folder);
        assert_eq!(file_kind(Path::new("link.png"), false, false, true), FileKind::Symlink);
        // Por extensión (colapsa lenguajes en Code, documentos en Document).
        assert_eq!(file_kind(Path::new("main.rs"), false, false, false), FileKind::Code);
        assert_eq!(file_kind(Path::new("app.py"), false, false, false), FileKind::Code);
        assert_eq!(file_kind(Path::new("foto.PNG"), false, false, false), FileKind::Image);
        assert_eq!(file_kind(Path::new("doc.pdf"), false, false, false), FileKind::Document);
        assert_eq!(file_kind(Path::new("notas.md"), false, false, false), FileKind::Document);
        assert_eq!(file_kind(Path::new("data.tar.gz"), false, false, false), FileKind::Archive);
        assert_eq!(file_kind(Path::new("cfg.toml"), false, false, false), FileKind::Data);
        assert_eq!(file_kind(Path::new("font.ttf"), false, false, false), FileKind::Font);
        // Fallback por bit ejecutable.
        assert_eq!(file_kind(Path::new("run"), false, true, false), FileKind::Executable);
        assert_eq!(file_kind(Path::new("LICENSE"), false, false, false), FileKind::Generic);
    }
}
