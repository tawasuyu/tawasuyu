//! Decorador inteligente del output — encuentra "cosas interactivas"
//! (paths existentes, URLs, referencias `path:line:col`) y emite
//! [`Decoration`]s para que el frontend las pinte con un click handler.
//!
//! Es lo que convierte un `ls` en una lista clickeable o un mensaje de
//! cargo con `--> src/main.rs:42:7` en un link al editor — sin que el
//! comando tenga que cooperar.
//!
//! Diseño:
//!
//! - **Lookup, no parser**: no asumimos el shape del comando (no
//!   parseamos "ls -la" vs "ls"). Sólo miramos los tokens del output y
//!   probamos el sistema de archivos (stat real, una syscall barata).
//!   Funciona para `ls`, `find`, `tree`, `grep -l`, `git status`,
//!   incluso para tu output ad-hoc con paths en medio.
//! - **Anclado a cwd del run**: el path relativo se resuelve contra el
//!   directorio donde corrió el comando, no contra el shell ahora —
//!   crítico para que `:cd` posteriores no rompan las decoraciones de
//!   runs viejos.
//! - **No regex**: scanner manual de prefijos URL y delimitadores. Más
//!   barato y predecible. Para `path:line:col` usamos un parser simple
//!   en el inicio de línea (formato típico de grep/rg/compiladores).

use std::fs::Metadata;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Una decoración aplicable a un rango de bytes de una línea.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decoration {
    /// Inicio del rango (byte offset desde el comienzo de la línea).
    pub start: usize,
    /// Fin exclusivo del rango.
    pub end: usize,
    /// Qué representa el rango y qué acción dispara un click.
    pub kind: DecorationKind,
}

/// Tipos de decoración que el shell pinta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecorationKind {
    /// Un path que existe — el frontend lo pinta clickeable y la acción
    /// depende de `is_dir`/`is_executable`.
    Path {
        /// Path absoluto (joined con cwd si era relativo).
        abs: PathBuf,
        is_dir: bool,
        is_executable: bool,
        is_symlink: bool,
    },
    /// Una URL — el frontend la abre con `xdg-open` o equivalente.
    Url(String),
    /// `path:line[:col]` típico de grep/rg/compiladores. El frontend
    /// abre `path` en el editor saltando a `line_no`.
    GrepRef {
        abs: PathBuf,
        line_no: u32,
        col: Option<u32>,
    },
}

/// Punto de entrada: detecta decoraciones para una línea. `cwd` se usa
/// para resolver paths relativos. El orden es:
///
/// 1. URLs primero (las más específicas y no se confunden con paths).
/// 2. GrepRefs (formato `path:N[:N]` al inicio de línea).
/// 3. Paths sueltos en los tokens restantes.
///
/// Decoraciones que solapan se descartan (gana la más temprana).
pub fn decorate_line(line: &str, cwd: &Path) -> Vec<Decoration> {
    let mut out: Vec<Decoration> = Vec::new();
    find_urls(line, &mut out);
    find_grep_refs(line, cwd, &mut out);
    find_paths(line, cwd, &mut out);
    out.sort_by_key(|d| d.start);
    // Si hubo solapamientos, los `find_*` ya los respetaron — pero por
    // higiene re-chequeamos contiguamente.
    let mut merged: Vec<Decoration> = Vec::with_capacity(out.len());
    for d in out {
        if let Some(prev) = merged.last() {
            if d.start < prev.end {
                continue; // gana el primero
            }
        }
        merged.push(d);
    }
    merged
}

// --- URL detection ---

const URL_PREFIXES: &[&str] = &["http://", "https://", "file://", "ftp://", "ssh://"];

fn find_urls(line: &str, out: &mut Vec<Decoration>) {
    for prefix in URL_PREFIXES {
        let mut search_from = 0;
        while let Some(rel) = line[search_from..].find(prefix) {
            let start = search_from + rel;
            let mut end = start + prefix.len();
            let bytes = line.as_bytes();
            while end < bytes.len() {
                let c = bytes[end];
                if c.is_ascii_whitespace()
                    || matches!(c, b'<' | b'>' | b'`' | b'"' | b'\'' | b'(' | b')')
                {
                    break;
                }
                end += 1;
            }
            // Trim puntuación final típica de prosa: .,;:
            while end > start + prefix.len() {
                let last = bytes[end - 1];
                if matches!(last, b'.' | b',' | b';' | b':' | b']' | b'!' | b'?') {
                    end -= 1;
                } else {
                    break;
                }
            }
            if end > start + prefix.len() {
                out.push(Decoration {
                    start,
                    end,
                    kind: DecorationKind::Url(line[start..end].to_string()),
                });
            }
            search_from = end;
        }
    }
}

// --- GrepRef detection ---

fn find_grep_refs(line: &str, cwd: &Path, out: &mut Vec<Decoration>) {
    // Patrón estándar de grep/rg/compiladores: el path comienza al
    // inicio de la línea (eventualmente tras whitespace) y termina en
    // el primer `:` que va seguido de dígitos.
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let path_start = i;
    // Cargo/rustc emiten `--> path:line:col` o `<path>:<line>:<col>`.
    // Saltamos el prefijo `--> ` si está.
    if line[i..].starts_with("--> ") {
        i += 4;
    } else if line[i..].starts_with("error[") || line[i..].starts_with("warning[") {
        // Mensajes de cargo en otra forma — los manejamos por la flecha.
        return;
    }
    let path_start = if i > path_start { i } else { path_start };
    // Avanzar hasta encontrar un `:<digit>`. Permitimos `:` dentro del
    // path si el carácter que sigue no es un dígito.
    let mut path_end = path_start;
    while path_end < bytes.len() {
        let c = bytes[path_end];
        if c == b':' {
            if path_end + 1 < bytes.len() && bytes[path_end + 1].is_ascii_digit() {
                break;
            }
        }
        if c.is_ascii_whitespace() {
            return; // espacio antes de `:` → no es grep ref
        }
        path_end += 1;
    }
    if path_end >= bytes.len() || path_end == path_start {
        return;
    }
    let path_str = &line[path_start..path_end];
    let abs = resolve_path(path_str, cwd);
    if !abs.is_some_and(|p| p.exists()) {
        return;
    }
    let abs = resolve_path(path_str, cwd).expect("checked");
    // Tras `:`, leer el número de línea.
    let mut p = path_end + 1;
    let line_start = p;
    while p < bytes.len() && bytes[p].is_ascii_digit() {
        p += 1;
    }
    if p == line_start {
        return;
    }
    let line_no: u32 = line[line_start..p].parse().unwrap_or(0);
    // Opcional `:<col>`.
    let mut col: Option<u32> = None;
    let mut end = p;
    if p < bytes.len() && bytes[p] == b':' && p + 1 < bytes.len() && bytes[p + 1].is_ascii_digit() {
        let col_start = p + 1;
        let mut q = col_start;
        while q < bytes.len() && bytes[q].is_ascii_digit() {
            q += 1;
        }
        col = line[col_start..q].parse().ok();
        end = q;
    }
    out.push(Decoration {
        start: path_start,
        end,
        kind: DecorationKind::GrepRef { abs, line_no, col },
    });
}

// --- Path detection ---

fn find_paths(line: &str, cwd: &Path, out: &mut Vec<Decoration>) {
    for (start, end) in tokens_with_ranges(line) {
        // Saltar tokens muy cortos (`.`, `..`, números, etc.) — no
        // ganamos nada decorándolos y bajamos falsos positivos.
        if end - start < 2 {
            continue;
        }
        if overlaps_any(start, end, out) {
            continue;
        }
        let text = &line[start..end];
        // Caracteres de puntuación al borde (paréntesis, comas, etc.)
        // los recortamos antes de probar el path.
        let (trim_start, trim_end) = trim_punctuation(text);
        if trim_end <= trim_start {
            continue;
        }
        let path_text = &text[trim_start..trim_end];
        let Some(path) = resolve_path(path_text, cwd) else {
            continue;
        };
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let is_symlink = meta.file_type().is_symlink();
        let is_dir = if is_symlink {
            std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false)
        } else {
            meta.is_dir()
        };
        let is_executable = is_exec_unix(&meta);
        out.push(Decoration {
            start: start + trim_start,
            end: start + trim_end,
            kind: DecorationKind::Path { abs: path, is_dir, is_executable, is_symlink },
        });
    }
}

fn resolve_path(token: &str, cwd: &Path) -> Option<PathBuf> {
    if token.is_empty() {
        return None;
    }
    let candidate = if token.starts_with('~') {
        let home = std::env::var("HOME").ok()?;
        if token == "~" {
            PathBuf::from(home)
        } else if let Some(rest) = token.strip_prefix("~/") {
            PathBuf::from(home).join(rest)
        } else {
            return None;
        }
    } else if Path::new(token).is_absolute() {
        PathBuf::from(token)
    } else {
        cwd.join(token)
    };
    Some(candidate)
}

fn tokens_with_ranges(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if start < i {
            out.push((start, i));
        }
    }
    out
}

fn trim_punctuation(text: &str) -> (usize, usize) {
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut end = bytes.len();
    while start < end
        && matches!(bytes[start], b'(' | b'[' | b'<' | b'`' | b'"' | b'\'' | b',')
    {
        start += 1;
    }
    while end > start
        && matches!(
            bytes[end - 1],
            b')' | b']' | b'>' | b'`' | b'"' | b'\'' | b',' | b'.' | b';' | b':'
        )
    {
        end -= 1;
    }
    (start, end)
}

fn overlaps_any(start: usize, end: usize, ds: &[Decoration]) -> bool {
    ds.iter().any(|d| d.start < end && start < d.end)
}

fn is_exec_unix(meta: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn paths_in(line: &str, cwd: &Path) -> Vec<(usize, usize, PathBuf)> {
        decorate_line(line, cwd)
            .into_iter()
            .filter_map(|d| match d.kind {
                DecorationKind::Path { abs, .. } => Some((d.start, d.end, abs)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn detects_existing_files_in_a_line() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("alfa.txt"), "x").unwrap();
        fs::create_dir(d.path().join("beta")).unwrap();
        // Output típico de `ls` separado por whitespace.
        let line = "alfa.txt  beta  no-existe";
        let p = paths_in(line, d.path());
        let names: Vec<_> = p.iter().map(|(_, _, p)| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"alfa.txt".to_string()));
        assert!(names.contains(&"beta".to_string()));
        assert!(!names.contains(&"no-existe".to_string()));
    }

    #[test]
    fn ranges_point_at_the_token_in_the_line() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("foo"), "x").unwrap();
        let line = "ver foo aquí";
        let p = paths_in(line, d.path());
        assert_eq!(p.len(), 1);
        let (s, e, _) = p[0];
        assert_eq!(&line[s..e], "foo");
    }

    #[test]
    fn directory_is_marked_as_dir() {
        let d = tempdir().unwrap();
        fs::create_dir(d.path().join("subdir")).unwrap();
        let line = "subdir";
        let dec = decorate_line(line, d.path());
        assert_eq!(dec.len(), 1);
        match &dec[0].kind {
            DecorationKind::Path { is_dir, .. } => assert!(*is_dir),
            _ => panic!("expected Path"),
        }
    }

    #[test]
    fn executable_bit_detected_on_unix() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let d = tempdir().unwrap();
            let p = d.path().join("script");
            fs::write(&p, "#!/bin/sh\n").unwrap();
            let mut perms = fs::metadata(&p).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&p, perms).unwrap();
            let dec = decorate_line("script", d.path());
            match &dec[0].kind {
                DecorationKind::Path { is_executable, .. } => assert!(*is_executable),
                _ => panic!("expected Path"),
            }
        }
    }

    #[test]
    fn absolute_paths_work_too() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("x"), "").unwrap();
        let abs = d.path().join("x");
        let line = format!("toca {}", abs.display());
        let p = paths_in(&line, Path::new("/"));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].2, abs);
    }

    #[test]
    fn punctuation_around_path_is_trimmed() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("foo"), "").unwrap();
        // En prosa: "abrí (foo)."
        let line = "abrí (foo).";
        let p = paths_in(line, d.path());
        assert_eq!(p.len(), 1);
        let (s, e, _) = p[0];
        assert_eq!(&line[s..e], "foo");
    }

    #[test]
    fn urls_are_detected() {
        let line = "ver https://example.com/x.html, también http://foo.bar";
        let dec = decorate_line(line, Path::new("/"));
        let urls: Vec<_> = dec
            .iter()
            .filter_map(|d| match &d.kind {
                DecorationKind::Url(u) => Some(u.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(urls.len(), 2);
        assert!(urls.contains(&"https://example.com/x.html".to_string()));
        assert!(urls.contains(&"http://foo.bar".to_string()));
    }

    #[test]
    fn url_strips_trailing_punctuation() {
        let line = "abrir https://foo.bar.";
        let dec = decorate_line(line, Path::new("/"));
        match &dec[0].kind {
            DecorationKind::Url(u) => assert_eq!(u, "https://foo.bar"),
            _ => panic!("expected Url"),
        }
    }

    #[test]
    fn grep_ref_at_start_of_line() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("src.rs"), "x").unwrap();
        let line = "src.rs:42:7: error here";
        let dec = decorate_line(line, d.path());
        let refs: Vec<_> = dec
            .iter()
            .filter_map(|d| match &d.kind {
                DecorationKind::GrepRef { abs, line_no, col } => Some((abs.clone(), *line_no, *col)),
                _ => None,
            })
            .collect();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].1, 42);
        assert_eq!(refs[0].2, Some(7));
    }

    #[test]
    fn grep_ref_without_column() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("foo.txt"), "x").unwrap();
        let line = "foo.txt:10: contenido";
        let dec = decorate_line(line, d.path());
        match &dec[0].kind {
            DecorationKind::GrepRef { line_no, col, .. } => {
                assert_eq!(*line_no, 10);
                assert_eq!(*col, None);
            }
            _ => panic!("expected GrepRef"),
        }
    }

    #[test]
    fn cargo_arrow_ref_is_picked_up() {
        let d = tempdir().unwrap();
        fs::create_dir(d.path().join("src")).unwrap();
        fs::write(d.path().join("src/main.rs"), "x").unwrap();
        // El prefijo `   --> ` con tabs/espacios variables.
        let line = "   --> src/main.rs:5:9";
        let dec = decorate_line(line, d.path());
        let r = dec.iter().find_map(|d| match &d.kind {
            DecorationKind::GrepRef { line_no, col, .. } => Some((*line_no, *col)),
            _ => None,
        });
        assert_eq!(r, Some((5, Some(9))));
    }

    #[test]
    fn overlaps_dont_double_decorate() {
        let d = tempdir().unwrap();
        fs::write(d.path().join("src.rs"), "x").unwrap();
        // El grep ref ocupa "src.rs:42"; el path "src.rs" solo se
        // tragaría aparte si no detectamos solape.
        let line = "src.rs:42: x";
        let dec = decorate_line(line, d.path());
        // Esperamos UNA decoración (la GrepRef cubre el path).
        assert_eq!(dec.len(), 1);
        assert!(matches!(dec[0].kind, DecorationKind::GrepRef { .. }));
    }

    #[test]
    fn empty_line_yields_nothing() {
        assert!(decorate_line("", Path::new("/")).is_empty());
        assert!(decorate_line("   ", Path::new("/")).is_empty());
    }

    #[test]
    fn nonexistent_paths_are_not_decorated() {
        let d = tempdir().unwrap();
        let line = "esto-no-existe.txt foo-tampoco.bin";
        assert!(paths_in(line, d.path()).is_empty());
    }
}
