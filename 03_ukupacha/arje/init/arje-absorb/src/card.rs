//! Traducción del modelo intermedio a Tarjetas Semilla brahman.
//!
//! Cada [`ForeignService`] se vuelve una Card hija; el conjunto cuelga
//! como `genesis` de una Card raíz que `arje-zero` encarna al arrancar.

use std::time::Duration;

use card_core::{
    Capability, Card, CgroupSpec, FsPolicy, Lifecycle, NetworkingPolicy, Payload, Permissions,
    Priority, SomaSpec, Supervision,
};

use crate::model::{ForeignService, ServiceKind};

/// Convierte un servicio absorbido en una Card hija (genesis child).
fn service_to_card(svc: &ForeignService) -> Card {
    let (lifecycle, supervision) = match svc.kind {
        ServiceKind::Daemon => (
            Lifecycle::Daemon,
            Supervision::Restart {
                initial: Duration::from_millis(1_000),
                max: Duration::from_millis(60_000),
            },
        ),
        ServiceKind::OneShot => (Lifecycle::Oneshot, Supervision::OneShot),
    };
    Card {
        payload: Payload::Native {
            exec: svc.exec.clone(),
            argv: svc.argv.clone(),
            envp: svc.env.clone(),
        },
        supervision,
        lifecycle,
        priority: Priority::Normal,
        // Servicio de sistema absorbido: sin aislar (namespaces en
        // `false` por defecto), con FS de escritura, red y subprocesos.
        // La migración conserva el comportamiento; endurecer el sandbox
        // queda como trabajo posterior, Card por Card.
        permissions: Permissions {
            filesystem: FsPolicy::ReadWrite,
            networking: NetworkingPolicy::Full,
            processes: true,
            ..Permissions::default()
        },
        soma: SomaSpec {
            cgroup: CgroupSpec {
                path: "ente.slice/absorbed".to_string(),
                ..CgroupSpec::default()
            },
            ..SomaSpec::default()
        },
        ..Card::new(svc.name.clone())
    }
}

/// La Card de `carmen-dm`: el compositor `mirada` en modo greeter,
/// como gestor de login gráfico. Para agregar a una Semilla absorbida
/// al migrar un escritorio (flag `--with-carmen`). Idéntica a la
/// entrada `carmen-dm` de `seeds/arje-host.card.json`.
pub fn carmen_dm_card() -> Card {
    Card {
        payload: Payload::Native {
            exec: "/usr/bin/mirada-compositor".to_string(),
            argv: vec!["--greeter".to_string(), "--drm".to_string()],
            envp: vec![
                (
                    "PATH".to_string(),
                    "/usr/local/bin:/usr/bin:/usr/sbin:/sbin:/bin".to_string(),
                ),
                ("XDG_RUNTIME_DIR".to_string(), "/run".to_string()),
                (
                    "MIRADA_GREETER_BIN".to_string(),
                    "/usr/bin/mirada-greeter".to_string(),
                ),
            ],
        },
        supervision: Supervision::Restart {
            initial: Duration::from_millis(2_000),
            max: Duration::from_millis(60_000),
        },
        lifecycle: Lifecycle::Daemon,
        priority: Priority::High,
        permissions: Permissions {
            filesystem: FsPolicy::ReadWrite,
            networking: NetworkingPolicy::None,
            processes: true,
            ..Permissions::default()
        },
        soma: SomaSpec {
            cgroup: CgroupSpec {
                path: "ente.slice/carmen".to_string(),
                ..CgroupSpec::default()
            },
            ..SomaSpec::default()
        },
        ..Card::new("carmen-dm")
    }
}

/// Arma una Tarjeta Semilla raíz que encarna todos los servicios
/// absorbidos como hijas `genesis` de `arje-zero`.
pub fn build_seed(label: &str, services: &[ForeignService]) -> Card {
    let genesis: Vec<Card> = services.iter().map(service_to_card).collect();
    Card {
        provides: [Capability::Spawn, Capability::Journal]
            .into_iter()
            .collect(),
        permissions: Permissions {
            filesystem: FsPolicy::ReadWrite,
            networking: NetworkingPolicy::Full,
            processes: true,
            ..Permissions::default()
        },
        genesis,
        ..Card::new(label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(name: &str, kind: ServiceKind) -> ForeignService {
        ForeignService {
            name: name.to_string(),
            exec: "/usr/bin/foo".to_string(),
            argv: vec!["-x".to_string()],
            env: Vec::new(),
            kind,
        }
    }

    #[test]
    fn seed_validates() {
        let seed = build_seed(
            "arje.seed.absorbed",
            &[svc("a", ServiceKind::Daemon), svc("b", ServiceKind::OneShot)],
        );
        seed.validate().expect("la Semilla absorbida debe validar");
        assert_eq!(seed.genesis.len(), 2);
    }

    #[test]
    fn daemon_maps_to_restart() {
        let c = service_to_card(&svc("d", ServiceKind::Daemon));
        assert!(matches!(c.supervision, Supervision::Restart { .. }));
        assert_eq!(c.lifecycle, Lifecycle::Daemon);
    }

    #[test]
    fn oneshot_maps_to_oneshot() {
        let c = service_to_card(&svc("o", ServiceKind::OneShot));
        assert!(matches!(c.supervision, Supervision::OneShot));
        assert_eq!(c.lifecycle, Lifecycle::Oneshot);
    }

    #[test]
    fn children_get_distinct_ids() {
        let seed = build_seed(
            "x",
            &[svc("a", ServiceKind::Daemon), svc("b", ServiceKind::Daemon)],
        );
        assert_ne!(seed.genesis[0].id, seed.genesis[1].id);
    }

    #[test]
    fn empty_service_list_still_validates() {
        let seed = build_seed("arje.seed.absorbed", &[]);
        seed.validate().expect("una Semilla sin hijas es válida");
    }

    #[test]
    fn carmen_card_is_valid_greeter() {
        let c = carmen_dm_card();
        c.validate().expect("la Card de carmen-dm debe validar");
        match &c.payload {
            card_core::Payload::Native { exec, argv, .. } => {
                assert_eq!(exec, "/usr/bin/mirada-compositor");
                assert!(argv.contains(&"--greeter".to_string()));
            }
            _ => panic!("carmen-dm debe ser un payload Native"),
        }
    }

    #[test]
    fn carmen_can_be_appended_to_absorbed_seed() {
        let mut seed = build_seed("arje.seed.absorbed", &[svc("a", ServiceKind::Daemon)]);
        seed.genesis.push(carmen_dm_card());
        seed.validate().expect("la Semilla con carmen debe validar");
        assert_eq!(seed.genesis.len(), 2);
    }
}
