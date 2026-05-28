//! Validación contra GnuCOBOL: compila el fuente con `cobc`, lo ejecuta,
//! y diffea su salida contra la del intérprete sombra. Es el «otro lado»
//! de [`super::interpret`] — la garantía de que la sombra produce lo
//! mismo que un compilador COBOL real.
//!
//! Si `cobc` no está en el `PATH`, las funciones de este módulo
//! devuelven `None` o un error explícito en vez de fallar — los tests
//! pueden filtrar por disponibilidad.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::{run_source, Outcome, ShadowError};

/// Tope para evitar que un binario COBOL en bucle infinito cuelgue el
/// test runner. Si `cobc` se está usando, también es bueno limitarse.
const RUN_TIMEOUT_SECONDS: u64 = 20;

/// El resultado de comparar la salida del intérprete sombra con la del
/// binario compilado por GnuCOBOL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CobcReport {
    /// Líneas que produjo el intérprete sombra.
    pub shadow: Vec<String>,
    /// Líneas que produjo el binario `cobc`.
    pub cobc: Vec<String>,
}

impl CobcReport {
    /// ¿Coinciden ambas salidas (ignorando espacios finales por línea)?
    pub fn matches(&self) -> bool {
        let trim: fn(&String) -> &str = |l| l.trim_end();
        self.shadow.iter().map(trim).collect::<Vec<_>>()
            == self.cobc.iter().map(trim).collect::<Vec<_>>()
    }
}

/// Falla del harness `cobc`.
#[derive(Debug, thiserror::Error)]
pub enum CobcError {
    #[error("`cobc` no está disponible en el PATH")]
    NotAvailable,
    #[error("error del pipeline sombra: {0}")]
    Shadow(#[from] ShadowError),
    #[error("error de IO al invocar cobc: {0}")]
    Io(#[from] std::io::Error),
    #[error("cobc {phase} falló (status {status}): {stderr}")]
    Compile {
        phase: &'static str,
        status: i32,
        stderr: String,
    },
}

/// ¿Está `cobc` disponible en el `PATH`?
pub fn is_available() -> bool {
    Command::new("cobc")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Compila `source` con `cobc` y ejecuta el binario, capturando stdout.
/// El binario se genera en un directorio temporal y se borra al salir.
pub fn run_with_cobc(source: &str) -> Result<Vec<String>, CobcError> {
    if !is_available() {
        return Err(CobcError::NotAvailable);
    }
    let dir = tempdir()?;
    let src_path = dir.path.join("program.cob");
    let bin_path = dir.path.join("program");

    std::fs::write(&src_path, source)?;
    let compile = Command::new("cobc")
        .arg("-x")
        .arg("-free")
        .arg("-o")
        .arg(&bin_path)
        .arg(&src_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    if !compile.status.success() {
        return Err(CobcError::Compile {
            phase: "compile",
            status: compile.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&compile.stderr).to_string(),
        });
    }

    let run = run_with_timeout(Command::new(&bin_path))?;
    if !run.status.success() {
        return Err(CobcError::Compile {
            phase: "run",
            status: run.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&run.stderr).to_string(),
        });
    }
    let stdout = String::from_utf8_lossy(&run.stdout)
        .lines()
        .map(|l| l.to_string())
        .collect();
    Ok(stdout)
}

/// Corre `source` por el intérprete sombra y por `cobc`, y devuelve un
/// reporte comparando ambas salidas.
pub fn compare_with_cobc(source: &str) -> Result<CobcReport, CobcError> {
    let shadow: Outcome = run_source(source)?;
    let cobc = run_with_cobc(source)?;
    Ok(CobcReport {
        shadow: shadow.lines,
        cobc,
    })
}

/// Ejecuta un `Command` con un timeout duro. Si vence, lo mata y
/// devuelve un error de IO.
fn run_with_timeout(mut cmd: Command) -> std::io::Result<std::process::Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            if let Some(mut o) = child.stdout.take() {
                std::io::Read::read_to_end(&mut o, &mut stdout)?;
            }
            if let Some(mut e) = child.stderr.take() {
                std::io::Read::read_to_end(&mut e, &mut stderr)?;
            }
            return Ok(std::process::Output {
                status,
                stdout,
                stderr,
            });
        }
        if start.elapsed().as_secs() >= RUN_TIMEOUT_SECONDS {
            let _ = child.kill();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "binario COBOL excedió el timeout",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Directorio temporal mínimo (autoeliminado en `Drop`) sin sumar la
/// dependencia `tempfile` al árbol del workspace.
struct TempDir {
    path: PathBuf,
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn tempdir() -> std::io::Result<TempDir> {
    let mut path = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    path.push(format!("chaka-cobc-{pid}-{nanos}"));
    std::fs::create_dir(&path)?;
    Ok(TempDir { path })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requiere GnuCOBOL (`cobc`) en el PATH"]
    fn cobc_and_shadow_agree_on_hello() {
        let source = include_str!("../../corpus/01-hola.cob");
        let report = compare_with_cobc(source).expect("comparación con cobc");
        assert!(report.matches(), "shadow={:?} cobc={:?}", report.shadow, report.cobc);
    }

    #[test]
    #[ignore = "requiere GnuCOBOL (`cobc`) en el PATH"]
    fn cobc_and_shadow_agree_on_arithmetic() {
        let source = include_str!("../../corpus/02-aritmetica.cob");
        let report = compare_with_cobc(source).expect("comparación con cobc");
        assert!(report.matches(), "shadow={:?} cobc={:?}", report.shadow, report.cobc);
    }

    #[test]
    fn not_available_when_cobc_missing() {
        // En este entorno cobc no está instalado: comprobamos el camino
        // de error explícito sin depender de un cobc real.
        if is_available() {
            return; // si lo está, no podemos validar el otro lado aquí.
        }
        let err = run_with_cobc("PROCEDURE DIVISION.\nMAIN.\n STOP RUN.\n").unwrap_err();
        assert!(matches!(err, CobcError::NotAvailable));
    }
}
