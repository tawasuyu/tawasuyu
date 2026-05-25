//! Construcción del entorno del hijo. Sin globals — toma EnvSpec por valor.

use brahman_card::Card;
use std::path::PathBuf;

/// Var env para el path del bus interno (cuando aplica). Mismo nombre que
/// usa ente-bus para que clientes existentes (`BusClient::from_env`) sigan
/// funcionando sin cambios.
pub const ENV_BUS_SOCK: &str = "ENTE_BUS_SOCK";

/// Var env para el ULID de la Card encarnada.
pub const ENV_ENTE_ID: &str = "ENTE_ID";

#[derive(Debug, Clone, Default)]
pub struct EnvSpec {
    /// Si `Some`, se inyecta como ENTE_BUS_SOCK.
    pub bus_sock: Option<PathBuf>,
    /// Si `Some`, se inyecta como NOTIFY_SOCKET (legacy sd_notify).
    pub notify_socket: Option<PathBuf>,
    /// Vars adicionales que el caller quiere forzar.
    pub extra: Vec<(String, String)>,
}

/// Hereda env del padre, aplica el envp explícito de la Card, y al final
/// inyecta las vars del fractal según `EnvSpec`.
pub fn build_env(card: &Card, base_envp: &[(String, String)], spec: &EnvSpec) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = std::env::vars().collect();

    for (k, v) in base_envp {
        env.retain(|(ek, _)| ek != k);
        env.push((k.clone(), v.clone()));
    }

    if let Some(p) = &spec.bus_sock {
        env.retain(|(k, _)| k != ENV_BUS_SOCK);
        env.push((ENV_BUS_SOCK.into(), p.to_string_lossy().into_owned()));
    }

    env.retain(|(k, _)| k != ENV_ENTE_ID);
    env.push((ENV_ENTE_ID.into(), card.id.to_string()));

    if let Some(p) = &spec.notify_socket {
        env.retain(|(k, _)| k != "NOTIFY_SOCKET");
        env.push(("NOTIFY_SOCKET".into(), p.to_string_lossy().into_owned()));
    }

    for (k, v) in &spec.extra {
        env.retain(|(ek, _)| ek != k);
        env.push((k.clone(), v.clone()));
    }

    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use brahman_card::Card;

    #[test]
    fn env_id_and_bus_injected() {
        let card = Card::new("test");
        let spec = EnvSpec {
            bus_sock: Some(PathBuf::from("/tmp/bus.sock")),
            notify_socket: None,
            extra: vec![],
        };
        let env = build_env(&card, &[], &spec);
        assert!(env.iter().any(|(k, v)| k == ENV_ENTE_ID && v == &card.id.to_string()));
        assert!(env.iter().any(|(k, v)| k == ENV_BUS_SOCK && v == "/tmp/bus.sock"));
    }

    #[test]
    fn extra_overrides_inherited() {
        let card = Card::new("test");
        let spec = EnvSpec {
            bus_sock: None,
            notify_socket: None,
            extra: vec![("PATH".into(), "/sandbox/bin".into())],
        };
        let env = build_env(&card, &[], &spec);
        let path_count = env.iter().filter(|(k, _)| k == "PATH").count();
        assert_eq!(path_count, 1);
        assert_eq!(env.iter().find(|(k, _)| k == "PATH").unwrap().1, "/sandbox/bin");
    }

    #[test]
    fn notify_socket_only_when_set() {
        let card = Card::new("test");
        let spec = EnvSpec::default();
        let env = build_env(&card, &[], &spec);
        assert!(!env.iter().any(|(k, _)| k == "NOTIFY_SOCKET"
            && std::env::var("NOTIFY_SOCKET").is_err()));
    }
}
