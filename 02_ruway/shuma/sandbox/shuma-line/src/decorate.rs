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
    /// SHA de git (hex 7..40 chars). El frontend sugiere `git show
    /// <sha>` en el input.
    GitSha(String),
    /// Referencia tipo `#1234` — issue/PR de GitHub/GitLab/Gitea. Sin
    /// click action porque la url depende del repo; el frontend puede
    /// resolverla mediante `git config remote.origin.url` + reglas
    /// por host en el futuro. Hoy sólo se pinta destacado.
    IssueRef(u32),
    /// Run contiguo de caracteres de **box-drawing** Unicode
    /// (U+2500..U+257F y U+2580..U+259F). El frontend los renderiza
    /// con la fuente monospace + color accent para que los bordes
    /// calcen entre filas y se vean como una caja real.
    BoxDraw,
    /// Número suelto (conteos, tamaños, ids), con sufijo de unidad
    /// opcional (`248`, `1024K`, `1.3 GiB` captura sólo `1.3`+unidad
    /// pegada). Sin acción de click — sólo color.
    Number,
    /// Fecha u hora reconocible: ISO (`2026-06-12`), hora (`10:12`,
    /// `10:12:33`) o `mes día` estilo `ls -l` (`jun  9`). Sólo color.
    DateTime,
    /// Palabra de estado con carga semántica (error/warning/ok) — el
    /// frontend la tiñe rojo/amarillo/verde para escanear de un vistazo.
    Severity(Severity),
    /// Versión tipo semver, con `v` opcional (`v1.2.3`, `0.7.0`).
    Version,
    /// Porcentaje (`85%`, `99.7%`). Sólo color.
    Percent,
    /// Máscara de permisos estilo `ls -l` (`drwxr-xr-x`, `-rw-r--r--+`).
    PermMask,
}

/// Nivel semántico de una palabra de estado en el output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warn,
    Ok,
}

/// Punto de entrada: detecta decoraciones para una línea. `cwd` se usa
/// para resolver paths relativos. El orden importa porque las primeras
/// reclaman el rango antes que las siguientes:
///
/// 1. **Box-drawing** — son chars no-ASCII contiguos, sólo necesitan
///    detección visual; van primero para que un `─` accidental no se
///    interprete como nada raro después.
/// 2. **URLs** — las más específicas, prefijo único.
/// 3. **GrepRefs** — `path:N[:N]` al inicio de línea (cargo, grep, rg).
/// 4. **GitSha** + **IssueRef** — patrones reconocibles fuera de
///    contextos de paths.
/// 5. **Paths** — generales, captura los tokens restantes.
///
/// Las decoraciones que solapan se descartan (gana la más temprana).
pub fn decorate_line(line: &str, cwd: &Path) -> Vec<Decoration> {
    let mut out: Vec<Decoration> = Vec::new();
    find_box_draw(line, &mut out);
    find_urls(line, &mut out);
    find_grep_refs(line, cwd, &mut out);
    find_git_shas(line, &mut out);
    find_issue_refs(line, &mut out);
    find_paths(line, cwd, &mut out);
    // Coloreo semántico de relleno — va al FINAL para que cualquier
    // decoración accionable (path/url/sha) le gane el rango. De lo más
    // específico a lo más genérico, cada finder respeta `overlaps_any`.
    find_perm_masks(line, &mut out);
    find_versions(line, &mut out);
    find_datetimes(line, &mut out);
    find_percents(line, &mut out);
    find_severities(line, &mut out);
    find_numbers(line, &mut out);
    out.sort_by_key(|d| d.start);
    let mut merged: Vec<Decoration> = Vec::with_capacity(out.len());
    for d in out {
        if let Some(prev) = merged.last() {
            if d.start < prev.end {
                continue;
            }
        }
        merged.push(d);
    }
    merged
}

// --- Box-drawing detection ---

/// `true` si `c` pertenece a las áreas Unicode de líneas/bordes que
/// las CLIs modernas (gemini, claude, cargo, etc.) usan para dibujar
/// cajas: `Box Drawing` (U+2500..U+257F) y `Block Elements`
/// (U+2580..U+259F).
fn is_box_draw(c: char) -> bool {
    let u = c as u32;
    (0x2500..=0x257F).contains(&u) || (0x2580..=0x259F).contains(&u)
}

fn find_box_draw(line: &str, out: &mut Vec<Decoration>) {
    let mut start: Option<usize> = None;
    for (i, c) in line.char_indices() {
        if is_box_draw(c) {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            out.push(Decoration {
                start: s,
                end: i,
                kind: DecorationKind::BoxDraw,
            });
        }
    }
    if let Some(s) = start {
        out.push(Decoration {
            start: s,
            end: line.len(),
            kind: DecorationKind::BoxDraw,
        });
    }
}

// --- Git SHA + Issue ref detection ---

/// SHAs de git: hex (0-9a-f) de 7..40 chars de largo. Para reducir
/// falsos positivos exigimos: rodeado por boundary (inicio/fin de
/// línea, whitespace o puntuación) y al menos un dígito O al menos
/// una letra (puro `aaaaaaa` raramente es un SHA).
fn find_git_shas(line: &str, out: &mut Vec<Decoration>) {
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        // Buscar inicio de un run hex
        if !is_boundary(line, i) {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        let mut has_digit = false;
        let mut has_alpha = false;
        while end < n {
            let c = bytes[end];
            if c.is_ascii_digit() {
                has_digit = true;
                end += 1;
            } else if matches!(c, b'a'..=b'f') {
                has_alpha = true;
                end += 1;
            } else {
                break;
            }
        }
        let len = end - start;
        if len >= 7 && len <= 40 && has_digit && has_alpha && is_end_boundary(line, end) {
            // Evitar solapar con decoraciones previas (URLs, paths,
            // etc.) — chequeo barato.
            if !overlaps_any(start, end, out) {
                out.push(Decoration {
                    start,
                    end,
                    kind: DecorationKind::GitSha(line[start..end].to_string()),
                });
            }
        }
        i = end.max(start + 1);
    }
}

/// `#NN` típico de issues/PRs en repos. Acepta 1..7 dígitos (millones
/// de issues son raros y ayudan a evitar falsos positivos con hashes
/// numéricos o números de línea).
fn find_issue_refs(line: &str, out: &mut Vec<Decoration>) {
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if bytes[i] == b'#' && is_boundary(line, i) {
            let start = i;
            let mut end = i + 1;
            while end < n && bytes[end].is_ascii_digit() {
                end += 1;
            }
            let digits = end - i - 1;
            if (1..=7).contains(&digits) && is_end_boundary(line, end) {
                if !overlaps_any(start, end, out) {
                    if let Ok(num) = line[start + 1..end].parse::<u32>() {
                        out.push(Decoration {
                            start,
                            end,
                            kind: DecorationKind::IssueRef(num),
                        });
                    }
                }
            }
            i = end.max(start + 1);
        } else {
            i += 1;
        }
    }
}

/// `true` si justo a la izquierda de `pos` hay un carácter no-palabra
/// (o estamos en inicio de línea). Lo usamos en find_git_shas /
/// find_issue_refs para anclar el INICIO del patrón.
fn is_boundary(line: &str, pos: usize) -> bool {
    let bytes = line.as_bytes();
    if pos == 0 || pos == bytes.len() {
        return true;
    }
    let prev = bytes[pos - 1];
    !(prev.is_ascii_alphanumeric() || prev == b'_')
}

/// `true` si justo a la derecha de `pos` hay un carácter no-palabra
/// (o estamos en fin de línea). Ancla el FIN del patrón.
fn is_end_boundary(line: &str, pos: usize) -> bool {
    let bytes = line.as_bytes();
    if pos >= bytes.len() {
        return true;
    }
    let next = bytes[pos];
    !(next.is_ascii_alphanumeric() || next == b'_')
}

// --- Coloreo semántico de relleno (números, fechas, severidades…) ---

/// Máscara de permisos `ls -l`: tipo + 9 de rwx, con sufijo ACL/SELinux
/// opcional (`+`/`.`). Muy específica, va primero entre los de relleno.
fn find_perm_masks(line: &str, out: &mut Vec<Decoration>) {
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i + 10 <= n {
        if !is_boundary(line, i) || !matches!(bytes[i], b'-' | b'd' | b'l' | b'b' | b'c' | b's' | b'p') {
            i += 1;
            continue;
        }
        let perms_ok = (1..10).all(|k| {
            let c = bytes[i + k];
            let esperado: &[u8] = match k % 3 {
                1 => b"r-",
                2 => b"w-",
                _ => b"xsStT-",
            };
            esperado.contains(&c)
        });
        let mut end = i + 10;
        if perms_ok {
            if end < n && matches!(bytes[end], b'+' | b'.') {
                end += 1;
            }
            if is_end_boundary(line, end) && !overlaps_any(i, end, out) {
                out.push(Decoration { start: i, end, kind: DecorationKind::PermMask });
            }
            i = end;
        } else {
            i += 1;
        }
    }
}

/// Versiones semver con `v` opcional: `v1.2.3`, `0.7.0`, `1.45.0-rc1`.
/// Exige al menos DOS puntos (un `1.5` suelto es un número decimal).
fn find_versions(line: &str, out: &mut Vec<Decoration>) {
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if !is_boundary(line, i) {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        if j < n && bytes[j] == b'v' {
            j += 1;
        }
        let mut grupos = 0;
        loop {
            let d0 = j;
            while j < n && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j == d0 {
                break;
            }
            grupos += 1;
            if j < n && bytes[j] == b'.' && j + 1 < n && bytes[j + 1].is_ascii_digit() {
                j += 1;
            } else {
                break;
            }
        }
        // Sufijo pre-release pegado (`-rc1`, `-beta.2`).
        if grupos >= 3 && j < n && bytes[j] == b'-' {
            let mut k = j + 1;
            while k < n && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'.') {
                k += 1;
            }
            if k > j + 1 {
                j = k;
            }
        }
        if grupos >= 3 && is_end_boundary(line, j) && !overlaps_any(start, j, out) {
            out.push(Decoration { start, end: j, kind: DecorationKind::Version });
            i = j;
        } else {
            i = (start + 1).max(j.min(start + 1));
        }
    }
}

const MESES: &[&str] = &[
    "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic",
    "jan", "apr", "aug", "dec",
];

/// Fechas y horas: ISO `2026-06-12`, horas `10:12[:33]`, y `mes día`
/// estilo `ls -l` (`jun  9`). Sólo coloreo, sin acción.
fn find_datetimes(line: &str, out: &mut Vec<Decoration>) {
    let bytes = line.as_bytes();
    let n = bytes.len();
    // ISO: dddd-dd-dd
    let mut i = 0;
    while i + 10 <= n {
        if is_boundary(line, i)
            && bytes[i..i + 4].iter().all(u8::is_ascii_digit)
            && bytes[i + 4] == b'-'
            && bytes[i + 5..i + 7].iter().all(u8::is_ascii_digit)
            && bytes[i + 7] == b'-'
            && bytes[i + 8..i + 10].iter().all(u8::is_ascii_digit)
            && is_end_boundary(line, i + 10)
            && !overlaps_any(i, i + 10, out)
        {
            out.push(Decoration { start: i, end: i + 10, kind: DecorationKind::DateTime });
            i += 10;
        } else {
            i += 1;
        }
    }
    // Hora: d?d:dd(:dd)?
    let mut i = 0;
    while i < n {
        if !is_boundary(line, i) || !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        while j < n && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j - start <= 2 && j + 2 < n && bytes[j] == b':' && bytes[j + 1].is_ascii_digit() && bytes[j + 2].is_ascii_digit() {
            let mut end = j + 3;
            if end + 2 < n
                && bytes[end] == b':'
                && bytes[end + 1].is_ascii_digit()
                && bytes[end + 2].is_ascii_digit()
            {
                end += 3;
            }
            if is_end_boundary(line, end) && !overlaps_any(start, end, out) {
                out.push(Decoration { start, end, kind: DecorationKind::DateTime });
            }
            i = end;
        } else {
            i = j.max(start + 1);
        }
    }
    // `mes día` (ls -l): palabra de 3 letras del set + espacios + 1-2 dígitos.
    let lower = line.to_ascii_lowercase();
    let lb = lower.as_bytes();
    let mut i = 0;
    while i + 3 <= n {
        // Sólo runs ASCII alfabéticos de 3 bytes (los meses lo son); un byte
        // no-ASCII acá sería el medio de un char multibyte — no sliceable.
        if !is_boundary(line, i)
            || !lb[i..i + 3].iter().all(u8::is_ascii_alphabetic)
        {
            i += 1;
            continue;
        }
        let word = &lower[i..i + 3];
        if MESES.contains(&word) && is_end_boundary(line, i + 3) {
            // espacios + día
            let mut j = i + 3;
            while j < n && lb[j] == b' ' {
                j += 1;
            }
            let d0 = j;
            while j < n && lb[j].is_ascii_digit() {
                j += 1;
            }
            let digits = j - d0;
            if (1..=2).contains(&digits)
                && j - i <= 7
                && is_end_boundary(line, j)
                && !overlaps_any(i, j, out)
            {
                out.push(Decoration { start: i, end: j, kind: DecorationKind::DateTime });
                i = j;
                continue;
            }
        }
        i += 1;
    }
}

/// Porcentajes: `85%`, `99.7%`.
fn find_percents(line: &str, out: &mut Vec<Decoration>) {
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if !is_boundary(line, i) || !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        while j < n && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
            j += 1;
        }
        if j < n && bytes[j] == b'%' && !overlaps_any(start, j + 1, out) {
            out.push(Decoration { start, end: j + 1, kind: DecorationKind::Percent });
            i = j + 1;
        } else {
            i = j.max(start + 1);
        }
    }
}

/// Palabras de estado (case-insensitive) + glifos ✔/✓/✖/✗/⚠.
fn find_severities(line: &str, out: &mut Vec<Decoration>) {
    const ERR: &[&str] = &[
        "error", "err", "failed", "failure", "fail", "fatal", "panic", "denied", "abort",
        "aborted", "rechazado", "fallo", "falló",
    ];
    const WARN: &[&str] = &["warning", "warn", "aviso", "deprecated", "stale"];
    const OK: &[&str] = &[
        "ok", "done", "success", "succeeded", "passed", "ready", "finished", "listo", "hecho",
    ];
    let lower = line.to_ascii_lowercase();
    // Palabras: escaneo por tokens alfabéticos.
    let lb = lower.as_bytes();
    let n = lb.len();
    let mut i = 0;
    while i < n {
        if !lb[i].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        while j < n && lb[j].is_ascii_alphabetic() {
            j += 1;
        }
        let word = &lower[start..j];
        let sev = if ERR.contains(&word) {
            Some(Severity::Error)
        } else if WARN.contains(&word) {
            Some(Severity::Warn)
        } else if OK.contains(&word) {
            Some(Severity::Ok)
        } else {
            None
        };
        if let Some(sev) = sev {
            if is_boundary(line, start) && is_end_boundary(line, j) && !overlaps_any(start, j, out)
            {
                out.push(Decoration { start, end: j, kind: DecorationKind::Severity(sev) });
            }
        }
        i = j;
    }
    // Glifos sueltos.
    for (idx, c) in line.char_indices() {
        let sev = match c {
            '✔' | '✓' => Severity::Ok,
            '✖' | '✗' => Severity::Error,
            '⚠' => Severity::Warn,
            _ => continue,
        };
        let end = idx + c.len_utf8();
        if !overlaps_any(idx, end, out) {
            out.push(Decoration { start: idx, end, kind: DecorationKind::Severity(sev) });
        }
    }
}

/// Números sueltos (enteros o decimales), con sufijo de unidad corto
/// pegado (`248`, `4096`, `1.3M`, `512KB`, `350ms`). El finder más
/// genérico: va último y respeta todo lo ya reclamado.
fn find_numbers(line: &str, out: &mut Vec<Decoration>) {
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if !is_boundary(line, i) || !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        let mut punto = false;
        while j < n {
            let c = bytes[j];
            if c.is_ascii_digit() {
                j += 1;
            } else if c == b'.' && !punto && j + 1 < n && bytes[j + 1].is_ascii_digit() {
                punto = true;
                j += 1;
            } else {
                break;
            }
        }
        // Sufijo de unidad pegado, hasta 3 letras (K, MB, GiB, ms, s).
        let mut end = j;
        let mut letras = 0;
        while end < n && letras < 3 && bytes[end].is_ascii_alphabetic() {
            end += 1;
            letras += 1;
        }
        if end < n && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end = j; // sufijo demasiado largo → no era unidad; sólo el número
        }
        if is_end_boundary(line, end) && !overlaps_any(start, end, out) {
            out.push(Decoration { start, end, kind: DecorationKind::Number });
        }
        i = end.max(start + 1);
    }
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

    #[test]
    fn box_drawing_chars_are_detected_as_one_run() {
        // Tres chars contiguos U+2500..U+257F = una sola decoración.
        let line = "┌───┐ texto │ otro";
        let dec = decorate_line(line, Path::new("/"));
        let boxes: Vec<_> = dec
            .iter()
            .filter(|d| matches!(d.kind, DecorationKind::BoxDraw))
            .collect();
        // Una para "┌───┐" y otra para "│".
        assert_eq!(boxes.len(), 2);
    }

    #[test]
    fn box_drawing_runs_are_contiguous_only() {
        // Espacios cortan el run.
        let line = "─ ─";
        let dec = decorate_line(line, Path::new("/"));
        let n_boxes = dec.iter().filter(|d| matches!(d.kind, DecorationKind::BoxDraw)).count();
        assert_eq!(n_boxes, 2);
    }

    #[test]
    fn block_elements_count_as_box_draw() {
        // Bloques de progresión (cargo "███") también.
        let line = "▓▓░ progreso";
        let dec = decorate_line(line, Path::new("/"));
        assert!(dec.iter().any(|d| matches!(d.kind, DecorationKind::BoxDraw)));
    }

    #[test]
    fn git_sha_is_detected_with_hex_and_min_length() {
        let line = "commit a1b2c3d  por sergio";
        let dec = decorate_line(line, Path::new("/"));
        let shas: Vec<_> = dec
            .iter()
            .filter_map(|d| match &d.kind {
                DecorationKind::GitSha(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(shas, vec!["a1b2c3d"]);
    }

    #[test]
    fn git_sha_long_form() {
        let line = "ref a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let dec = decorate_line(line, Path::new("/"));
        let count = dec
            .iter()
            .filter(|d| matches!(d.kind, DecorationKind::GitSha(_)))
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn pure_letters_are_not_a_sha() {
        // "abcdefg" es hex pero sin dígito — descarta.
        let line = "abcdefg";
        let dec = decorate_line(line, Path::new("/"));
        assert!(!dec.iter().any(|d| matches!(d.kind, DecorationKind::GitSha(_))));
    }

    #[test]
    fn issue_ref_is_picked_up() {
        let line = "fixes #1234 y también #56";
        let dec = decorate_line(line, Path::new("/"));
        let refs: Vec<_> = dec
            .iter()
            .filter_map(|d| match &d.kind {
                DecorationKind::IssueRef(n) => Some(*n),
                _ => None,
            })
            .collect();
        assert_eq!(refs, vec![1234, 56]);
    }

    #[test]
    fn pound_inside_a_word_is_not_an_issue_ref() {
        let line = "abc#123";
        let dec = decorate_line(line, Path::new("/"));
        assert!(!dec.iter().any(|d| matches!(d.kind, DecorationKind::IssueRef(_))));
    }
}
