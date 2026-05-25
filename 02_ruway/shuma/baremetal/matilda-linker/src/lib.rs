//! `matilda-linker` — el enlace SSH que aplica un plan en un servidor.
//!
//! El [`Linker`] conecta a un host vía `brahman-ssh-multiplex` y aplica
//! los [`ApplyStep`]s **remotamente**: escribe los archivos (con un
//! heredoc) y corre los comandos, cada uno sobre la conexión SSH
//! multiplexada. Produce el mismo [`ApplyReport`] que `matilda-ghost`,
//! así el consumidor no distingue aplicación local de remota.
//!
//! La prueba real necesita un servidor SSH — se hace fuera del unit
//! test. Lo puro y testeable es la construcción del comando de escritura.

#![forbid(unsafe_code)]

use matilda_apply::{ApplyStep, FileWrite};
use matilda_ghost::{ApplyReport, StepResult};

pub use ssh::{SshAuth, SshConfig, SshError};
use ssh::SshSession;

/// Marcador de heredoc para escribir archivos remotos.
const HEREDOC: &str = "MATILDA_LINKER_EOF";

/// Comando de shell que escribe `f.content` en `f.path` del host remoto.
fn file_write_command(f: &FileWrite) -> String {
    format!(
        "cat > '{}' <<'{HEREDOC}'\n{}\n{HEREDOC}",
        f.path, f.content
    )
}

/// Enlace activo a un servidor: una sesión SSH multiplexada.
pub struct Linker {
    session: SshSession,
}

impl Linker {
    /// Conecta y autentica contra el host descrito por `config`.
    pub async fn connect(config: &SshConfig) -> Result<Linker, SshError> {
        Ok(Linker { session: SshSession::connect(config).await? })
    }

    /// Aplica un paso en el host remoto: escribe sus archivos, corre sus
    /// comandos. Se detiene en el primer error.
    async fn apply_step(&self, step: &ApplyStep) -> StepResult {
        let mut log = Vec::new();
        let mut ok = true;

        for f in &step.files {
            match self.session.exec(&file_write_command(f)).await {
                Ok(out) if out.exit_code == 0 => log.push(format!("✔ escrito {}", f.path)),
                Ok(out) => {
                    log.push(format!(
                        "✘ escribir {}: {}",
                        f.path,
                        String::from_utf8_lossy(&out.stderr).trim()
                    ));
                    ok = false;
                    break;
                }
                Err(e) => {
                    log.push(format!("✘ {e}"));
                    ok = false;
                    break;
                }
            }
        }

        if ok {
            for cmd in &step.commands {
                log.push(format!("$ {cmd}"));
                match self.session.exec(cmd).await {
                    Ok(out) => {
                        for l in String::from_utf8_lossy(&out.stdout).lines() {
                            log.push(format!("  {l}"));
                        }
                        for l in String::from_utf8_lossy(&out.stderr).lines() {
                            log.push(format!("  {l}"));
                        }
                        if out.exit_code != 0 {
                            log.push(format!("✘ el comando salió con código {}", out.exit_code));
                            ok = false;
                            break;
                        }
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

    /// Aplica los pasos en orden sobre el host remoto. Se detiene en el
    /// primero que falle (semántica `set -e`).
    pub async fn apply(&self, steps: &[ApplyStep]) -> ApplyReport {
        let mut results = Vec::new();
        for step in steps {
            let result = self.apply_step(step).await;
            let failed = !result.ok;
            results.push(result);
            if failed {
                break;
            }
        }
        ApplyReport { results }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_write_command_uses_a_heredoc() {
        let f = FileWrite {
            path: "/etc/nginx/sites-enabled/site.conf".into(),
            content: "server { listen 80; }".into(),
        };
        let cmd = file_write_command(&f);
        assert!(cmd.starts_with("cat > '/etc/nginx/sites-enabled/site.conf' <<'"));
        assert!(cmd.contains("server { listen 80; }"));
        assert!(cmd.ends_with(HEREDOC));
    }

    #[test]
    fn ssh_config_is_re_exported() {
        // El consumidor arma la conexión sin depender de ssh-multiplex.
        let c = SshConfig::new("srv.example", "deploy", SshAuth::Password("x".into()));
        assert_eq!(c.host, "srv.example");
    }

    // La aplicación remota real (`Linker::connect` + `apply`) necesita un
    // servidor SSH — se prueba fuera del unit test.
}
