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
