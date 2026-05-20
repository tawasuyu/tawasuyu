//! `matilda` — demostración de administración de servidores.
//!
//! Declara un inventario *deseado*, renderiza su `docker-compose.yml` y
//! su configuración nginx, y luego calcula el *plan* que lleva un
//! servidor desde un estado actual distinto hasta el deseado.
//!
//! Smoke test legible del módulo: `cargo run -p matilda`.

use matilda_core::{Container, Host, Inventory, RestartPolicy, VHost};
use matilda_config::{compose_file, nginx_sites};
use matilda_plan::plan;

/// El inventario que queremos tener en el servidor.
fn desired() -> Inventory {
    let mut inv = Inventory::new();
    inv.add_host(Host::new("edge-1", "10.0.0.1").with_tag("prod"));

    inv.add_container(
        Container::new("web", "nginx:1.27")
            .with_port(8080, 80)
            .with_volume("/srv/site", "/usr/share/nginx/html")
            .with_restart(RestartPolicy::Always),
    );
    inv.add_container(
        Container::new("api", "ghcr.io/jls/api:2.4")
            .with_port(9000, 9000)
            .with_env("DATABASE_URL", "postgres://db/app")
            .with_restart(RestartPolicy::UnlessStopped),
    );
    inv.add_container(
        Container::new("db", "postgres:16")
            .with_env("POSTGRES_DB", "app")
            .with_volume("/srv/pgdata", "/var/lib/postgresql/data")
            .with_restart(RestartPolicy::Always),
    );

    inv.add_vhost(VHost::to_container("jlsoltech.com", "web", 80).with_alias("www.jlsoltech.com").with_tls());
    inv.add_vhost(VHost::to_container("api.jlsoltech.com", "api", 9000).with_tls());
    inv
}

/// El estado en que está el servidor hoy: `web` con imagen vieja, sin
/// `api`, y un contenedor `legacy` que ya no se quiere.
fn current() -> Inventory {
    let mut inv = Inventory::new();
    inv.add_host(Host::new("edge-1", "10.0.0.1").with_tag("prod"));
    inv.add_container(Container::new("web", "nginx:1.25").with_port(8080, 80));
    inv.add_container(Container::new("db", "postgres:16")
        .with_env("POSTGRES_DB", "app")
        .with_volume("/srv/pgdata", "/var/lib/postgresql/data")
        .with_restart(RestartPolicy::Always));
    inv.add_container(Container::new("legacy", "old/cgi:1"));
    inv.add_vhost(VHost::to_container("jlsoltech.com", "web", 80));
    inv
}

fn rule(title: &str) {
    println!("\n── {title} {}", "─".repeat(56usize.saturating_sub(title.len())));
}

fn main() {
    let desired = desired();

    rule("docker-compose.yml (deseado)");
    print!("{}", compose_file(&desired));

    rule("nginx — sites (deseado)");
    print!("{}", nginx_sites(&desired));

    rule("plan de reconciliación (actual → deseado)");
    let current = current();
    let p = plan(&current, &desired);
    if p.is_empty() {
        println!("  sin cambios: el servidor ya está al día.");
    } else {
        for (i, action) in p.actions.iter().enumerate() {
            println!("  {:>2}. {}", i + 1, action.describe());
        }
        println!(
            "\n  {} acciones — {} crear, {} actualizar, {} eliminar.",
            p.len(),
            p.count(matilda_plan::Op::Create),
            p.count(matilda_plan::Op::Update),
            p.count(matilda_plan::Op::Remove),
        );
    }

    let broken = desired.broken_vhosts();
    rule("consistencia");
    if broken.is_empty() {
        println!("  todos los vhosts apuntan a contenedores existentes. ✔");
    } else {
        for v in broken {
            println!("  ✘ vhost «{}» apunta a un contenedor inexistente", v.domain);
        }
    }
    println!();
}
