//! `matilda-config` — del modelo declarativo a archivos de configuración.
//!
//! Funciones puras: toman un tipo de `matilda-core` y devuelven el texto
//! de configuración listo para escribir en el servidor. No tocan disco
//! ni Docker — sólo construyen strings, así que cada salida es testeable
//! y determinista.
//!
//! - [`docker`] — `Container` → `docker run` / servicio docker-compose.
//! - [`nginx`] — `VHost` → bloque `server` de nginx.

#![forbid(unsafe_code)]

pub mod docker;
pub mod nginx;

pub use docker::{compose_service, docker_run_command};
pub use nginx::nginx_server_block;

use matilda_core::Inventory;

/// Renderiza el `docker-compose.yml` completo de un inventario.
pub fn compose_file(inv: &Inventory) -> String {
    let mut out = String::from("services:\n");
    for c in inv.containers() {
        out.push_str(&compose_service(c));
    }
    out
}

/// Renderiza el archivo de sites de nginx — un bloque `server` por
/// vhost, separados por una línea en blanco.
pub fn nginx_sites(inv: &Inventory) -> String {
    inv.vhosts()
        .map(nginx_server_block)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use matilda_core::{Container, VHost};

    #[test]
    fn compose_file_lists_every_container() {
        let mut inv = Inventory::new();
        inv.add_container(Container::new("web", "nginx"));
        inv.add_container(Container::new("db", "postgres:16"));
        let yaml = compose_file(&inv);
        assert!(yaml.starts_with("services:\n"));
        assert!(yaml.contains("  web:\n") && yaml.contains("  db:\n"));
    }

    #[test]
    fn nginx_sites_renders_every_vhost() {
        let mut inv = Inventory::new();
        inv.add_vhost(VHost::to_container("a.com", "web", 80));
        inv.add_vhost(VHost::to_container("b.com", "web", 80));
        let conf = nginx_sites(&inv);
        assert!(conf.contains("server_name a.com;"));
        assert!(conf.contains("server_name b.com;"));
    }
}
