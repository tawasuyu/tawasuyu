//! `arje-applaunch` — el puente de lanzamiento entre el menú de apps
//! (`app-bus`) y el orquestador (`arje`).
//!
//! Hoy los frontends de escritorio lanzan apps con `std::process::Command`
//! directo: pata desde `Msg::LaunchApp` (`AppEntry::spawn`) y shuma desde su
//! barra (`spawn_exec`). Eso funciona, pero deja la app **fuera** del plano de
//! control de arje aunque el orquestador esté levantado: sin supervisión, sin
//! cuotas, sin telemetría, sin aparecer en el `list` del Engine.
//!
//! Este crate centraliza el mismo patrón **opt-in + fallback** que ya tiene el
//! session-manager de mirada (`mirada-compositor::session`):
//!
//!   - **Si arje está levantado** (`ENTE_BUS_SOCK` presente): la app se entrega
//!     como Ente vía [`arje_bus::BusRequest::RunCard`]. Entra al grafo, la ve
//!     `sandokan list`, cuenta para cuotas y telemetría.
//!   - **Si no hay bus** (dev / nested / arje caído) o **arje la rechaza**
//!     (p.ej. el caller no tiene `Capability::Spawn`): cae al spawn crudo del
//!     host, idéntico al comportamiento previo. El usuario nunca queda sin app.
//!
//! Diferencia con el session-manager de mirada: las apps que el usuario lanza a
//! mano son [`Supervision::OneShot`] (si salen, salieron — no se resucitan) y
//! **no** dependen del piso `wayland_floor()` (el compositor ya está arriba
//! cuando hay un menú donde clickear). Las de sesión sí: `Restart` + re-floor.

#![forbid(unsafe_code)]

use app_bus::{AppEntry, Launch};
use arje_bus::{BusClient, BusRequest, BusResponse, ENV_BUS_SOCK};
use arje_card::{EntityCard, FsPolicy, NetworkingPolicy, Payload, Supervision, WireCard};

/// Cómo terminó un intento de lanzamiento. Informativo (para log / tests); los
/// call sites suelen ignorarlo — lo importante es que *algo* arrancó.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Aceptada por arje como Ente supervisado (`RunCard` → `Ok`).
    ViaEnte,
    /// Lanzada como proceso crudo del host (no había bus, o arje la rechazó).
    ViaProcess,
    /// El modo de la app no lo resuelve este puente (`Action`/`Wasm`): los
    /// despacha el chasis/kernel, no un proceso del SO.
    Unsupported,
    /// Ni el Ente ni el spawn crudo pudieron arrancar la app.
    Failed(String),
}

/// ¿Hay un orquestador arje al que hablarle? Detecta el socket del bus
/// (`ENTE_BUS_SOCK`), que es lo que el Init exporta a sus hijos. Su presencia
/// **es** el opt-in: si arje no levantó, el socket no está y vamos a crudo.
pub fn orchestrator_present() -> bool {
    std::env::var_os(ENV_BUS_SOCK).is_some()
}

/// Entorno actual del frontend, para que el Ente herede exactamente lo que
/// heredaría un `Command::spawn` (WAYLAND_DISPLAY, XDG_RUNTIME_DIR, tema…).
/// arje encarna con *este* envp; sin él, el hijo perdería la sesión gráfica.
fn current_env() -> Vec<(String, String)> {
    std::env::vars().collect()
}

/// Card de una app de usuario: `OneShot`, con red/FS/sub-procesos (lo que una
/// app de escritorio necesita) y el entorno del frontend. Sin `requires` (el
/// piso ya está) y sin `run_as` (el frontend ya corre como el usuario).
fn exec_card(label: &str, program: &str, argv: Vec<String>) -> EntityCard {
    let mut card = EntityCard::new(format!("app.{label}"));
    card.supervision = Supervision::OneShot;
    card.permissions.networking = NetworkingPolicy::Full;
    card.permissions.filesystem = FsPolicy::ReadWrite;
    card.permissions.processes = true;
    card.payload = Payload::Native {
        exec: program.to_string(),
        argv,
        envp: current_env(),
    };
    card
}

/// Intenta `RunCard` contra arje (best-effort, síncrono — abre un runtime
/// current-thread porque los call sites son bucles Elm, no async). `true` si
/// arje la aceptó; `false` para que el caller caiga al spawn crudo.
fn try_run_card(card: EntityCard, label: &str) -> bool {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(_) => return false,
    };
    rt.block_on(async move {
        let mut client = match BusClient::from_env().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("arje-applaunch: sin bus para RunCard — sigo crudo: {e}");
                return false;
            }
        };
        let wire: WireCard = card.into();
        match client.call(BusRequest::RunCard { card: wire }).await {
            Ok(BusResponse::Ok) => {
                eprintln!("arje-applaunch: «{label}» lanzado como Ente OneShot (en el grafo de arje)");
                true
            }
            Ok(other) => {
                eprintln!("arje-applaunch: arje rechazó RunCard de «{label}» — sigo crudo: {other:?}");
                false
            }
            Err(e) => {
                eprintln!("arje-applaunch: RunCard de «{label}» falló — sigo crudo: {e}");
                false
            }
        }
    })
}

/// Lanza una [`AppEntry`] del registro. Si arje está levantado, como Ente
/// OneShot supervisado; si no (o si arje la rechaza), spawn crudo vía app-bus.
/// `Action`/`Wasm` devuelven [`Outcome::Unsupported`] — los despacha el chasis.
///
/// Reemplaza el `let _ = app.spawn();` que tenían los call sites: misma garantía
/// de arranque, pero coordinado con el orquestador cuando lo hay.
pub fn launch_entry(entry: &AppEntry) -> Outcome {
    let Launch::Exec { program, args } = &entry.launch else {
        return Outcome::Unsupported;
    };
    if orchestrator_present()
        && try_run_card(exec_card(&entry.id, program, args.clone()), &entry.id)
    {
        return Outcome::ViaEnte;
    }
    match entry.spawn() {
        Ok(Some(_child)) => Outcome::ViaProcess,
        Ok(None) => Outcome::Unsupported,
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

/// Como [`launch_entry`] pero **open-with**: abre `target` con la app (expande
/// `%f`/`%u`, semántica freedesktop, vía [`app_bus::expand_target`]).
pub fn open_entry(entry: &AppEntry, target: &str) -> Outcome {
    let Launch::Exec { program, args } = &entry.launch else {
        return Outcome::Unsupported;
    };
    if orchestrator_present() {
        let argv = app_bus::expand_target(args, target);
        if try_run_card(exec_card(&entry.id, program, argv), &entry.id) {
            return Outcome::ViaEnte;
        }
    }
    match entry.open(target) {
        Ok(Some(_child)) => Outcome::ViaProcess,
        Ok(None) => Outcome::Unsupported,
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

/// Lanza una **línea de comando cruda** (`programa arg arg…`) — la forma que usa
/// la barra de shuma, que no tiene `AppEntry` sino un `exec` libre. Parseo por
/// whitespace (sin quoting: un launcher invoca binarios, no scripts). Si arje
/// está levantado va por `RunCard`; si no, spawn crudo **detached**
/// (`process_group(0)` + stdio a `/dev/null`), preservando la semántica previa
/// de `shuma::spawn_exec`.
pub fn launch_exec_line(exec_line: &str) -> Outcome {
    let mut parts = exec_line.split_whitespace();
    let Some(program) = parts.next() else {
        return Outcome::Failed("línea de exec vacía".into());
    };
    let args: Vec<String> = parts.map(str::to_string).collect();

    if orchestrator_present()
        && try_run_card(exec_card(program, program, args.clone()), program)
    {
        return Outcome::ViaEnte;
    }
    spawn_detached(program, &args)
}

/// Spawn crudo detached del frontend: nuevo grupo de proceso y stdio mudo, para
/// que cerrar/relanzar el launcher no se lleve a la app ni le ensucie la consola.
fn spawn_detached(program: &str, args: &[String]) -> Outcome {
    use std::os::unix::process::CommandExt;
    match std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
    {
        Ok(_) => Outcome::ViaProcess,
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec_entry(id: &str, program: &str) -> AppEntry {
        AppEntry {
            id: id.into(),
            label: id.into(),
            icon: None,
            category: None,
            launch: Launch::Exec { program: program.into(), args: vec![] },
            handles: vec![],
        }
    }

    #[test]
    fn la_card_de_usuario_es_oneshot_sin_piso() {
        let card = exec_card("cosmos", "cosmos-app-llimphi", vec![]);
        // OneShot: una app que el usuario lanzó y salió, salió (no se resucita).
        assert!(matches!(card.supervision, Supervision::OneShot));
        // No depende del piso: el compositor ya está arriba al haber menú.
        assert!(card.requires.is_empty());
        // Corre como el frontend (sin drop de privilegios).
        assert!(card.soma.run_as.is_none());
        // El label la namespacea bajo `app.*` en el grafo.
        assert_eq!(card.label, "app.cosmos");
        card.validate().unwrap();
    }

    #[test]
    fn la_card_lleva_el_binario_y_el_entorno() {
        let card = exec_card("nada", "nada", vec!["x.txt".into()]);
        match &card.payload {
            Payload::Native { exec, argv, envp } => {
                assert_eq!(exec, "nada");
                assert_eq!(argv, &vec!["x.txt".to_string()]);
                // Hereda el entorno del frontend (no vacío en un proceso real).
                assert!(envp.iter().any(|(k, _)| k == "PATH") || envp.is_empty());
            }
            other => panic!("payload no es Native: {other:?}"),
        }
    }

    #[test]
    fn action_y_wasm_no_los_resuelve_el_puente() {
        let mut a = exec_entry("shell", "x");
        a.launch = Launch::Action("focus:shell".into());
        assert_eq!(launch_entry(&a), Outcome::Unsupported);

        let mut w = exec_entry("hola", "x");
        w.launch = Launch::Wasm {
            bytecode_hex: "deadbeef".into(),
            grant_hex: None,
        };
        assert_eq!(launch_entry(&w), Outcome::Unsupported);
    }

    #[test]
    fn open_expande_el_target_en_la_card() {
        // Sin bus, open_entry cae a crudo; acá sólo verificamos que la card que
        // se construiría lleva el target expandido en argv.
        let argv = app_bus::expand_target(&["%f".to_string()], "/tmp/x.png");
        let card = exec_card("tullpu", "tullpu-app-llimphi", argv);
        match &card.payload {
            Payload::Native { argv, .. } => assert_eq!(argv, &vec!["/tmp/x.png".to_string()]),
            other => panic!("payload no es Native: {other:?}"),
        }
    }
}
