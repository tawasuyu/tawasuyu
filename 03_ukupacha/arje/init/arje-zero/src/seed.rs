//! Construcción de la Tarjeta Semilla.
//!
//! Tres caminos:
//!   1. `--restore <path>`: leer `FractalSnapshot` y reconstruir Semilla
//!      con seed_id preservado + entes anteriores como genesis.
//!   2. `seed.card.json` en disco: deserialize directo (prod o dev).
//!   3. Fallback dev: sintetizar Semilla + 6 genesis Entes que ejercitan
//!      todas las capacidades del fractal.

use anyhow::Context;
use arje_card::{
    Capability, CardError, CgroupSpec, EntityCard, NamespaceSet, Payload,
    ResourceLimits, SomaSpec, Supervision, CARD_SCHEMA_VERSION,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{info, warn};
use ulid::Ulid;

const SEED_PATH_PROD: &str = "/ente/seed.card";
const SEED_PATH_DEV: &str = "seed.card";

pub fn load(dev_mode: bool, restore: Option<&Path>) -> anyhow::Result<EntityCard> {
    let card = if let Some(path) = restore {
        load_from_snapshot(path)?
    } else {
        load_or_synthesize(dev_mode)?
    };
    card.validate()
        .map_err(|e: CardError| anyhow::anyhow!("semilla inválida: {e}"))?;
    Ok(card)
}

fn load_from_snapshot(path: &Path) -> anyhow::Result<EntityCard> {
    let snap = arje_snapshot::FractalSnapshot::read(path)
        .with_context(|| format!("read snapshot {}", path.display()))?;
    info!(
        path = %path.display(),
        seed_id = %snap.seed_id,
        entes = snap.entes.len(),
        timestamp_ms = snap.timestamp_ms,
        "snapshot cargado, restaurando fractal"
    );
    // Reconstruimos la Semilla con su Ulid original. Las Cards persistidas
    // van a `genesis` con sus Ulids preservados — son las mismas identidades
    // que vivieron antes del checkpoint.
    let mut provides = BTreeSet::new();
    provides.insert(Capability::Spawn);
    provides.insert(Capability::Journal);
    Ok(EntityCard {
        schema_version: CARD_SCHEMA_VERSION,
        id: snap.seed_id,
        lineage: None,
        label: snap.seed_label,
        provides,
        requires: BTreeSet::new(),
        soma: SomaSpec::default(),
        payload: Payload::Virtual,
        supervision: Supervision::OneShot,
        genesis: snap.entes,
        ..Default::default()
    })
}

fn load_or_synthesize(dev_mode: bool) -> anyhow::Result<EntityCard> {
    // Buscamos primero `.json` (canónico), luego sin extensión por
    // compatibilidad con instalaciones que dejan el archivo crudo. La puerta
    // genética se cruza vía `arje_brain::load_card_file` que pasa por
    // `validate()` extendido.
    let candidates: &[&str] = if dev_mode {
        &["seed.card.json", SEED_PATH_DEV]
    } else {
        &["/ente/seed.card.json", SEED_PATH_PROD]
    };
    for cand in candidates {
        let path = PathBuf::from(cand);
        if !path.exists() { continue; }
        let card = arje_brain::load_card_file(&path)
            .with_context(|| format!("load {}", path.display()))?;
        info!(path = %path.display(), "Tarjeta Semilla cargada y validada");
        return Ok(card);
    }
    if dev_mode {
        info!("sin seed.card — sintetizando semilla mínima (dev)");
        return Ok(synthesize_dev_seed());
    }
    anyhow::bail!("seed.card no encontrada en /ente/seed.card.json ni /ente/seed.card")
}

fn synthesize_dev_seed() -> EntityCard {
    let mut provides = BTreeSet::new();
    provides.insert(Capability::Spawn);
    provides.insert(Capability::Journal);

    // Pre-registramos el módulo Wasm demo en el CAS y obtenemos su SHA real.
    // Si el CAS no es escribible (raro en dev) caemos a un SHA cero — la
    // resolución fallará y el Wasm no encarnará, pero el resto queda intacto.
    let demo_wasm_sha = match arje_wasm::demo_module_bytes()
        .and_then(|b| arje_cas::store(&b))
    {
        Ok(sha) => sha,
        Err(e) => {
            warn!(?e, "CAS no disponible — demo-wasm no encarnará");
            [0u8; 32]
        }
    };

    let mut genesis = Vec::new();
    genesis.push(make_card("demo-sleep", Payload::Native {
        exec: "/bin/sleep".into(), argv: vec!["1".into()], envp: vec![],
    }, Supervision::OneShot));

    genesis.push(make_card("demo-persist", Payload::Native {
        exec: "/bin/sleep".into(), argv: vec!["60".into()], envp: vec![],
    }, restart_supervision()));

    // Card namespaced: padre escribe uid_map, hijo cat /proc/self/uid_map.
    let mut ns_card = make_card("demo-userns", Payload::Native {
        exec: "/bin/cat".into(),
        argv: vec!["/proc/self/uid_map".into()],
        envp: vec![],
    }, Supervision::OneShot);
    ns_card.soma = SomaSpec {
        namespaces: NamespaceSet { user: true, ..Default::default() },
        ..Default::default()
    };
    genesis.push(ns_card);

    genesis.push(make_card("demo-wasm", Payload::Wasm {
        module_sha256: demo_wasm_sha,
        entry: "_start".into(),
    }, Supervision::OneShot));

    if let Some(card) = optional_native_card(
        "demo-echo", "target/debug/ente-echo",
        [arje_echo::echo_capability()].into_iter().collect(),
        restart_supervision(),
    ) {
        genesis.push(card);
    }

    if let Some(card) = optional_native_card(
        "compat-logind", "target/debug/ente-logind-compat",
        [Capability::LegacyLogind].into_iter().collect(),
        restart_supervision(),
    ) {
        genesis.push(card);
    }

    // Constelación de shims D-Bus que reemplazan systemd: cada uno provee
    // un nombre `org.freedesktop.X1` que GNOME/KDE consultan al boot.
    for (label, bin) in &[
        ("compat-hostnamed", "target/debug/ente-hostnamed-compat"),
        ("compat-timedated", "target/debug/ente-timedated-compat"),
        ("compat-localed",   "target/debug/ente-localed-compat"),
        ("compat-journald",  "target/debug/ente-journald-compat"),
        ("compat-resolved",  "target/debug/ente-resolved-compat"),
        ("compat-polkit",    "target/debug/ente-polkit-compat"),
        ("compat-machined",  "target/debug/ente-machined-compat"),
        ("policy-provider",  "target/debug/ente-policy-provider"),
        ("compat-systemd1",  "target/debug/ente-systemd1-compat"),
        ("compat-notify",    "target/debug/ente-notify-compat"),
        ("compat-timer",     "target/debug/ente-timer-compat"),
    ] {
        if let Some(card) = optional_native_card(
            label, bin,
            std::collections::BTreeSet::new(),
            restart_supervision(),
        ) {
            genesis.push(card);
        }
    }

    // El splash nativo del arranque sin parpadeo (`arje-splash`): Ente de
    // prioridad alta que toma el DRM reusando el modo del loader y pinta el
    // splash animado hasta que mirada toma la pantalla. Va ANTES de mirada en
    // el orden de génesis para que pinte el primer frame gráfico. Best-effort:
    // si el binario no está o no hay GPU, el fractal arranca igual.
    if let Some(card) = arje_splash_card() {
        genesis.push(card);
    }

    // El compositor Wayland tawasuyu (`mirada-compositor --drm`) como Ente
    // supervisado por arje-zero. Si el binario no esta instalado en el
    // host, el fractal arranca sin compositor — util en CI o devs sin GPU.
    // Reemplaza al script `mirada-session` (queda como fallback legacy).
    if let Some(card) = mirada_session_card() {
        genesis.push(card);
    }

    EntityCard {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        lineage: None,
        label: "ente-zero-dev".into(),
        provides,
        requires: BTreeSet::new(),
        soma: SomaSpec {
            namespaces: NamespaceSet::default(),
            rlimits: ResourceLimits::default(),
            cgroup: CgroupSpec {
                path: "ente.slice/zero".into(),
                cpu_weight: None,
                io_weight: None,
            },
            cpu_affinity: None,
        },
        payload: Payload::Virtual,
        supervision: Supervision::OneShot,
        genesis,
        ..Default::default()
    }
}

fn make_card(label: &str, payload: Payload, supervision: Supervision) -> EntityCard {
    EntityCard {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        lineage: None,
        label: label.into(),
        provides: BTreeSet::new(),
        requires: BTreeSet::new(),
        soma: SomaSpec::default(),
        payload,
        supervision,
        genesis: vec![],
        ..Default::default()
    }
}

fn optional_native_card(
    label: &str,
    bin_path: &str,
    provides: BTreeSet<Capability>,
    supervision: Supervision,
) -> Option<EntityCard> {
    let path = Path::new(bin_path);
    if !path.exists() {
        return None;
    }
    Some(EntityCard {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        lineage: None,
        label: label.into(),
        provides,
        requires: BTreeSet::new(),
        soma: SomaSpec::default(),
        payload: Payload::Native {
            exec: path.to_string_lossy().into_owned(),
            argv: vec![],
            envp: vec![],
        },
        supervision,
        genesis: vec![],
        ..Default::default()
    })
}

fn restart_supervision() -> Supervision {
    Supervision::Restart {
        initial: Duration::from_millis(100),
        max: Duration::from_secs(30),
    }
}

/// Construye el Ente `mirada-session`: el compositor Wayland soberano
/// (`mirada-compositor --drm`) supervisado por arje-zero, con las
/// variables de entorno Wayland que el script `mirada-session` exportaba
/// inline. Devuelve `None` si el binario no está instalado — así el
/// fractal arranca igual en un host sin compositor (CI, devs sin GPU).
///
/// Reemplaza al script bash `02_ruway/mirada/mirada-compositor/session/mirada-session`
/// elevándolo a Ente del grafo. Beneficios:
///   - el ciclo init → compositor pasa por una sola autoridad (arje-zero
///     supervisa, reinicia con back-off, registra eventos en el bus);
///   - una crash del compositor relanza por el supervisor en lugar de
///     quedar la pantalla negra esperando a que el DM externo lo note;
///   - los envp se versionan con el resto del fractal en seed.card.json,
///     no en un script shell desperdigado en `/usr/local/bin`.
/// Construye el Ente `arje-splash`: el splash nativo del arranque sin
/// parpadeo (`SDD-ARRANQUE-SIN-PARPADEO.md`, Fase 1). Toma el nodo DRM
/// reusando el modo que dejó el GOP del loader (sin re-modeset → sin flash) y
/// pinta un splash animado hasta soltar la pantalla para mirada.
///
/// Va declarado **antes** que `mirada_session_card()` en el génesis para que
/// pinte el primer frame gráfico. `OneShot`: es decorativo y se autotermina
/// (por SIGTERM o por su tope de tiempo), así que no debe reiniciarse con
/// back-off. Devuelve `None` si el binario no está instalado — el fractal
/// arranca igual sin splash (CI, dev sin GPU).
fn arje_splash_card() -> Option<EntityCard> {
    const SPLASH_BIN: &str = "/usr/local/bin/arje-splash";
    if !Path::new(SPLASH_BIN).exists() {
        return None;
    }
    // Abre /dev/dri/* directo (igual que mirada). Declaramos la capacidad para
    // que quien valide la card sepa que esta Ente toca DRM.
    let mut requires = BTreeSet::new();
    requires.insert(Capability::Device { class: arje_card::DeviceClass::Drm });
    Some(EntityCard {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        lineage: None,
        label: "arje-splash".into(),
        provides: BTreeSet::new(),
        requires,
        soma: SomaSpec::default(),
        payload: Payload::Native {
            exec: SPLASH_BIN.into(),
            argv: vec![],
            envp: vec![],
        },
        supervision: Supervision::OneShot,
        genesis: vec![],
        ..Default::default()
    })
}

fn mirada_session_card() -> Option<EntityCard> {
    const COMPOSITOR_BIN: &str = "/usr/local/bin/mirada-compositor";
    if !Path::new(COMPOSITOR_BIN).exists() {
        return None;
    }
    // El compositor necesita /dev/dri/* abierto (backend DRM directo,
    // sin pasar por logind). La capacidad se declara aqui — quien
    // valide la card en otro contexto sabra que esta Ente toma DRM.
    let mut requires = BTreeSet::new();
    requires.insert(Capability::Device { class: arje_card::DeviceClass::Drm });
    Some(EntityCard {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        lineage: None,
        label: "mirada-session".into(),
        provides: BTreeSet::new(),
        requires,
        soma: SomaSpec::default(),
        payload: Payload::Native {
            exec: COMPOSITOR_BIN.into(),
            argv: vec!["--drm".into()],
            // El mismo conjunto que el script `mirada-session` exportaba
            // antes. `XDG_CURRENT_DESKTOP=carmen` identifica al
            // compositor frente a las apps GUI (xdg-portal, gnome-keyring
            // y similares lo leen). Las `QT_QPA_PLATFORM` / `SDL_VIDEODRIVER` /
            // `MOZ_ENABLE_WAYLAND` empujan a los toolkits hacia su backend
            // Wayland nativo cuando lo tienen.
            envp: vec![
                ("XDG_SESSION_TYPE".into(), "wayland".into()),
                ("XDG_CURRENT_DESKTOP".into(), "carmen".into()),
                ("XDG_SESSION_DESKTOP".into(), "carmen".into()),
                ("MOZ_ENABLE_WAYLAND".into(), "1".into()),
                ("QT_QPA_PLATFORM".into(), "wayland;xcb".into()),
                ("SDL_VIDEODRIVER".into(), "wayland".into()),
                ("_JAVA_AWT_WM_NONREPARENTING".into(), "1".into()),
            ],
        },
        // Reiniciar con back-off: si el compositor cae, lo levantamos
        // automaticamente; el back-off cubre crashes en cascada por
        // bugs sin agotar el bus de eventos.
        supervision: restart_supervision(),
        genesis: vec![],
        ..Default::default()
    })
}
