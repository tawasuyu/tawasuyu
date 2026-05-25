//! Renderizado de un [`Container`] a Docker — `docker run` y compose.

use matilda_core::Container;

/// Comando `docker run` de un contenedor, en una sola línea. El orden de
/// los flags es fijo (determinista): `-d --name --restart -p -e -v img`.
pub fn docker_run_command(c: &Container) -> String {
    let mut parts: Vec<String> = vec![
        "docker".into(),
        "run".into(),
        "-d".into(),
        "--name".into(),
        c.name.clone(),
        "--restart".into(),
        c.restart.docker_flag().into(),
    ];
    for p in &c.ports {
        parts.push("-p".into());
        parts.push(format!("{}:{}", p.host, p.container));
    }
    for (k, v) in &c.env {
        parts.push("-e".into());
        parts.push(format!("{k}={v}"));
    }
    for (host, container) in &c.volumes {
        parts.push("-v".into());
        parts.push(format!("{host}:{container}"));
    }
    parts.push(c.image.clone());
    parts.join(" ")
}

/// Bloque de servicio para un `docker-compose.yml`. Viene indentado para
/// colocarse tal cual bajo la clave `services:`.
pub fn compose_service(c: &Container) -> String {
    let mut out = String::new();
    out.push_str(&format!("  {}:\n", c.name));
    out.push_str(&format!("    image: {}\n", c.image));
    out.push_str(&format!("    restart: {}\n", c.restart.docker_flag()));
    if !c.ports.is_empty() {
        out.push_str("    ports:\n");
        for p in &c.ports {
            out.push_str(&format!("      - \"{}:{}\"\n", p.host, p.container));
        }
    }
    if !c.env.is_empty() {
        out.push_str("    environment:\n");
        for (k, v) in &c.env {
            out.push_str(&format!("      - {k}={v}\n"));
        }
    }
    if !c.volumes.is_empty() {
        out.push_str("    volumes:\n");
        for (host, container) in &c.volumes {
            out.push_str(&format!("      - {host}:{container}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use matilda_core::RestartPolicy;

    fn sample() -> Container {
        Container::new("web", "nginx:1.27")
            .with_port(8080, 80)
            .with_env("TZ", "America/Caracas")
            .with_volume("/srv/web", "/usr/share/nginx/html")
            .with_restart(RestartPolicy::Always)
    }

    #[test]
    fn run_command_has_all_flags() {
        let cmd = docker_run_command(&sample());
        assert!(cmd.starts_with("docker run -d --name web --restart always"));
        assert!(cmd.contains("-p 8080:80"));
        assert!(cmd.contains("-e TZ=America/Caracas"));
        assert!(cmd.contains("-v /srv/web:/usr/share/nginx/html"));
        assert!(cmd.ends_with("nginx:1.27"));
    }

    #[test]
    fn run_command_is_deterministic() {
        assert_eq!(docker_run_command(&sample()), docker_run_command(&sample()));
    }

    #[test]
    fn compose_service_indents_under_services() {
        let yaml = compose_service(&sample());
        assert!(yaml.contains("  web:\n"));
        assert!(yaml.contains("    image: nginx:1.27\n"));
        assert!(yaml.contains("    restart: always\n"));
        assert!(yaml.contains("      - \"8080:80\"\n"));
    }

    #[test]
    fn minimal_container_omits_empty_sections() {
        let yaml = compose_service(&Container::new("bare", "alpine"));
        assert!(!yaml.contains("ports:"));
        assert!(!yaml.contains("environment:"));
        assert!(!yaml.contains("volumes:"));
    }
}
