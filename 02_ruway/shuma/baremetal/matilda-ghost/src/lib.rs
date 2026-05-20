//! `matilda-ghost` — el agente que aplica los pasos en la máquina destino.
//!
//! El «Ghost» es quien realmente ejecuta: recibe los [`ApplyStep`]s que
//! tradujo `matilda-apply` y, en orden, escribe los archivos y corre los
//! comandos en *esta* máquina (la del servidor). Reporta paso a paso en
//! un [`ApplyReport`].
//!
//! Semántica `set -e`: si un paso falla, se detiene — no se aplican los
//! siguientes. [`dry_run`] muestra lo que haría sin tocar nada.
//!
//! La aplicación *remota* (por SSH) la hace `matilda-linker`, que produce
//! el mismo [`ApplyReport`] reusando estos tipos.

#![forbid(unsafe_code)]

use matilda_apply::ApplyStep;
use serde::{Deserialize, Serialize};

/// Resultado de un paso de aplicación.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepResult {
    /// Descripción de la acción aplicada.
    pub describe: String,
    /// `true` si el paso completó sin errores.
    pub ok: bool,
    /// Bitácora legible: archivos escritos, comandos y su salida.
    pub log: Vec<String>,
}

/// El reporte de aplicar un plan: un resultado por paso ejecutado.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApplyReport {
    pub results: Vec<StepResult>,
}

impl ApplyReport {
    /// `true` si todos los pasos ejecutados salieron bien.
    pub fn all_ok(&self) -> bool {
        self.results.iter().all(|r| r.ok)
    }

    /// Cantidad de pasos que salieron bien.
    pub fn applied(&self) -> usize {
        self.results.iter().filter(|r| r.ok).count()
    }

    /// El primer paso que falló, si lo hubo.
    pub fn failed(&self) -> Option<&StepResult> {
        self.results.iter().find(|r| !r.ok)
    }
}

/// Corre un comando de shell, juntando su salida (stdout + stderr).
/// Los comandos de matilda llevan `&&`, redirecciones… → van por `sh -c`.
fn run_command(cmd: &str) -> std::io::Result<(i32, Vec<String>)> {
    let out = std::process::Command::new("sh").arg("-c").arg(cmd).output()?;
    let mut lines = Vec::new();
    for chunk in [&out.stdout, &out.stderr] {
        for l in String::from_utf8_lossy(chunk).lines() {
            lines.push(l.to_string());
        }
    }
    Ok((out.status.code().unwrap_or(-1), lines))
}

/// Aplica un paso en esta máquina: escribe sus archivos y corre sus
/// comandos. Devuelve el resultado; se detiene en el primer error.
fn apply_step(step: &ApplyStep) -> StepResult {
    let mut log = Vec::new();
    let mut ok = true;

    for f in &step.files {
        match std::fs::write(&f.path, &f.content) {
            Ok(()) => log.push(format!("✔ escrito {}", f.path)),
            Err(e) => {
                log.push(format!("✘ no se pudo escribir {}: {e}", f.path));
                ok = false;
                break;
            }
        }
    }

    if ok {
        for cmd in &step.commands {
            log.push(format!("$ {cmd}"));
            match run_command(cmd) {
                Ok((0, out)) => {
                    log.extend(out.into_iter().map(|l| format!("  {l}")));
                }
                Ok((code, out)) => {
                    log.extend(out.into_iter().map(|l| format!("  {l}")));
                    log.push(format!("✘ el comando salió con código {code}"));
                    ok = false;
                    break;
                }
                Err(e) => {
                    log.push(format!("✘ no se pudo ejecutar: {e}"));
                    ok = false;
                    break;
                }
            }
        }
    }

    StepResult { describe: step.describe.clone(), ok, log }
}

/// Aplica los pasos en orden. Se detiene en el primero que falle
/// (semántica `set -e`): los posteriores no se ejecutan.
pub fn apply(steps: &[ApplyStep]) -> ApplyReport {
    let mut results = Vec::new();
    for step in steps {
        let result = apply_step(step);
        let failed = !result.ok;
        results.push(result);
        if failed {
            break;
        }
    }
    ApplyReport { results }
}

/// Simula la aplicación: reporta qué archivos y comandos se ejecutarían,
/// sin tocar nada. Seguro para previsualizar.
pub fn dry_run(steps: &[ApplyStep]) -> ApplyReport {
    let results = steps
        .iter()
        .map(|s| {
            let mut log = Vec::new();
            for f in &s.files {
                log.push(format!("escribiría {} ({} bytes)", f.path, f.content.len()));
            }
            for c in &s.commands {
                log.push(format!("$ {c}"));
            }
            StepResult { describe: s.describe.clone(), ok: true, log }
        })
        .collect();
    ApplyReport { results }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matilda_apply::FileWrite;

    /// Paso que escribe un archivo temporal y corre un comando.
    fn step(describe: &str, file: Option<FileWrite>, cmds: &[&str]) -> ApplyStep {
        ApplyStep {
            describe: describe.into(),
            files: file.into_iter().collect(),
            commands: cmds.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn temp(name: &str) -> String {
        std::env::temp_dir()
            .join(format!("matilda-ghost-{}-{name}", std::process::id()))
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn dry_run_touches_nothing() {
        let path = temp("dry");
        let _ = std::fs::remove_file(&path);
        let steps = vec![step(
            "crear x",
            Some(FileWrite { path: path.clone(), content: "hola".into() }),
            &["echo hecho"],
        )];
        let report = dry_run(&steps);
        assert!(report.all_ok());
        assert_eq!(report.results.len(), 1);
        // dry_run no escribió el archivo.
        assert!(!std::path::Path::new(&path).exists());
    }

    #[test]
    fn apply_writes_files_and_runs_commands() {
        let path = temp("apply");
        let _ = std::fs::remove_file(&path);
        let steps = vec![step(
            "crear config",
            Some(FileWrite { path: path.clone(), content: "contenido".into() }),
            &["echo aplicado"],
        )];
        let report = apply(&steps);
        assert!(report.all_ok());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "contenido");
        assert!(report.results[0].log.iter().any(|l| l.contains("aplicado")));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn apply_stops_at_the_first_failure() {
        let steps = vec![
            step("ok", None, &["true"]),
            step("falla", None, &["exit 7"]),
            step("nunca", None, &["echo no-deberia-correr"]),
        ];
        let report = apply(&steps);
        // El tercer paso no se ejecutó.
        assert_eq!(report.results.len(), 2);
        assert!(!report.all_ok());
        assert_eq!(report.applied(), 1);
        assert!(report.failed().unwrap().describe.contains("falla"));
    }

    #[test]
    fn nonzero_exit_marks_the_step_failed() {
        let report = apply(&[step("test", None, &["false"])]);
        assert!(!report.results[0].ok);
    }

    #[test]
    fn empty_plan_applies_cleanly() {
        let report = apply(&[]);
        assert!(report.all_ok());
        assert_eq!(report.applied(), 0);
    }
}
