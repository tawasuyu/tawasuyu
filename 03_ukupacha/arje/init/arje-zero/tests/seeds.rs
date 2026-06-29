//! Smoke tests sobre las semillas canónicas del repo.
//!
//! Las seeds viven en `03_ukupacha/arje/seeds/*.card.json`. Cada una
//! describe un *target de despliegue* — `arje-host` para hardware real,
//! `arje-qemu` para pruebas en QEMU sin GPU. Este test las parsea con
//! [`EntityCard::from_path`] (que también valida) para garantizar que
//! cualquier cambio futuro al schema o a las propias seeds se note en
//! CI antes de que llegue al hardware.
//!
//! Por qué viven aquí: arje-zero es el único consumidor del archivo
//! seed.card.json — el resto del fractal lo trata como input opaco.

use std::path::PathBuf;

use arje_card::EntityCard;

fn seeds_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR de arje-zero apunta a init/arje-zero/. Subimos
    // dos niveles a 03_ukupacha/arje/ y entramos a seeds/.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("seeds")
}

fn validate_seed(name: &str) {
    let path = seeds_dir().join(name);
    let card = EntityCard::from_path(&path)
        .unwrap_or_else(|e| panic!("seed {name} no parseó/validó: {e}"));
    assert!(!card.label.is_empty(), "seed {name} sin label");
    assert!(
        !card.genesis.is_empty(),
        "seed {name} sin entes en genesis — un host sin servicios no arranca a nada",
    );
}

#[test]
fn arje_host_seed_es_valida() {
    validate_seed("arje-host.card.json");
}

#[test]
fn arje_qemu_seed_es_valida() {
    validate_seed("arje-qemu.card.json");
}

#[test]
fn arje_tawasuyu_seed_es_valida() {
    // La seed de producción tawasuyu-sobre-hammer: génesis splash →
    // mirada-greeter (DM real con Mesa) + hammerd (lab) + getty de rescate.
    validate_seed("arje-tawasuyu.card.json");
}

#[test]
fn tawasuyu_seed_arranca_el_dm_real_no_el_getty_stub() {
    // El salto demo→producción: el génesis debe lanzar el DM REAL —
    // `mirada-compositor --drm --greeter` (el camino verificado en metal del
    // SDD-ARRANQUE-SIN-PARPADEO: el compositor toma el DRM tras el handoff del
    // splash y hospeda al greeter como cliente) — no el arje-getty-stub del demo.
    // El compositor PROVEE el piso (display Wayland) para los clientes de sesión.
    // Y el splash debe ir ANTES, con prioridad alta.
    let path = seeds_dir().join("arje-tawasuyu.card.json");
    let card = EntityCard::from_path(&path).unwrap();
    use arje_card::{wayland_floor, Payload, Priority};

    let dm = card
        .genesis
        .iter()
        .find(|c| c.label == "display-manager-mesa")
        .expect("la seed de producción debe traer el display-manager-mesa");
    match &dm.payload {
        Payload::Native { exec, argv, .. } => {
            assert!(
                exec.ends_with("mirada-compositor"),
                "el DM debe ser mirada-compositor (dueño del DRM, hospeda al greeter): {exec}",
            );
            assert!(
                argv.iter().any(|a| a == "--drm") && argv.iter().any(|a| a == "--greeter"),
                "el DM debe correr --drm --greeter (camino metal del SDD): {argv:?}",
            );
        }
        otro => panic!("payload del DM no es Native: {otro:?}"),
    }
    // El piso (display Wayland) NO se declara estático: mirada-compositor lo
    // ANUNCIA por readiness (UpdateCapabilities) cuando su socket ya escucha, así
    // los clientes de sesión arrancan recién entonces (sin carrera) y el re-floor
    // sólo dispara cuando el piso está de verdad listo. Ver `mirada::floor`.
    assert!(
        !dm.provides.contains(&wayland_floor()),
        "el piso debe anunciarse por readiness, no declararse estático en el seed",
    );

    let splash = card
        .genesis
        .iter()
        .find(|c| c.label == "arje-splash")
        .expect("la seed de producción debe traer el splash sin parpadeo");
    assert_eq!(
        splash.priority,
        Priority::High,
        "el splash debe ir con prioridad alta (antes que el resto)",
    );

    assert!(
        card.genesis.iter().all(|c| !matches!(
            &c.payload,
            Payload::Native { exec, .. } if exec.ends_with("arje-getty-stub")
        )),
        "la seed de PRODUCCIÓN no debe contener el arje-getty-stub del demo",
    );
}

#[test]
fn arje_tawasuyu_host_seed_es_valida() {
    // La seed para "vivir en arje sobre tu Linux actual": arje-zero como PID 1
    // sobre el root real (con su Mesa), génesis net → splash → mirada-compositor
    // (DM real) → getty de rescate. Paths /usr/local/* = los que instala
    // scripts/install-arje-init.sh.
    validate_seed("arje-tawasuyu-host.card.json");
}

#[test]
fn tawasuyu_host_seed_arranca_dm_real_con_seatd() {
    // El DM debe ser mirada-compositor --drm --greeter (igual que producción),
    // y como arje corre de PID 1 SIN logind, el compositor toma el seat vía
    // `seatd` (Ente aparte; el backend `builtin` de libseat CRASHEA con mirada).
    let path = seeds_dir().join("arje-tawasuyu-host.card.json");
    let card = EntityCard::from_path(&path).unwrap();
    use arje_card::{Payload, Priority};

    let dm = card
        .genesis
        .iter()
        .find(|c| c.label == "display-manager-mesa")
        .expect("la seed host debe traer el display-manager-mesa");
    match &dm.payload {
        Payload::Native { exec, argv, envp } => {
            assert!(
                exec.ends_with("mirada-compositor"),
                "el DM debe ser mirada-compositor: {exec}",
            );
            assert!(
                argv.iter().any(|a| a == "--drm") && argv.iter().any(|a| a == "--greeter"),
                "el DM debe correr --drm --greeter: {argv:?}",
            );
            assert!(
                envp.iter()
                    .any(|(k, v)| k == "LIBSEAT_BACKEND" && v == "seatd"),
                "el compositor debe tomar el seat vía seatd (builtin crashea): {envp:?}",
            );
        }
        otro => panic!("payload del DM no es Native: {otro:?}"),
    }

    // seatd debe estar declarado como Ente (provee el seat a libseat sin logind).
    assert!(
        card.genesis.iter().any(|c| c.label == "seatd"),
        "la seed debe traer un Ente seatd antes del compositor",
    );
    // udevd + coldplug: sin udevd los dispositivos no llevan ID_INPUT y libinput
    // no ve teclado/mouse bajo arje (no hay systemd-udevd corriendo).
    assert!(
        card.genesis.iter().any(|c| c.label == "udevd"),
        "la seed debe traer udevd (si no, libinput no encuentra el teclado)",
    );

    let splash = card
        .genesis
        .iter()
        .find(|c| c.label == "arje-splash")
        .expect("la seed host debe traer el splash (mirada temprana)");
    assert_eq!(
        splash.priority,
        Priority::High,
        "el splash debe ir con prioridad alta (antes del DM)",
    );

    // Un getty de rescate en tty2 (tty1 queda para el compositor gráfico).
    let getty = card
        .genesis
        .iter()
        .find(|c| c.label.starts_with("agetty"))
        .expect("la seed host debe traer un getty de rescate");
    match &getty.payload {
        Payload::Native { exec, argv, .. } => {
            assert!(exec.ends_with("agetty"), "el rescate no es agetty: {exec}");
            assert!(
                argv.iter().any(|a| a == "tty2"),
                "el getty de rescate va en tty2 (tty1 es del compositor): {argv:?}",
            );
        }
        otro => panic!("payload del getty no es Native: {otro:?}"),
    }
}

#[test]
fn host_seed_provee_spawn_y_journal() {
    let path = seeds_dir().join("arje-host.card.json");
    let card = EntityCard::from_path(&path).unwrap();
    use arje_card::Capability;
    assert!(
        card.provides.contains(&Capability::Spawn),
        "el host debe poder generar hijas (Spawn)",
    );
    assert!(
        card.provides.contains(&Capability::Journal),
        "el host debe ofrecer journal al fractal",
    );
}

#[test]
fn session_gnome_fragment_es_valido_y_trae_compat() {
    // El fragmento de sesión `gnome` se anexa a la base por `overlay_session`
    // cuando el cmdline trae `arje.session=gnome`. Debe parsear/validar igual
    // que una seed y aportar los shims D-Bus que GNOME consulta al boot.
    let path = seeds_dir()
        .join("fragments")
        .join("session-gnome.card.json");
    let card = EntityCard::from_path(&path)
        .unwrap_or_else(|e| panic!("fragmento session-gnome no parseó/validó: {e}"));
    assert!(
        !card.genesis.is_empty(),
        "session-gnome sin entes — no aportaría ningún backend a la sesión",
    );
    for clave in ["logind", "hostnamed", "polkit", "systemd1"] {
        assert!(
            card.genesis.iter().any(|c| c.label.contains(clave)),
            "session-gnome debe traer el shim '{clave}' (GNOME lo consulta al arrancar)",
        );
    }
}

#[test]
fn host_seed_lleva_un_getty_en_tty1() {
    // Sin un getty arrancable el host no acepta login interactivo.
    let path = seeds_dir().join("arje-host.card.json");
    let card = EntityCard::from_path(&path).unwrap();
    let getty = card
        .genesis
        .iter()
        .find(|c| c.label.starts_with("agetty"))
        .expect("la semilla host debe incluir un agetty");
    use arje_card::Payload;
    match &getty.payload {
        Payload::Native { exec, argv, .. } => {
            assert!(exec.ends_with("agetty"), "el exec no es agetty: {exec}");
            assert!(
                argv.iter().any(|a| a == "tty1"),
                "el getty no apunta a tty1: {argv:?}",
            );
        }
        otro => panic!("payload del getty no es Native: {otro:?}"),
    }
}
