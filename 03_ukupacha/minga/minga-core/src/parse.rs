//! Adaptadores de parsing por dialecto.
//!
//! Cada función devuelve un [`SemanticNode`] normalizado a partir del
//! source code. La normalización vive en `ast::SemanticNode::from_tree_sitter`
//! y es agnóstica al lenguaje — cualquier tree-sitter grammar produce
//! el mismo shape de árbol semántico (sin whitespace, sin comentarios).
//!
//! Lenguajes soportados (cada uno son ~6 LOC + dep tree-sitter-X):
//! - [`rust`] — Rust completo (con α-hashing en `alpha::hash_node_alpha`).
//! - [`python`] — Python 3.x.
//! - [`typescript`] — TypeScript (no TSX).
//! - [`javascript`] — JavaScript / ECMAScript.
//! - [`go`] — Go.
//!
//! Para hashing α-equivalente, sólo Rust tiene implementación dedicada
//! hoy. Otros lenguajes caen al [`crate::cas::hash_node`] estructural,
//! que es α-NO-equivalente: dos versiones del mismo término que
//! difieren en nombres de variables ligadas tendrán hashes distintos.
//! Suficiente para detección de cambios; no para detección de
//! equivalencia semántica.
//!
//! ## Auto-detección por extensión
//!
//! [`detect_by_extension`] mapea `.rs` → Rust, `.py` → Python, etc.
//! Útil para `minga ingest` cuando el caller no quiere especificar
//! el dialecto a mano.

use crate::ast::SemanticNode;
use thiserror::Error;
use tree_sitter::{Language, Parser};

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("tree-sitter no pudo configurar el lenguaje")]
    Language,
    #[error("tree-sitter no produjo árbol para la entrada")]
    NoTree,
}

/// Identificadores estables de cada dialecto soportado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Go,
}

impl Dialect {
    /// Nombre canónico para logging / display.
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Rust => "rust",
            Dialect::Python => "python",
            Dialect::TypeScript => "typescript",
            Dialect::JavaScript => "javascript",
            Dialect::Go => "go",
        }
    }

    /// Byte estable para persistir/transmitir el dialecto. Numérico para
    /// no depender del orden de la enum si se agregan lenguajes.
    pub fn as_byte(self) -> u8 {
        match self {
            Dialect::Rust => 1,
            Dialect::Python => 2,
            Dialect::TypeScript => 3,
            Dialect::JavaScript => 4,
            Dialect::Go => 5,
        }
    }

    /// Inversa de [`Dialect::as_byte`]. `None` si el byte no corresponde
    /// a un dialecto conocido por esta versión.
    pub fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            1 => Dialect::Rust,
            2 => Dialect::Python,
            3 => Dialect::TypeScript,
            4 => Dialect::JavaScript,
            5 => Dialect::Go,
            _ => return None,
        })
    }

    /// Parsea `source` con la gramática de este dialecto.
    pub fn parse(self, source: &str) -> Result<SemanticNode, ParseError> {
        match self {
            Dialect::Rust => rust(source),
            Dialect::Python => python(source),
            Dialect::TypeScript => typescript(source),
            Dialect::JavaScript => javascript(source),
            Dialect::Go => go(source),
        }
    }
}

/// Mapea una extensión de archivo (sin el `.`) al dialecto correspondiente.
/// `None` si la extensión no corresponde a un lenguaje soportado.
///
/// ```
/// use minga_core::parse::{detect_by_extension, Dialect};
/// assert_eq!(detect_by_extension("rs"), Some(Dialect::Rust));
/// assert_eq!(detect_by_extension("py"), Some(Dialect::Python));
/// assert_eq!(detect_by_extension("unknown"), None);
/// ```
pub fn detect_by_extension(ext: &str) -> Option<Dialect> {
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Some(Dialect::Rust),
        "py" | "pyi" => Some(Dialect::Python),
        "ts" => Some(Dialect::TypeScript),
        "js" | "mjs" | "cjs" => Some(Dialect::JavaScript),
        "go" => Some(Dialect::Go),
        _ => None,
    }
}

/// Detecta el dialecto leyendo la primera línea como shebang. Reconoce
/// las formas habituales:
/// - `#!/usr/bin/env python3` → Python
/// - `#!/usr/bin/python3.11` → Python
/// - `#!/usr/bin/env node` / `deno` → JavaScript
/// - `#!/usr/bin/env -S deno run --ext=ts` / `tsx` → TypeScript
/// - `#!/usr/bin/env bash` / `sh` → `None` (no soportado)
///
/// Sólo mira la **primera línea**: si no comienza por `#!`, devuelve
/// `None` sin tocar el resto del buffer.
pub fn detect_by_shebang(source: &str) -> Option<Dialect> {
    let first = source.lines().next()?;
    let rest = first.strip_prefix("#!")?.trim();
    let interpreter = last_token(rest);
    let lower = interpreter.to_ascii_lowercase();
    let trimmed = lower.trim_start_matches(|c: char| c == '/' || c.is_ascii_alphanumeric() == false);
    let last_segment = lower.rsplit('/').next().unwrap_or(&lower);
    // Coincidencia laxa por sufijo: cubre versiones como `python3.11`.
    let cand = if last_segment.starts_with("python") {
        Some(Dialect::Python)
    } else if last_segment == "node" || last_segment == "deno" || last_segment == "bun" {
        // Por defecto JS; el ext flag se evalúa abajo.
        Some(Dialect::JavaScript)
    } else if last_segment == "tsx" || last_segment == "ts-node" {
        Some(Dialect::TypeScript)
    } else if last_segment.ends_with("rustc") {
        Some(Dialect::Rust)
    } else if last_segment == "go" {
        Some(Dialect::Go)
    } else {
        None
    };
    // Override por `--ext=ts` en la cadena (env -S deno run --ext=ts).
    if rest.contains("--ext=ts") || rest.contains("--ext ts") {
        return Some(Dialect::TypeScript);
    }
    let _ = trimmed; // suppress unused
    cand
}

/// Detecta el dialecto **por contenido**. Combina dos señales:
///
/// 1. **Marcadores textuales** distintivos por lenguaje (palabras
///    clave en posición de declaración: `def `/`class ` para Python,
///    `fn `/`impl `/`pub ` para Rust, `func `/`package ` para Go,
///    `function `/`interface `/anotaciones de tipo TS para JS/TS).
///    Tree-sitter es muy permisivo — acepta casi cualquier cosa con
///    pocos `ERROR`, así que no se puede confiar sólo en eso.
/// 2. **Ratio de nodos `ERROR`** al parsear con la gramática candidata.
///    Sirve como tie-break: si dos lenguajes empatan en marcadores,
///    gana el que produce el AST más limpio.
///
/// Si ningún candidato consigue un parse limpio (≤ 5 % errores) o
/// ningún marcador textual identifica un lenguaje, devuelve `None`.
pub fn detect_by_content(source: &str) -> Option<Dialect> {
    if source.trim().is_empty() {
        return None;
    }
    // Marcadores textuales por lenguaje. Cada lenguaje suma puntos
    // según cuántos marcadores aparecen en el source.
    let mut scores: [(Dialect, u32); 5] = [
        (Dialect::Rust, 0),
        (Dialect::Python, 0),
        (Dialect::TypeScript, 0),
        (Dialect::JavaScript, 0),
        (Dialect::Go, 0),
    ];
    for line in source.lines() {
        let t = line.trim_start();
        // Rust: `fn `/`pub fn `/`impl `/`use `/`mod `/`let `.
        if t.starts_with("fn ")
            || t.starts_with("pub fn ")
            || t.starts_with("pub(crate) fn ")
            || t.starts_with("impl ")
            || t.starts_with("use ")
            || t.starts_with("mod ")
            || t.starts_with("let ")
            || t.starts_with("struct ")
            || t.starts_with("enum ")
            || t.starts_with("trait ")
        {
            scores[0].1 += 1;
        }
        // Python: `def `/`class `/`import `/`from `/`elif `.
        if t.starts_with("def ")
            || t.starts_with("class ")
            || t.starts_with("import ")
            || t.starts_with("from ")
            || t.starts_with("elif ")
            || t.starts_with("async def ")
        {
            scores[1].1 += 1;
        }
        // TypeScript: anotaciones `: \w+`, `interface `, `type `,
        // `enum `. JS no usa la mayoría de éstos.
        if t.starts_with("interface ")
            || t.starts_with("type ")
            || t.starts_with("enum ")
        {
            scores[2].1 += 1;
        }
        // JavaScript/TypeScript ambos.
        if t.starts_with("function ")
            || t.starts_with("const ")
            || t.starts_with("let ")
            || t.starts_with("var ")
            || t.starts_with("export ")
            || t.starts_with("import {")
        {
            // `let ` también lo usa Rust — dejamos que el score Rust
            // gane si hay otros indicadores; aquí sólo sumamos a JS
            // si no parece Rust en otros aspectos. Para simplicidad,
            // sumamos a JS y a Rust independientemente y dejamos que
            // el tie-break de errores decida.
            scores[3].1 += 1;
        }
        // Go.
        if t.starts_with("func ")
            || t.starts_with("package ")
            || t.starts_with("import (")
            || t.starts_with("type ") && t.contains(" struct ")
        {
            scores[4].1 += 1;
        }
    }

    // Si nadie tiene marcadores, no podemos decidir.
    let max_score = scores.iter().map(|(_, n)| *n).max().unwrap_or(0);
    if max_score == 0 {
        return None;
    }

    // Top candidatos: los que empatan en el score más alto.
    let top: Vec<Dialect> = scores
        .iter()
        .filter(|(_, n)| *n == max_score)
        .map(|(d, _)| *d)
        .collect();

    // Si hay un único top, devolverlo (sin verificar errores —
    // marcadores robustos ya identifican el lenguaje).
    if top.len() == 1 {
        return Some(top[0]);
    }

    // Empate: parsear con cada candidato y elegir el de menos errores.
    let mut best: Option<(Dialect, f32)> = None;
    for d in top {
        if let Ok(node) = d.parse(source) {
            let (errors, total) = count_errors(&node);
            if total == 0 {
                continue;
            }
            let ratio = errors as f32 / total as f32;
            if best.map_or(true, |(_, r)| ratio < r) {
                best = Some((d, ratio));
            }
        }
    }
    best.and_then(|(d, r)| if r <= 0.05 { Some(d) } else { None })
}

/// Cuenta nodos `ERROR`/`MISSING` y el total de nodos en el subárbol.
/// Tree-sitter marca con `kind == "ERROR"` cualquier sección que la
/// gramática no pudo absorber; `is_named() == false` y un nombre vacío
/// suele indicar tokens missing.
fn count_errors(node: &SemanticNode) -> (usize, usize) {
    let mut errors = 0usize;
    let mut total = 0usize;
    walk_count(node, &mut errors, &mut total);
    (errors, total)
}

fn walk_count(n: &SemanticNode, errors: &mut usize, total: &mut usize) {
    *total += 1;
    if n.kind == "ERROR" {
        *errors += 1;
    }
    for c in &n.children {
        walk_count(c, errors, total);
    }
}

/// Devuelve el último "token" (separado por whitespace) de la cadena.
/// Para shebangs `#!/usr/bin/env python3 -u` queremos el `python3`,
/// pero también `#!/usr/bin/python3.11` (sin `env`) — el truco es:
/// si hay `env`, tomar lo siguiente; si no, el path completo y luego
/// el último segmento. Esta función devuelve el penúltimo token cuando
/// el primero parece un path absoluto a `env`.
fn last_token(s: &str) -> &str {
    let mut tokens = s.split_whitespace().filter(|t| !t.starts_with('-'));
    let first = tokens.next().unwrap_or("");
    if first.ends_with("/env") || first == "env" {
        tokens.next().unwrap_or(first)
    } else {
        first
    }
}

fn parse_with(lang: Language, source: &str) -> Result<SemanticNode, ParseError> {
    let mut parser = Parser::new();
    parser.set_language(&lang).map_err(|_| ParseError::Language)?;
    let tree = parser.parse(source, None).ok_or(ParseError::NoTree)?;
    Ok(SemanticNode::from_tree_sitter(tree.root_node(), source.as_bytes()))
}

pub fn rust(source: &str) -> Result<SemanticNode, ParseError> {
    parse_with(tree_sitter_rust::LANGUAGE.into(), source)
}

pub fn python(source: &str) -> Result<SemanticNode, ParseError> {
    parse_with(tree_sitter_python::LANGUAGE.into(), source)
}

pub fn typescript(source: &str) -> Result<SemanticNode, ParseError> {
    parse_with(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), source)
}

pub fn javascript(source: &str) -> Result<SemanticNode, ParseError> {
    parse_with(tree_sitter_javascript::LANGUAGE.into(), source)
}

pub fn go(source: &str) -> Result<SemanticNode, ParseError> {
    parse_with(tree_sitter_go::LANGUAGE.into(), source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_parses(d: Dialect, source: &str) -> SemanticNode {
        let node = d.parse(source).expect("parse should succeed");
        // Sanity: el root siempre tiene al menos un child para code real.
        assert!(
            !node.children.is_empty(),
            "{}: root node sin children — parse posiblemente vacío",
            d.name()
        );
        node
    }

    #[test]
    fn rust_parses_basic() {
        assert_parses(Dialect::Rust, "fn add(a: i32, b: i32) -> i32 { a + b }");
    }

    #[test]
    fn python_parses_basic() {
        assert_parses(
            Dialect::Python,
            "def add(a: int, b: int) -> int:\n    return a + b\n",
        );
    }

    #[test]
    fn typescript_parses_basic() {
        assert_parses(
            Dialect::TypeScript,
            "function add(a: number, b: number): number { return a + b; }",
        );
    }

    #[test]
    fn javascript_parses_basic() {
        assert_parses(
            Dialect::JavaScript,
            "function add(a, b) { return a + b; }",
        );
    }

    #[test]
    fn go_parses_basic() {
        assert_parses(
            Dialect::Go,
            "package main\n\nfunc add(a, b int) int {\n    return a + b\n}\n",
        );
    }

    #[test]
    fn detect_extension_canonical() {
        assert_eq!(detect_by_extension("rs"), Some(Dialect::Rust));
        assert_eq!(detect_by_extension("py"), Some(Dialect::Python));
        assert_eq!(detect_by_extension("pyi"), Some(Dialect::Python));
        assert_eq!(detect_by_extension("ts"), Some(Dialect::TypeScript));
        assert_eq!(detect_by_extension("js"), Some(Dialect::JavaScript));
        assert_eq!(detect_by_extension("mjs"), Some(Dialect::JavaScript));
        assert_eq!(detect_by_extension("cjs"), Some(Dialect::JavaScript));
        assert_eq!(detect_by_extension("go"), Some(Dialect::Go));
        assert_eq!(detect_by_extension("unknown"), None);
        assert_eq!(detect_by_extension(""), None);
    }

    #[test]
    fn detect_shebang_python_env() {
        assert_eq!(
            detect_by_shebang("#!/usr/bin/env python3\nprint(1)\n"),
            Some(Dialect::Python)
        );
    }

    #[test]
    fn detect_shebang_python_direct() {
        assert_eq!(
            detect_by_shebang("#!/usr/bin/python3.11\n"),
            Some(Dialect::Python)
        );
    }

    #[test]
    fn detect_shebang_node() {
        assert_eq!(
            detect_by_shebang("#!/usr/bin/env node\nconsole.log(1)\n"),
            Some(Dialect::JavaScript)
        );
    }

    #[test]
    fn detect_shebang_deno_with_ext_ts() {
        assert_eq!(
            detect_by_shebang("#!/usr/bin/env -S deno run --ext=ts\n"),
            Some(Dialect::TypeScript)
        );
    }

    #[test]
    fn detect_content_rust_clean() {
        assert_eq!(
            detect_by_content("fn main() { let x = 1; println!(\"{}\", x); }"),
            Some(Dialect::Rust)
        );
    }

    #[test]
    fn detect_content_python_clean() {
        assert_eq!(
            detect_by_content("def add(a, b):\n    return a + b\n"),
            Some(Dialect::Python)
        );
    }

    #[test]
    fn detect_content_go_clean() {
        assert_eq!(
            detect_by_content("package main\n\nfunc add(a, b int) int { return a + b }\n"),
            Some(Dialect::Go)
        );
    }

    #[test]
    fn detect_content_returns_none_for_garbage() {
        // Texto que ninguna gramática absorbe sin un montón de ERRORs.
        let garbage = "++++ @@@@ ###% %%%% [[[[[[[[[ ............";
        assert_eq!(detect_by_content(garbage), None);
    }

    #[test]
    fn detect_content_handles_empty() {
        assert_eq!(detect_by_content(""), None);
        assert_eq!(detect_by_content("   \n\t  "), None);
    }

    #[test]
    fn detect_shebang_no_match_for_bash() {
        assert_eq!(detect_by_shebang("#!/bin/bash\necho hola\n"), None);
    }

    #[test]
    fn detect_shebang_requires_hashbang() {
        // No es shebang — empieza con `//` (comentario JS), no debe matchear.
        assert_eq!(detect_by_shebang("// nope\n"), None);
    }

    #[test]
    fn detect_extension_case_insensitive() {
        assert_eq!(detect_by_extension("RS"), Some(Dialect::Rust));
        assert_eq!(detect_by_extension("Py"), Some(Dialect::Python));
        assert_eq!(detect_by_extension("TS"), Some(Dialect::TypeScript));
    }

    #[test]
    fn dialect_name_canonical() {
        assert_eq!(Dialect::Rust.name(), "rust");
        assert_eq!(Dialect::Python.name(), "python");
        assert_eq!(Dialect::TypeScript.name(), "typescript");
        assert_eq!(Dialect::JavaScript.name(), "javascript");
        assert_eq!(Dialect::Go.name(), "go");
    }

    #[test]
    fn structural_hash_distinguishes_languages() {
        // Mismo "shape" textual pero distintos lenguajes producen
        // árboles distintos (las gramáticas no coinciden) y por tanto
        // hashes estructurales distintos. Importante para evitar
        // colisiones en el CAS cuando el mismo source se ingiere
        // bajo dialectos distintos.
        use crate::cas::hash_node;
        let py = Dialect::Python.parse("x = 1").unwrap();
        let js = Dialect::JavaScript.parse("x = 1").unwrap();
        assert_ne!(
            hash_node(&py),
            hash_node(&js),
            "py y js deberían tener hashes distintos para el mismo source"
        );
    }
}
