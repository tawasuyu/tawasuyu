//! `matilda-apply` — el puente entre el plan y la ejecución real.
//!
//! `matilda-plan` dice *qué* cambiar (una lista ordenada de `Action`s).
//! Este crate dice *cómo*: traduce cada acción a un [`ApplyStep`]
//! concreto — los archivos a escribir en el servidor y los comandos a
//! correr, en orden.
//!
//! Sigue siendo **agnóstico de transporte**: no abre conexiones ni
//! ejecuta nada. Aplicar los pasos —localmente, por SSH o vía el agente
//! `matilda-ghost`— es trabajo de la capa de I/O. Aquí todo es una
//! función pura y testeable.

#![forbid(unsafe_code)]

pub mod lifecycle;
pub use lifecycle::{ContainerAction, ServiceAction};

use matilda_config::{docker_run_command, nginx_server_block};
use matilda_core::Inventory;
use matilda_plan::{Op, Plan, Resource};
use serde::{Deserialize, Serialize};

/// Directorio donde matilda deja los `server` de nginx.
const NGINX_SITES: &str = "/etc/nginx/sites-enabled";

/// Un archivo a escribir en el servidor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileWrite {
    pub path: String,
    pub content: String,
}

/// Un paso de aplicación: la traducción concreta de una acción del plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyStep {
    /// Descripción legible de la acción de origen.
    pub describe: String,
    /// Archivos a escribir en el servidor (antes de los comandos).
    pub files: Vec<FileWrite>,
    /// Comandos de shell a ejecutar, en orden.
    pub commands: Vec<String>,
}

/// Ruta del archivo `server` de un dominio.
fn vhost_path(domain: &str) -> String {
    format!("{NGINX_SITES}/{domain}.conf")
}

/// Traduce un plan a pasos concretos de aplicación.
///
/// Necesita el inventario **deseado** para conocer los detalles de cada
/// recurso (imagen del contenedor, upstream del vhost). Las acciones
/// sobre *hosts* no producen pasos: un host es a qué servidor conectarse,
/// no algo que se "aplique" en él.
pub fn plan_to_steps(plan: &Plan, desired: &Inventory) -> Vec<ApplyStep> {
    let mut steps = Vec::new();
    for action in &plan.actions {
        let describe = action.describe();
        let step = match (action.op, action.resource) {
            // --- Contenedores ---
            (Op::Create, Resource::Container) => desired
                .container(&action.name)
                .map(|c| ApplyStep {
                    describe,
                    files: Vec::new(),
                    commands: vec![docker_run_command(c)],
                }),
            (Op::Update, Resource::Container) => desired.container(&action.name).map(|c| {
                ApplyStep {
                    describe,
                    files: Vec::new(),
                    // Recrear: quitar el viejo, lanzar el nuevo.
                    commands: vec![
                        format!("docker rm -f {}", action.name),
                        docker_run_command(c),
                    ],
                }
            }),
            (Op::Remove, Resource::Container) => Some(ApplyStep {
                describe,
                files: Vec::new(),
                commands: vec![format!("docker rm -f {}", action.name)],
            }),

            // --- VHosts ---
            (Op::Create | Op::Update, Resource::VHost) => {
                desired.vhost(&action.name).map(|v| ApplyStep {
                    describe,
                    files: vec![FileWrite {
                        path: vhost_path(&action.name),
                        content: nginx_server_block(v),
                    }],
                    commands: vec!["nginx -t && nginx -s reload".to_string()],
                })
            }
            (Op::Remove, Resource::VHost) => Some(ApplyStep {
                describe,
                files: Vec::new(),
                commands: vec![
                    format!("rm -f {}", vhost_path(&action.name)),
                    "nginx -t && nginx -s reload".to_string(),
                ],
            }),

            // --- Hosts: no se "aplican" (son destino de conexión) ---
            (_, Resource::Host) => None,
        };
        if let Some(step) = step {
            steps.push(step);
        }
    }
    steps
}

/// Vuelca los pasos a un script de shell único — útil para revisarlo, o
/// para ejecutarlo de un tirón en el servidor. Los archivos se emiten
/// como heredocs.
pub fn steps_to_script(steps: &[ApplyStep]) -> String {
    let mut out = String::from("#!/usr/bin/env bash\nset -euo pipefail\n");
    for step in steps {
        out.push_str(&format!("\n# {}\n", step.describe));
        for f in &step.files {
            out.push_str(&format!("cat > {} <<'MATILDA_EOF'\n", f.path));
            out.push_str(&f.content);
            if !f.content.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("MATILDA_EOF\n");
        }
        for cmd in &step.commands {
            out.push_str(cmd);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use matilda_core::{Container, VHost};

    fn desired() -> Inventory {
        let mut inv = Inventory::new();
        inv.add_container(Container::new("web", "nginx:1.27").with_port(8080, 80));
        inv.add_vhost(VHost::to_container("site.com", "web", 8080));
        inv
    }

    #[test]
    fn fresh_inventory_produces_create_steps() {
        let steps = plan_to_steps(&matilda_plan::plan(&Inventory::new(), &desired()), &desired());
        assert_eq!(steps.len(), 2); // un contenedor + un vhost
        // El contenedor se crea con `docker run`.
        assert!(steps[0].commands[0].starts_with("docker run -d --name web"));
        // El vhost escribe su archivo y recarga nginx.
        assert_eq!(steps[1].files.len(), 1);
        assert!(steps[1].files[0].path.ends_with("site.com.conf"));
        assert!(steps[1].commands[0].contains("nginx -s reload"));
    }

    #[test]
    fn update_recreates_the_container() {
        let mut current = Inventory::new();
        current.add_container(Container::new("web", "nginx:1.25"));
        let steps = plan_to_steps(&matilda_plan::plan(&current, &desired()), &desired());
        let cont = steps.iter().find(|s| s.describe.contains("contenedor")).unwrap();
        assert_eq!(cont.commands[0], "docker rm -f web");
        assert!(cont.commands[1].starts_with("docker run"));
    }

    #[test]
    fn removal_steps_clean_up() {
        let mut current = Inventory::new();
        current.add_container(Container::new("viejo", "img"));
        current.add_vhost(VHost::to_address("viejo.com", "1.2.3.4:80"));
        let steps = plan_to_steps(&matilda_plan::plan(&current, &Inventory::new()), &Inventory::new());
        let cmds: Vec<&str> = steps
            .iter()
            .flat_map(|s| s.commands.iter())
            .map(|s| s.as_str())
            .collect();
        assert!(cmds.iter().any(|c| c.contains("docker rm -f viejo")));
        assert!(cmds.iter().any(|c| c.contains("rm -f") && c.contains("viejo.com")));
    }

    #[test]
    fn host_actions_produce_no_steps() {
        let mut desired = Inventory::new();
        desired.add_host(matilda_core::Host::new("edge", "10.0.0.1"));
        let steps = plan_to_steps(&matilda_plan::plan(&Inventory::new(), &desired), &desired);
        assert!(steps.is_empty());
    }

    #[test]
    fn empty_plan_yields_no_steps() {
        let inv = desired();
        let steps = plan_to_steps(&matilda_plan::plan(&inv, &inv.clone()), &inv);
        assert!(steps.is_empty());
    }

    #[test]
    fn script_emits_heredocs_and_commands() {
        let steps = plan_to_steps(&matilda_plan::plan(&Inventory::new(), &desired()), &desired());
        let script = steps_to_script(&steps);
        assert!(script.starts_with("#!/usr/bin/env bash"));
        assert!(script.contains("docker run -d --name web"));
        assert!(script.contains("cat > /etc/nginx/sites-enabled/site.com.conf <<'MATILDA_EOF'"));
        assert!(script.contains("MATILDA_EOF"));
    }
}
