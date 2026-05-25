//! Path simple: spawn directo, sin namespacing.

use crate::env::{build_env, EnvSpec};
use crate::error::IncarnateError;
use crate::pre_exec::{apply_unchecked, ChildSetup};
use crate::ChildStdio;
use card_core::{Card, Payload};
use nix::unistd::Pid;
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

pub fn incarnate_plain(
    card: &Card,
    env_spec: &EnvSpec,
    stdio: &ChildStdio,
    setup: &ChildSetup,
) -> Result<Pid, IncarnateError> {
    let (exec, argv, base_envp) = match &card.payload {
        Payload::Native { exec, argv, envp } => (exec.clone(), argv.clone(), envp.clone()),
        Payload::Legacy { exec, argv, .. } => (exec.clone(), argv.clone(), Vec::new()),
        _ => return Err(IncarnateError::NonExecutablePayload),
    };
    let env = build_env(card, &base_envp, env_spec);
    let mut cmd = Command::new(&exec);
    cmd.args(&argv);
    cmd.env_clear();
    for (k, v) in &env {
        cmd.env(k, v);
    }
    if let Some(fd) = stdio.stdin_fd {
        // SAFETY: el caller garantiza que `fd` está abierto y le
        // transfiere ownership al child. `Command` lo cierra tras spawn.
        cmd.stdin(unsafe { Stdio::from_raw_fd(fd) });
    }
    if let Some(fd) = stdio.stdout_fd {
        cmd.stdout(unsafe { Stdio::from_raw_fd(fd) });
    }
    if let Some(fd) = stdio.stderr_fd {
        cmd.stderr(unsafe { Stdio::from_raw_fd(fd) });
    }
    if !setup.is_empty() {
        // Clone para que la closure sea 'static (Command::pre_exec lo exige).
        let ops = setup.ops.clone();
        // SAFETY: pre_exec corre post-fork pre-exec. apply_unchecked sólo
        // hace syscalls async-signal-safe.
        unsafe {
            cmd.pre_exec(move || {
                let r = apply_unchecked(&ops);
                if r != 0 {
                    Err(std::io::Error::from_raw_os_error(libc::EINVAL))
                } else {
                    Ok(())
                }
            });
        }
    }
    let child = cmd.spawn()?;
    Ok(Pid::from_raw(child.id() as i32))
}
