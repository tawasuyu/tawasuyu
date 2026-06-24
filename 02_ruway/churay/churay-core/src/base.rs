//! **Sistema base / escritorio mirada** — el display manager + compositor +
//! sesión, *componentizado*. NO son apps: van como opción especial.
//!
//! Espeja `scripts/install-mirada-dm.sh`, pero data-driven y reusando las
//! mismas [`Source`](crate::install::Source) (bundle / repo / compilar). Instala
//! **de verdad** bajo un `sysroot` (en producción, `/`): binarios a
//! `usr/local/bin`, el PAM del greeter a `etc/pam.d/`, las sesiones Wayland a
//! `usr/share/wayland-sessions/`, y suma al usuario a los grupos del asiento.
//!
//! Los archivos de sistema (PAM, scripts de sesión, `.desktop`) van **embebidos**
//! para que la instalación funcione sin el repo presente (en cualquier Linux).

use std::path::Path;

use crate::install::{resolve_source, InstallConfig, InstallError, Step};
use crate::manifest::{Manifest, Scope, Unit};

// --- archivos de sistema embebidos (desde el repo, a compile-time) ---
const PAM_MIRADA: &str = include_str!("../../../../shared/auth/auth-core/data/mirada");
const SESSION: &str =
    include_str!("../../../../02_ruway/mirada/mirada-compositor/session/mirada-session");
const SESSION_PATA: &str =
    include_str!("../../../../02_ruway/mirada/mirada-compositor/session/mirada-session-pata");
const SESSION_DESKTOP: &str =
    include_str!("../../../../02_ruway/mirada/mirada-compositor/session/mirada.desktop");
const SESSION_PATA_DESKTOP: &str =
    include_str!("../../../../02_ruway/mirada/mirada-compositor/session/mirada-pata.desktop");
const MIRADA_DM: &str = include_str!("../../../../scripts/mirada-dm");

/// Una acción de sistema más allá de copiar binarios.
pub enum Extra {
    /// Escribe un archivo embebido bajo `<sysroot>/<rel>` con `mode`.
    File { rel: &'static str, contents: &'static str, mode: u32 },
    /// Suma al usuario al grupo (si existe). Sólo en instalación real (`/`).
    Group(&'static str),
    /// Siembra una línea en `~/.config/mirada/autostart` (idempotente).
    Autostart(&'static str),
}

/// Un componente independiente del sistema base (marcable por separado).
pub struct Component {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    /// Binarios (nombres de programa) que instala.
    pub programs: &'static [&'static str],
    /// Pasos extra (archivos del DM, grupos, autostart).
    pub extras: &'static [Extra],
}

/// Los componentes del sistema base, en orden de instalación. Todos **activos
/// por defecto** en la UI.
pub fn base_system() -> Vec<Component> {
    vec![
        Component {
            id: "compositor",
            label: "Compositor mirada",
            description: "El compositor Wayland + gestor de ventanas (el corazón del escritorio).",
            programs: &["mirada-compositor"],
            extras: &[],
        },
        Component {
            id: "greeter",
            label: "Display manager (greeter + sesión)",
            description: "Login gráfico tipo init: PAM, sesiones Wayland y el lanzador del DM.",
            programs: &["mirada-greeter"],
            extras: &[
                Extra::File { rel: "usr/local/bin/mirada-dm", contents: MIRADA_DM, mode: 0o755 },
                Extra::File { rel: "usr/local/bin/mirada-session", contents: SESSION, mode: 0o755 },
                Extra::File { rel: "usr/local/bin/mirada-session-pata", contents: SESSION_PATA, mode: 0o755 },
                Extra::File { rel: "etc/pam.d/mirada", contents: PAM_MIRADA, mode: 0o644 },
                Extra::File { rel: "usr/share/wayland-sessions/mirada.desktop", contents: SESSION_DESKTOP, mode: 0o644 },
                Extra::File { rel: "usr/share/wayland-sessions/mirada-pata.desktop", contents: SESSION_PATA_DESKTOP, mode: 0o644 },
                Extra::Group("seat"),
                Extra::Group("video"),
                Extra::Group("input"),
            ],
        },
        Component {
            id: "barra",
            label: "Barra (pata)",
            description: "La barra de estado / panel del escritorio.",
            programs: &["pata-llimphi"],
            extras: &[],
        },
        Component {
            id: "shell",
            label: "Shell (shuma)",
            description: "La terminal/workspace que arranca con la sesión.",
            programs: &["shuma-shell-llimphi"],
            extras: &[],
        },
        Component {
            id: "launcher",
            label: "Lanzador de apps",
            description: "El menú/lanzador (Super+p) de la barra.",
            programs: &["mirada-launcher"],
            extras: &[],
        },
        Component {
            id: "control",
            label: "Control de ventanas (mirada-ctl)",
            description: "CRÍTICO: la CLI que la barra usa para escritorios y foco de ventanas.",
            programs: &["mirada-ctl"],
            extras: &[],
        },
        Component {
            id: "panel",
            label: "Panel de mirada",
            description: "Vista espacial Prezi, atajos y vistas de escritorio.",
            programs: &["mirada-llimphi"],
            extras: &[],
        },
        Component {
            id: "portal",
            label: "Portal XDG + wallpaper",
            description: "File pickers, capturas y tema para apps; setter de fondo.",
            programs: &["mirada-portal", "mirada-wallpaper"],
            extras: &[],
        },
        Component {
            id: "panel-unificado",
            label: "Panel de control unificado",
            description: "Config de mirada, pata y sistema en un solo panel (allichay).",
            programs: &["wawa-panel"],
            extras: &[],
        },
        Component {
            id: "notificaciones",
            label: "Notificaciones de escritorio",
            description: "Daemon de toasts + historial agrupado y triage semántico.",
            programs: &["pata-notify", "pata-notify-panel", "pata-notify-triage"],
            extras: &[Extra::Autostart("pata-notify")],
        },
    ]
}

/// Todos los nombres de binario del sistema base — para que el bundle/manifest
/// los incluya además de las apps.
pub fn base_programs() -> Vec<&'static str> {
    base_system().into_iter().flat_map(|c| c.programs.iter().copied().collect::<Vec<_>>()).collect()
}

/// Instala los componentes `comps` bajo `sysroot` (en producción, `/`),
/// reusando `cfg` para conseguir los binarios (bundle/repo/compilar). Si se da
/// `manifest`, los binarios se verifican por hash. Reporta progreso por
/// `(component_id, step, ratio)`.
pub fn install_base(
    sysroot: &Path,
    cfg: &InstallConfig,
    manifest: Option<&Manifest>,
    comps: &[&Component],
    on: &mut dyn FnMut(&str, Step, f32),
) -> Result<(), InstallError> {
    let real = sysroot == Path::new("/");
    let bindir = sysroot.join("usr/local/bin");
    std::fs::create_dir_all(&bindir)?;

    for c in comps {
        for prog in c.programs {
            on(c.id, Step::Resolviendo, 0.0);
            let unit = prog_unit(prog, manifest);
            let src = resolve_source(cfg, &unit)
                .ok_or_else(|| InstallError::SinFuente { program: prog.to_string() })?;
            let dest = bindir.join(prog);
            src.provide(&unit, &dest, &mut |s, r| on(c.id, s, r))?;
        }
        for extra in c.extras {
            apply_extra(sysroot, real, extra)?;
        }
        on(c.id, Step::Hecho, 1.0);
    }
    Ok(())
}

/// Unidad mínima para un binario del sistema base, con el hash del manifiesto
/// si está disponible (habilita verificación + descarga remota).
fn prog_unit(program: &str, manifest: Option<&Manifest>) -> Unit {
    let bin_hash = manifest
        .and_then(|m| m.units.iter().find(|u| u.program == program))
        .and_then(|u| u.bin_hash.clone());
    Unit {
        id: program.to_string(),
        label: program.to_string(),
        version: crate::SUITE_VERSION.to_string(),
        category: "sistema".to_string(),
        icon: "⚙".to_string(),
        description: String::new(),
        program: program.to_string(),
        scope: Scope::System,
        suggests: Vec::new(),
        handles: Vec::new(),
        launchable: false,
        post_install: None,
        bin_hash,
        size_bytes: None,
    }
}

fn apply_extra(sysroot: &Path, real: bool, extra: &Extra) -> Result<(), InstallError> {
    use std::os::unix::fs::PermissionsExt;
    match extra {
        Extra::File { rel, contents, mode } => {
            let path = sysroot.join(rel);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, contents)?;
            let mut perms = std::fs::metadata(&path)?.permissions();
            perms.set_mode(*mode);
            std::fs::set_permissions(&path, perms)?;
        }
        Extra::Group(g) => {
            if real {
                if let Ok(user) = std::env::var("USER") {
                    // Sólo si el grupo existe; ignora fallos (igual que el script).
                    let existe = std::process::Command::new("getent")
                        .args(["group", g])
                        .stdout(std::process::Stdio::null())
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                    if existe {
                        let _ = std::process::Command::new("usermod")
                            .args(["-aG", g, &user])
                            .status();
                    }
                }
            }
        }
        Extra::Autostart(line) => {
            if real {
                if let Some(home) = directories::BaseDirs::new() {
                    let dir = home.home_dir().join(".config").join("mirada");
                    let _ = std::fs::create_dir_all(&dir);
                    let auto = dir.join("autostart");
                    let ya = std::fs::read_to_string(&auto).unwrap_or_default();
                    if !ya.lines().any(|l| l.trim() == *line) {
                        use std::io::Write;
                        if let Ok(mut f) =
                            std::fs::OpenOptions::new().create(true).append(true).open(&auto)
                        {
                            let _ = writeln!(f, "{line}");
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::{InstallConfig, InstallMode};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn instala_base_a_un_sysroot_real() {
        let root = std::env::temp_dir().join(format!("churay-base-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bundle = root.join("bundle");
        let sysroot = root.join("sys");
        std::fs::create_dir_all(bundle.join("bin")).unwrap();
        // Binarios falsos para todos los programas del sistema base.
        for prog in base_programs() {
            std::fs::write(bundle.join("bin").join(prog), b"#!/bin/sh\n").unwrap();
        }

        let cfg = InstallConfig {
            mode: InstallMode::System,
            prefix: sysroot.join("usr/local"),
            bundle_dir: Some(bundle),
            workspace_root: None,
            remote_base_url: None,
            cache_dir: root.join("c"),
        };
        let comps = base_system();
        let refs: Vec<&Component> = comps.iter().collect();
        install_base(&sysroot, &cfg, None, &refs, &mut |_, _, _| {}).unwrap();

        // Compositor + greeter + ctl instalados y ejecutables.
        for prog in ["mirada-compositor", "mirada-greeter", "mirada-ctl", "pata-llimphi"] {
            let p = sysroot.join("usr/local/bin").join(prog);
            assert!(p.exists(), "{prog} no instalado");
            assert!(p.metadata().unwrap().permissions().mode() & 0o111 != 0);
        }
        // Archivos del DM en su lugar, con sus modos.
        let pam = sysroot.join("etc/pam.d/mirada");
        assert!(pam.exists());
        assert_eq!(pam.metadata().unwrap().permissions().mode() & 0o777, 0o644);
        let dm = sysroot.join("usr/local/bin/mirada-dm");
        assert_eq!(dm.metadata().unwrap().permissions().mode() & 0o777, 0o755);
        assert!(sysroot.join("usr/share/wayland-sessions/mirada.desktop").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn instalar_un_solo_componente() {
        let root = std::env::temp_dir().join(format!("churay-base1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bundle = root.join("bundle");
        let sysroot = root.join("sys");
        std::fs::create_dir_all(bundle.join("bin")).unwrap();
        std::fs::write(bundle.join("bin").join("mirada-compositor"), b"x").unwrap();

        let cfg = InstallConfig {
            mode: InstallMode::System,
            prefix: sysroot.join("usr/local"),
            bundle_dir: Some(bundle),
            workspace_root: None,
            remote_base_url: None,
            cache_dir: root.join("c"),
        };
        let comps = base_system();
        let solo: Vec<&Component> = comps.iter().filter(|c| c.id == "compositor").collect();
        install_base(&sysroot, &cfg, None, &solo, &mut |_, _, _| {}).unwrap();
        assert!(sysroot.join("usr/local/bin/mirada-compositor").exists());
        // No instaló el greeter.
        assert!(!sysroot.join("usr/local/bin/mirada-greeter").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
