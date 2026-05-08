use std::path::Path;
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum KclError {
    #[error("kcl binary not found on PATH (install: https://kcl-lang.io)")]
    BinaryMissing,
    #[error("kcl validation failed:\n{0}")]
    ValidationFailed(String),
    #[error("io invoking kcl: {0}")]
    Io(#[from] std::io::Error),
}

/// Validate `state_path` (json) against a schema defined in `schema_path` (.k),
/// targeting the named schema.
pub fn vet(schema_path: &Path, state_path: &Path, schema_name: &str) -> Result<(), KclError> {
    let out = match Command::new("kcl")
        .arg("vet")
        .arg(state_path)
        .arg(schema_path)
        .arg("-s")
        .arg(schema_name)
        .output()
    {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(KclError::BinaryMissing),
        Err(e) => return Err(e.into()),
    };

    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let msg = if stderr.trim().is_empty() {
            stdout
        } else {
            stderr
        };
        Err(KclError::ValidationFailed(msg))
    }
}
