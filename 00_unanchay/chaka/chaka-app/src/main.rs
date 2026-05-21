//! `charka` — la CLI del transpilador COBOL → Rust.
//!
//! Envuelve el pipeline (lexer → parser → IR → codegen) y el validador
//! en sombra en cuatro comandos:
//!
//! - `transpile` — emite el código Rust de un fuente COBOL.
//! - `scaffold`  — genera un crate Rust completo y compilable.
//! - `run`       — ejecuta el programa (intérprete sombra) y lo imprime.
//! - `check`     — ejecuta y compara la salida contra un archivo dado.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use charka_ir::{Ir, PerformTarget, Stmt};
use clap::{Parser, Subcommand};

/// Ruta a `charka-runtime`, fijada al compilar — el crate generado por
/// `scaffold` la usa como dependencia.
const RUNTIME_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../modules/charka/charka-runtime"
);

/// El transpilador de COBOL a Rust.
#[derive(Parser)]
#[command(name = "charka", version, about = "Transpilador COBOL → Rust")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Transpila un fuente COBOL a código Rust.
    Transpile {
        /// El fuente COBOL (.cob), en formato libre.
        input: PathBuf,
        /// Archivo de salida; si se omite, va a la salida estándar.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Genera un crate Rust completo y compilable.
    Scaffold {
        /// El fuente COBOL (.cob).
        input: PathBuf,
        /// El directorio del crate a crear.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Ejecuta un programa COBOL (intérprete sombra) y muestra su salida.
    Run {
        /// El fuente COBOL (.cob).
        input: PathBuf,
    },
    /// Ejecuta un programa y compara su salida con un archivo esperado.
    Check {
        /// El fuente COBOL (.cob).
        input: PathBuf,
        /// El archivo con la salida esperada.
        #[arg(short, long)]
        expect: PathBuf,
    },
}

fn main() -> ExitCode {
    match dispatch(Cli::parse().command) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("charka: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(command: Command) -> Result<ExitCode> {
    match command {
        Command::Transpile { input, output } => transpile(&input, output.as_deref()),
        Command::Scaffold { input, output } => scaffold(&input, &output),
        Command::Run { input } => run(&input),
        Command::Check { input, expect } => check(&input, &expect),
    }
}

// ── Comandos ──────────────────────────────────────────────────────

fn transpile(input: &Path, output: Option<&Path>) -> Result<ExitCode> {
    let rust = charka_codegen::generate(&load_ir(input)?);
    match output {
        Some(path) => {
            fs::write(path, rust)
                .with_context(|| format!("no se pudo escribir {}", path.display()))?;
            eprintln!("charka: escrito {}", path.display());
        }
        None => print!("{rust}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn scaffold(input: &Path, output: &Path) -> Result<ExitCode> {
    let ir = load_ir(input)?;
    let rust = charka_codegen::generate(&ir);
    let name = crate_name(input);

    fs::create_dir_all(output.join("src"))
        .with_context(|| format!("no se pudo crear {}", output.display()))?;
    fs::write(output.join("src/main.rs"), rust)?;
    fs::write(output.join("Cargo.toml"), cargo_toml(&name))?;

    eprintln!("charka: crate «{name}» generado en {}", output.display());
    eprintln!(
        "  cargo run --manifest-path {}",
        output.join("Cargo.toml").display()
    );
    warn_unknowns(&ir);
    Ok(ExitCode::SUCCESS)
}

fn run(input: &Path) -> Result<ExitCode> {
    let ir = load_ir(input)?;
    let outcome = charka_shadow::interpret(&ir);
    for line in &outcome.lines {
        println!("{line}");
    }
    warn_unknowns(&ir);
    if outcome.halt == charka_shadow::Halt::StepLimit {
        eprintln!("charka: aviso — se agotó el tope de pasos (¿un bucle sin fin?)");
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

fn check(input: &Path, expect: &Path) -> Result<ExitCode> {
    let ir = load_ir(input)?;
    let outcome = charka_shadow::interpret(&ir);
    let expected = fs::read_to_string(expect)
        .with_context(|| format!("no se pudo leer {}", expect.display()))?;

    let got: Vec<&str> = outcome.lines.iter().map(|l| l.trim_end()).collect();
    let want: Vec<&str> = expected.lines().map(|l| l.trim_end()).collect();

    if got == want {
        println!("charka: OK — {} líneas coinciden", got.len());
        Ok(ExitCode::SUCCESS)
    } else {
        eprintln!("charka: FALLA — la salida difiere de {}", expect.display());
        report_diff(&got, &want);
        Ok(ExitCode::FAILURE)
    }
}

// ── Apoyo ─────────────────────────────────────────────────────────

/// Lee un fuente COBOL y lo lleva hasta el IR.
fn load_ir(input: &Path) -> Result<Ir> {
    let source = fs::read_to_string(input)
        .with_context(|| format!("no se pudo leer {}", input.display()))?;
    let tokens =
        charka_lexer::lex(&source, charka_lexer::SourceFormat::Free).context("error de léxico")?;
    let program = charka_parser::parse(&tokens).context("error de parseo")?;
    Ok(charka_ir::lower(&program))
}

/// El `Cargo.toml` de un crate generado por `scaffold`.
fn cargo_toml(name: &str) -> String {
    format!(
        "[package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         \n\
         [[bin]]\n\
         name = \"{name}\"\n\
         path = \"src/main.rs\"\n\
         \n\
         [dependencies]\n\
         charka-runtime = {{ path = \"{RUNTIME_PATH}\" }}\n\
         \n\
         [workspace]\n"
    )
}

/// Un nombre de crate válido derivado del nombre del archivo fuente.
fn crate_name(input: &Path) -> String {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("programa");
    let mut name: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        name = format!("cobol_{name}");
    }
    name
}

/// Avisa de los verbos COBOL que el transpilador no soporta todavía.
fn warn_unknowns(ir: &Ir) {
    let mut verbs = Vec::new();
    for proc in &ir.procedures {
        collect_unknowns(&proc.body, &mut verbs);
    }
    if verbs.is_empty() {
        return;
    }
    verbs.sort();
    verbs.dedup();
    eprintln!(
        "charka: aviso — verbos no transpilados (se omitieron): {}",
        verbs.join(", ")
    );
}

/// Recoge los verbos de los `Stmt::Unknown`, incluso los anidados.
fn collect_unknowns(stmts: &[Stmt], out: &mut Vec<String>) {
    for s in stmts {
        match s {
            Stmt::Unknown { verb, .. } => out.push(verb.clone()),
            Stmt::If {
                then_branch,
                else_branch,
                ..
            } => {
                collect_unknowns(then_branch, out);
                collect_unknowns(else_branch, out);
            }
            Stmt::Evaluate { whens, other, .. } => {
                for w in whens {
                    collect_unknowns(&w.body, out);
                }
                collect_unknowns(other, out);
            }
            Stmt::Read {
                at_end, not_at_end, ..
            } => {
                collect_unknowns(at_end, out);
                collect_unknowns(not_at_end, out);
            }
            Stmt::Perform(p) => {
                if let PerformTarget::Inline(body) = &p.target {
                    collect_unknowns(body, out);
                }
            }
            _ => {}
        }
    }
}

/// Imprime las líneas en que la salida obtenida difiere de la esperada.
fn report_diff(got: &[&str], want: &[&str]) {
    for i in 0..got.len().max(want.len()) {
        let g = got.get(i).copied().unwrap_or("<falta>");
        let w = want.get(i).copied().unwrap_or("<falta>");
        if g != w {
            eprintln!("  línea {}:", i + 1);
            eprintln!("    obtenido: {g}");
            eprintln!("    esperado: {w}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ir_of(src: &str) -> Ir {
        let toks = charka_lexer::lex(src, charka_lexer::SourceFormat::Free).unwrap();
        charka_ir::lower(&charka_parser::parse(&toks).unwrap())
    }

    #[test]
    fn crate_name_is_sanitized() {
        assert_eq!(crate_name(Path::new("/x/06-nomina.cob")), "cobol_06_nomina");
        assert_eq!(crate_name(Path::new("PAYROLL.CBL")), "payroll");
    }

    #[test]
    fn cargo_toml_names_the_crate_and_the_runtime() {
        let toml = cargo_toml("demo");
        assert!(toml.contains("name = \"demo\""));
        assert!(toml.contains("charka-runtime"));
        assert!(toml.contains("[workspace]"));
    }

    #[test]
    fn unknown_verbs_are_collected() {
        let ir = ir_of(
            "PROCEDURE DIVISION.\n\
             MAIN.\n\
                 CALL 'SUBPROG'.\n",
        );
        let mut verbs = Vec::new();
        for proc in &ir.procedures {
            collect_unknowns(&proc.body, &mut verbs);
        }
        assert_eq!(verbs, vec!["CALL".to_string()]);
    }

    #[test]
    fn known_program_has_no_unknowns() {
        let ir = ir_of("PROCEDURE DIVISION.\nMAIN.\n DISPLAY 'OK'.\n STOP RUN.\n");
        let mut verbs = Vec::new();
        for proc in &ir.procedures {
            collect_unknowns(&proc.body, &mut verbs);
        }
        assert!(verbs.is_empty());
    }
}
