//! Traducción del modelo intermedio a Tarjetas Semilla brahman.
//!
//! Cada [`ForeignService`] se vuelve una Card hija; el conjunto cuelga
//! como `genesis` de una Card raíz que `arje-zero` encarna al arrancar.

use std::path::Path;
use std::time::Duration;

use card_core::{
    Capability, Card, CgroupSpec, FsPolicy, Lifecycle, NetworkingPolicy, Payload, Permissions,
    Priority, SomaSpec, Supervision,
};

use crate::model::{ForeignService, ServiceKind};

/// Primer candidato de `cands` que existe bajo `root`; devuelve el path
/// ABSOLUTO en el sistema destino (sin el prefijo `root`, que sólo se usa para
/// chequear presencia — el seed corre sobre el sistema real con `root=/`).
/// Es el corazón de la generalización: el seed se DERIVA de qué binarios tiene
/// la máquina, en vez de hardcodear `/usr/bin/...` de una distro.
fn detect(root: &Path, cands: &[&str]) -> Option<String> {
    cands.iter().find_map(|c| {
        let rel = c.strip_prefix('/').unwrap_or(c);
        root.join(rel).exists().then(|| (*c).to_string())
    })
}

/// Construye una Card Native del overlay gráfico (daemon supervisado o oneshot).
fn overlay_card(
    name: &str,
    exec: String,
    argv: &[&str],
    daemon: bool,
    priority: Priority,
) -> Card {
    let (lifecycle, supervision) = if daemon {
        (
            Lifecycle::Daemon,
            Supervision::Restart {
                initial: Duration::from_millis(300),
                max: Duration::from_millis(30_000),
            },
        )
    } else {
        (Lifecycle::Oneshot, Supervision::OneShot)
    };
    Card {
        payload: Payload::Native {
            exec,
            argv: argv.iter().map(|s| s.to_string()).collect(),
            envp: Vec::new(),
        },
        supervision,
        lifecycle,
        priority,
        permissions: Permissions {
            filesystem: FsPolicy::ReadWrite,
            networking: NetworkingPolicy::Full,
            processes: true,
            ..Permissions::default()
        },
        soma: SomaSpec {
            cgroup: CgroupSpec {
                path: format!("ente.slice/{name}"),
                ..CgroupSpec::default()
            },
            ..SomaSpec::default()
        },
        ..Card::new(name)
    }
}

/// Nombres de servicio que el overlay gráfico PROVEE como daemons propios; sus
/// equivalentes absorbidos (wrappers `/etc/init.d/<svc>` oneshot) se descartan
/// para no duplicarlos. Ver [`OVERLAY_OVERRIDES`].
pub const OVERLAY_OVERRIDES: &[&str] = &[
    "udev", "eudev", "seatd", "agetty", "getty", "elogind",
    // display-managers ajenos: carmen ES el DM; dos pelean por el DRM.
    "sddm", "gdm", "lightdm", "greetd", "lxdm", "xdm", "slim", "-ly", "ly.",
];

/// Overlay del **escritorio tawasuyu**: el stack gráfico que `arje-absorb` no
/// puede sacar del init del host (es nuestro), con los paths de sistema
/// DETECTADOS bajo `root` para que ande en cualquier distro, no sólo en una.
/// Orden: udev (input) → seatd (asiento) → red → splash → compositor → getty.
pub fn graphical_overlay(root: &Path) -> Vec<Card> {
    let mut cards = Vec::new();

    // Montaje de /etc/fstab (p. ej. /home en partición aparte) y swap. Sin esto
    // la sesión queda sin home. Oneshots.
    if let Some(mount) = detect(root, &["/bin/mount", "/usr/bin/mount"]) {
        cards.push(overlay_card("mount-fstab", mount, &["-a"], false, Priority::High));
    }
    if let Some(swapon) = detect(root, &["/sbin/swapon", "/usr/sbin/swapon", "/usr/bin/swapon"]) {
        cards.push(overlay_card("swap-on", swapon, &["-a"], false, Priority::Normal));
    }

    // udevd: sin él los dispositivos no llevan ID_INPUT y libinput no ve el
    // teclado. Daemon (el wrapper init.d absorbido es oneshot → no supervisa).
    if let Some(udevd) = detect(
        root,
        &[
            "/usr/bin/udevd",
            "/sbin/udevd",
            "/lib/systemd/systemd-udevd",
            "/usr/lib/systemd/systemd-udevd",
        ],
    ) {
        cards.push(overlay_card("udevd", udevd, &[], true, Priority::High));
        if let Some(udevadm) = detect(root, &["/usr/bin/udevadm", "/sbin/udevadm", "/bin/udevadm"])
        {
            cards.push(overlay_card(
                "udev-coldplug",
                udevadm,
                &["trigger", "--action=add", "--settle"],
                false,
                Priority::High,
            ));
        }
    }

    // seatd: el asiento para libseat cuando no hay logind (el backend builtin
    // crashea con mirada). Daemon. Si no está, libseat auto-detecta (logind).
    if let Some(seatd) = detect(root, &["/usr/bin/seatd", "/sbin/seatd", "/usr/local/bin/seatd"]) {
        cards.push(overlay_card("seatd", seatd, &[], true, Priority::High));
    }

    // Red. WiFi necesita el gestor que ya tiene las redes guardadas del host
    // (NetworkManager / iwd / connman); esos hablan por el **bus de sistema**, así
    // que si hay uno, levantamos dbus-system primero. dhcpcd solo (cableado) es el
    // último recurso. Todo DETECTADO: corre el gestor que la máquina YA usa, con
    // sus credenciales — no uno que yo elija.
    let dbus = detect(root, &["/usr/bin/dbus-daemon", "/bin/dbus-daemon", "/usr/sbin/dbus-daemon"]);
    let nm = detect(root, &["/usr/bin/NetworkManager", "/usr/sbin/NetworkManager"]);
    let iwd = detect(root, &["/usr/lib/iwd/iwd", "/usr/libexec/iwd", "/usr/libexec/iwd/iwd"]);
    let connman = detect(root, &["/usr/bin/connmand", "/usr/sbin/connmand"]);

    if (nm.is_some() || iwd.is_some() || connman.is_some()) && dbus.is_some() {
        // El gestor de WiFi habla por el bus de sistema → sin esto no arranca.
        cards.push(overlay_card(
            "dbus-system",
            dbus.unwrap(),
            &["--system", "--nofork"],
            true,
            Priority::High,
        ));
    }
    if let Some(nm) = nm {
        cards.push(overlay_card("networkmanager", nm, &["--no-daemon"], true, Priority::Normal));
    } else if let Some(iwd) = iwd {
        cards.push(overlay_card("iwd", iwd, &[], true, Priority::Normal));
    } else if let Some(connman) = connman {
        cards.push(overlay_card("connman", connman, &["-n"], true, Priority::Normal));
    } else if let Some(dhcpcd) =
        detect(root, &["/usr/sbin/dhcpcd", "/sbin/dhcpcd", "/usr/bin/dhcpcd"])
    {
        cards.push(overlay_card("dhcpcd", dhcpcd, &["-B"], true, Priority::Normal));
    }

    // Splash sin parpadeo (la chakana). Path tawasuyu, lo instala el instalador.
    cards.push(overlay_card(
        "arje-splash",
        "/usr/local/lib/arje/arje-splash".to_string(),
        &[],
        false,
        Priority::High,
    ));

    // El compositor-greeter (auto-detecta el asiento; sin pin de LIBSEAT_BACKEND).
    cards.push(carmen_dm_card());

    // Getty de rescate en tty2 (tty1 es del compositor).
    if let Some(getty) = detect(root, &["/sbin/agetty", "/usr/bin/agetty", "/bin/agetty"]) {
        cards.push(overlay_card(
            "agetty-rescue",
            getty,
            &["--noclear", "tty2", "linux"],
            true,
            Priority::Normal,
        ));
    }

    cards
}

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
            exec: "/usr/local/bin/mirada-compositor".to_string(),
            argv: vec!["--greeter".to_string(), "--drm".to_string()],
            envp: vec![
                (
                    "PATH".to_string(),
                    "/usr/local/bin:/usr/bin:/usr/sbin:/sbin:/bin".to_string(),
                ),
                ("XDG_RUNTIME_DIR".to_string(), "/run".to_string()),
                (
                    "MIRADA_GREETER_BIN".to_string(),
                    "/usr/local/bin/mirada-greeter".to_string(),
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
                assert_eq!(exec, "/usr/local/bin/mirada-compositor");
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

    #[test]
    fn overlay_detecta_paths_del_host_y_arma_el_stack() {
        use std::fs;
        // Raíz falsa con binarios "instalados" en rutas típicas.
        let tmp = std::env::temp_dir().join(format!("arje-absorb-overlay-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        for p in [
            "usr/bin/udevd",
            "usr/bin/udevadm",
            "usr/bin/seatd",
            "sbin/agetty",
            "usr/sbin/dhcpcd",
        ] {
            let f = tmp.join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(&f, b"").unwrap();
        }
        let overlay = graphical_overlay(&tmp);
        let names: Vec<&str> = overlay.iter().map(|c| c.label.as_str()).collect();
        for must in ["udevd", "seatd", "dhcpcd", "arje-splash", "carmen-dm", "agetty-rescue"] {
            assert!(names.contains(&must), "falta {must} en {names:?}");
        }
        // El exec es el path ABSOLUTO del sistema destino, SIN el prefijo root.
        let udevd = overlay.iter().find(|c| c.label == "udevd").unwrap();
        match &udevd.payload {
            Payload::Native { exec, .. } => assert_eq!(exec, "/usr/bin/udevd"),
            _ => panic!("udevd debe ser Native"),
        }
        // Una semilla absorbida + overlay debe validar.
        let mut seed = build_seed("arje.seed.host", &[svc("dbus", ServiceKind::Daemon)]);
        seed.genesis.extend(overlay);
        seed.validate().expect("seed con overlay debe validar");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn overlay_sin_binarios_igual_trae_splash_y_compositor() {
        // En una raíz sin udev/seatd/getty, el overlay igual aporta el piso
        // gráfico (splash + carmen), que son paths tawasuyu, no del host.
        let tmp = std::env::temp_dir().join(format!("arje-absorb-empty-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let names: Vec<String> =
            graphical_overlay(&tmp).iter().map(|c| c.label.clone()).collect();
        assert!(names.contains(&"arje-splash".to_string()), "{names:?}");
        assert!(names.contains(&"carmen-dm".to_string()), "{names:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn overlay_con_networkmanager_levanta_dbus_y_gana_a_dhcpcd() {
        use std::fs;
        let tmp = std::env::temp_dir().join(format!("arje-absorb-nm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        for p in ["usr/bin/NetworkManager", "usr/bin/dbus-daemon", "usr/sbin/dhcpcd"] {
            let f = tmp.join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(&f, b"").unwrap();
        }
        let names: Vec<String> =
            graphical_overlay(&tmp).iter().map(|c| c.label.clone()).collect();
        // WiFi por NM necesita el bus de sistema arriba primero.
        assert!(names.contains(&"dbus-system".to_string()), "falta dbus: {names:?}");
        assert!(names.contains(&"networkmanager".to_string()), "{names:?}");
        // NM (que maneja WiFi con credenciales) gana sobre dhcpcd (sólo cableado).
        assert!(!names.contains(&"dhcpcd".to_string()), "NM presente → sin dhcpcd: {names:?}");
        let _ = fs::remove_dir_all(&tmp);
    }
}
