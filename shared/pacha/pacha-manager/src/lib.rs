//! `pacha-manager` — el **activador** de contextos de usuario.
//!
//! `pacha-core` decide *qué* efectos hay que emitir al cambiar de contexto;
//! este crate los **ejecuta** contra las superficies reales y mantiene el
//! estado vivo persistido. El reparto:
//!
//! * [`Surfaces`] — el trait que abstrae los efectos de borde (escribir el
//!   overlay de `wawa-config`, llamar `mirada-ctl`, encarnar apps por el
//!   `Engine` de sandokan, manipular cgroups). Mockeable → la lógica de
//!   activación se testea sin tocar disco ni levantar mirada/sandokan.
//! * [`linux::LinuxSurfaces`] — la implementación real sobre Linux.
//! * [`Manager`] — junta `Catalog` + `Runtime` + una `Surfaces` y orquesta
//!   `switch`/`close` aplicando los `Effect`.
//! * [`proto`] — protocolo de socket; [`server`]/cliente para que CLI y UI
//!   manejen el daemon.

#![forbid(unsafe_code)]

pub mod linux;
pub mod paths;
pub mod proto;
pub mod server;

use std::collections::BTreeMap;

use async_trait::async_trait;
use pacha_core::{AppSpec, BringUp, Catalog, Effect, Runtime, WawaOverlay};
use thiserror::Error;

pub use pacha_core;

/// Las superficies del sistema que un contexto activa/desactiva. Cada método
/// es un efecto de borde concreto; `pacha-core` los encadena. Los errores se
/// aplanan a `String` para no acoplar el trait a los tipos de cada superficie.
#[async_trait]
pub trait Surfaces: Send {
    /// Escribe el overlay de config del SO (`context.json`).
    async fn write_overlay(&mut self, overlay: &WawaOverlay) -> Result<(), String>;
    /// Borra el overlay → la config vuelve a la base de usuario.
    async fn clear_overlay(&mut self) -> Result<(), String>;
    /// Aplica la vista/keymap del compositor.
    async fn apply_vista(&mut self, vista: &str) -> Result<(), String>;
    /// Encarna una app de la receta bajo `slice` y mueve su ventana al
    /// special-workspace `special`. Devuelve el id de la unidad encarnada.
    async fn spawn(&mut self, spec: &AppSpec, slice: &str, special: &str) -> Result<String, String>;
    /// Reabre una app persistida por su `app_id` (mismo destino que `spawn`).
    async fn respawn(&mut self, app_id: &str, slice: &str, special: &str) -> Result<String, String>;
    /// Oculta las ventanas del contexto (toggle-special para esconder).
    async fn hide_windows(&mut self, special: &str) -> Result<(), String>;
    /// Muestra las ventanas del contexto.
    async fn show_windows(&mut self, special: &str) -> Result<(), String>;
    /// Reescribe `cpu.weight` del slice (reweight en caliente).
    async fn set_cpu_weight(&mut self, slice: &str, weight: u32) -> Result<(), String>;
    /// Congela (`true`) o descongela (`false`) el slice.
    async fn freeze(&mut self, slice: &str, frozen: bool) -> Result<(), String>;
    /// Para las unidades del contexto (close).
    async fn stop_units(&mut self, units: &[String]) -> Result<(), String>;
    /// Captura los `app_id` vivos del special-workspace (para `last_session`).
    async fn snapshot_apps(&mut self, special: &str) -> Result<Vec<String>, String>;
    /// Materializa en `$HOME` la instantánea `raiz` de un set de dotfiles.
    async fn materialize_dotfiles(&mut self, set_id: &str, raiz: [u8; 32]) -> Result<(), String>;
    /// Recaptura un set de dotfiles desde `$HOME` y devuelve la nueva
    /// instantánea (hash del árbol raíz) para avanzar el pin del runtime.
    async fn capture_dotfiles(&mut self, set_id: &str) -> Result<[u8; 32], String>;
}

/// Errores del activador. Sólo los **duros** (la planificación del core);
/// los fallos de efectos de borde son best-effort y se devuelven como
/// warnings desde [`Manager::switch`]/[`Manager::close`], no como error.
#[derive(Debug, Error)]
pub enum ManagerError {
    #[error(transparent)]
    Core(#[from] pacha_core::PachaError),
}

/// El activador: definiciones + estado vivo + las superficies que toca.
pub struct Manager<S: Surfaces> {
    pub catalog: Catalog,
    pub runtime: Runtime,
    surf: S,
}

impl<S: Surfaces> Manager<S> {
    pub fn new(catalog: Catalog, runtime: Runtime, surf: S) -> Self {
        Self { catalog, runtime, surf }
    }

    /// Acceso de sólo lectura a las superficies (para impls que exponen
    /// estado, p. ej. tests).
    pub fn surfaces(&self) -> &S {
        &self.surf
    }

    /// Cambia el foco al contexto `to`. Planea la transición con `pacha-core`
    /// y ejecuta los efectos. `bring` controla si se respeta `last_session`.
    /// Devuelve la lista de **warnings** de efectos best-effort que fallaron
    /// (cgroup sin delegación, compositor ausente…) sin abortar el cambio.
    pub async fn switch(&mut self, to: &str, bring: BringUp) -> Result<Vec<String>, ManagerError> {
        let fx = self.runtime.plan_switch(&self.catalog, to, bring)?;
        Ok(self.apply(fx).await)
    }

    /// Cierra explícitamente un contexto (libera sus recursos) sin cambiar el
    /// foco. Devuelve warnings de efectos best-effort.
    pub async fn close(&mut self, id: &str) -> Result<Vec<String>, ManagerError> {
        let fx = self.runtime.plan_close(&self.catalog, id)?;
        Ok(self.apply(fx).await)
    }

    /// Ejecuta una lista de efectos contra las superficies. Realimenta al
    /// `Runtime` lo que sólo se sabe tras el efecto: ids de unidades
    /// encarnadas y snapshots de apps.
    ///
    /// **Resiliencia:** un efecto de borde que falla NO aborta la transición
    /// — el estado del contexto ya cambió en el core, y un fallo de
    /// orquestación (p. ej. cgroup no delegado, mirada headless) no debe
    /// dejar al usuario sin poder cambiar de contexto. Cada fallo se acumula
    /// como warning y se sigue con el resto. Devuelve los warnings.
    async fn apply(&mut self, fx: Vec<Effect>) -> Vec<String> {
        // Unidades encarnadas en esta tanda, agrupadas por slice → contexto.
        let mut spawned: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut warnings: Vec<String> = Vec::new();
        // Destructuramos para tomar prestados `surf` y `runtime` a la vez.
        let Self { surf, runtime, .. } = self;

        /// Acumula un warning con el nombre del efecto si el resultado falló.
        macro_rules! best_effort {
            ($efecto:literal, $call:expr) => {
                if let Err(causa) = $call {
                    let w = format!("{}: {causa}", $efecto);
                    tracing::warn!("efecto best-effort falló — {w}");
                    warnings.push(w);
                }
            };
        }

        for e in fx {
            match e {
                Effect::WriteOverlay { overlay } => {
                    best_effort!("write_overlay", surf.write_overlay(&overlay).await);
                }
                Effect::ClearOverlay => {
                    best_effort!("clear_overlay", surf.clear_overlay().await);
                }
                Effect::ApplyVista { vista } => {
                    best_effort!("apply_vista", surf.apply_vista(&vista).await);
                }
                Effect::SetCpuWeight { slice, cpu_weight } => {
                    best_effort!("set_cpu_weight", surf.set_cpu_weight(&slice, cpu_weight).await);
                }
                Effect::Freeze { slice } => {
                    best_effort!("freeze", surf.freeze(&slice, true).await);
                }
                Effect::Unfreeze { slice } => {
                    best_effort!("unfreeze", surf.freeze(&slice, false).await);
                }
                Effect::HideWindows { special } => {
                    best_effort!("hide_windows", surf.hide_windows(&special).await);
                }
                Effect::ShowWindows { special } => {
                    best_effort!("show_windows", surf.show_windows(&special).await);
                }
                Effect::StopUnits { units } => {
                    best_effort!("stop_units", surf.stop_units(&units).await);
                }
                Effect::SnapshotApps { pacha, special } => match surf.snapshot_apps(&special).await {
                    Ok(ids) => runtime.set_last_session(&pacha, ids),
                    Err(causa) => warnings.push(format!("snapshot_apps: {causa}")),
                },
                Effect::MaterializarDotfiles { set_id, raiz } => {
                    best_effort!("materializar_dotfiles", surf.materialize_dotfiles(&set_id, raiz).await);
                }
                Effect::CapturarDotfiles { pacha, set_id } => {
                    match surf.capture_dotfiles(&set_id).await {
                        Ok(raiz) => runtime.set_dotfile_pin(&pacha, &set_id, raiz),
                        Err(causa) => warnings.push(format!("capturar_dotfiles {set_id}: {causa}")),
                    }
                }
                Effect::SpawnApp { spec, slice, special } => {
                    match surf.spawn(&spec, &slice, &special).await {
                        Ok(unit) => spawned.entry(slice).or_default().push(unit),
                        Err(causa) => warnings.push(format!("spawn {}: {causa}", spec.app_id)),
                    }
                }
                Effect::RespawnApp { app_id, slice, special } => {
                    match surf.respawn(&app_id, &slice, &special).await {
                        Ok(unit) => spawned.entry(slice).or_default().push(unit),
                        Err(causa) => warnings.push(format!("respawn {app_id}: {causa}")),
                    }
                }
            }
        }

        // Una tanda de arranque reemplaza el set de unidades del contexto
        // entrante (no acumula sobre arranques previos: el viejo set ya se
        // detuvo o sigue vivo en background sin pasar por acá).
        for (slice, units) in spawned {
            if let Some(id) = pacha_core::id_from_slice(&slice) {
                runtime.set_units(id, units);
            }
        }
        warnings
    }
}

// =====================================================================
// Tests — Manager sobre una superficie que graba la secuencia de efectos
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use pacha_core::{OnLeave, Pacha};

    /// Superficie de prueba: graba cada llamada como una línea de texto y
    /// devuelve datos canned. Permite asertar la **secuencia exacta** de
    /// efectos sin mirada/sandokan reales.
    #[derive(Default)]
    struct Recorder {
        log: Vec<String>,
        /// app_ids que `snapshot_apps` devuelve (simula ventanas vivas).
        snapshot: Vec<String>,
        next_unit: usize,
    }

    #[async_trait]
    impl Surfaces for Recorder {
        async fn write_overlay(&mut self, ov: &WawaOverlay) -> Result<(), String> {
            self.log.push(format!("write_overlay theme={:?}", ov.theme_variant));
            Ok(())
        }
        async fn clear_overlay(&mut self) -> Result<(), String> {
            self.log.push("clear_overlay".into());
            Ok(())
        }
        async fn apply_vista(&mut self, v: &str) -> Result<(), String> {
            self.log.push(format!("apply_vista {v}"));
            Ok(())
        }
        async fn spawn(&mut self, spec: &AppSpec, slice: &str, special: &str) -> Result<String, String> {
            self.next_unit += 1;
            let unit = format!("unit-{}", self.next_unit);
            self.log.push(format!("spawn {} @{slice} →{special} = {unit}", spec.app_id));
            Ok(unit)
        }
        async fn respawn(&mut self, app_id: &str, slice: &str, special: &str) -> Result<String, String> {
            self.next_unit += 1;
            let unit = format!("unit-{}", self.next_unit);
            self.log.push(format!("respawn {app_id} @{slice} →{special} = {unit}"));
            Ok(unit)
        }
        async fn hide_windows(&mut self, s: &str) -> Result<(), String> {
            self.log.push(format!("hide {s}"));
            Ok(())
        }
        async fn show_windows(&mut self, s: &str) -> Result<(), String> {
            self.log.push(format!("show {s}"));
            Ok(())
        }
        async fn set_cpu_weight(&mut self, slice: &str, w: u32) -> Result<(), String> {
            self.log.push(format!("weight {slice}={w}"));
            Ok(())
        }
        async fn freeze(&mut self, slice: &str, frozen: bool) -> Result<(), String> {
            self.log.push(format!("freeze {slice}={frozen}"));
            Ok(())
        }
        async fn stop_units(&mut self, units: &[String]) -> Result<(), String> {
            self.log.push(format!("stop {units:?}"));
            Ok(())
        }
        async fn snapshot_apps(&mut self, special: &str) -> Result<Vec<String>, String> {
            self.log.push(format!("snapshot {special}"));
            Ok(self.snapshot.clone())
        }
        async fn materialize_dotfiles(&mut self, set_id: &str, raiz: [u8; 32]) -> Result<(), String> {
            self.log.push(format!("materializar {set_id} raiz={:02x}{:02x}", raiz[0], raiz[1]));
            Ok(())
        }
        async fn capture_dotfiles(&mut self, set_id: &str) -> Result<[u8; 32], String> {
            self.log.push(format!("capturar {set_id}"));
            Ok([0xAB; 32])
        }
    }

    fn cat() -> Catalog {
        let mut c = Catalog::new();
        let mut oficina = Pacha::new("oficina", "Oficina");
        oficina.on_leave = OnLeave::Background;
        oficina.overlay = Some(WawaOverlay { theme_variant: Some("light".into()), ..Default::default() });
        oficina.vista = Some("kde".into());
        oficina.apps = vec![AppSpec::new("puriy --profile oficina", "puriy")];
        c.upsert(oficina);
        let mut juegos = Pacha::new("juegos", "Juegos");
        juegos.on_leave = OnLeave::Close;
        juegos.resources.cpu_weight = Some(10000);
        juegos.apps = vec![AppSpec::new("steam", "steam")];
        c.upsert(juegos);
        c
    }

    #[tokio::test]
    async fn primer_switch_ejecuta_overlay_vista_spawn_y_registra_unidad() {
        let mut m = Manager::new(cat(), Runtime::new(), Recorder::default());
        m.switch("oficina", BringUp::Restore).await.unwrap();
        let log = &m.surfaces().log;
        assert_eq!(log[0], "write_overlay theme=Some(\"light\")");
        assert_eq!(log[1], "apply_vista kde");
        assert_eq!(log[2], "spawn puriy @pacha-oficina.slice →pacha-oficina = unit-1");
        assert_eq!(log[3], "weight pacha-oficina.slice=100");
        // La unidad encarnada quedó registrada en el runtime del contexto.
        assert_eq!(m.runtime.state("oficina").unwrap().units, vec!["unit-1"]);
    }

    #[tokio::test]
    async fn switch_con_saliente_background_baja_peso_oculta_y_trae_al_nuevo() {
        let mut m = Manager::new(cat(), Runtime::new(), Recorder::default());
        m.switch("oficina", BringUp::Restore).await.unwrap();
        m.switch("juegos", BringUp::Restore).await.unwrap();
        let log = &m.surfaces().log;
        // tras el primer switch: dejar oficina (bg) = weight 10 + hide.
        let leave = log.iter().position(|l| l == "weight pacha-oficina.slice=10").unwrap();
        assert_eq!(log[leave + 1], "hide pacha-oficina");
        // juegos sin overlay → clear; peso activo 10000.
        assert!(log.contains(&"clear_overlay".to_string()));
        assert!(log.contains(&"weight pacha-juegos.slice=10000".to_string()));
    }

    #[tokio::test]
    async fn close_con_persist_snapshotea_y_para_unidades() {
        let mut c = cat();
        let mut juegos = c.get("juegos").unwrap().clone();
        juegos.persist = true;
        c.upsert(juegos);

        let mut rec = Recorder::default();
        rec.snapshot = vec!["steam".into(), "discord".into()];
        let mut m = Manager::new(c, Runtime::new(), rec);

        m.switch("juegos", BringUp::Restore).await.unwrap();
        // Salir a oficina dispara el Close de juegos (persist): snapshot+stop.
        m.switch("oficina", BringUp::Restore).await.unwrap();
        let log = &m.surfaces().log;
        assert!(log.contains(&"snapshot pacha-juegos".to_string()));
        assert!(log.iter().any(|l| l.starts_with("stop [\"unit-")));
        // El snapshot se guardó como last_session del contexto.
        assert_eq!(
            m.runtime.state("juegos").unwrap().last_session,
            vec!["steam".to_string(), "discord".to_string()]
        );
    }

    #[tokio::test]
    async fn dotfiles_materializa_al_entrar_y_avanza_el_pin_al_salir() {
        let mut c = cat();
        let mut oficina = c.get("oficina").unwrap().clone();
        oficina.dotfiles = vec![pacha_core::DotfileRef {
            set_id: "shell".into(),
            instantanea: [3u8; 32],
            rastrear: true,
        }];
        c.upsert(oficina);
        let mut m = Manager::new(c, Runtime::new(), Recorder::default());

        m.switch("oficina", BringUp::Restore).await.unwrap();
        assert!(m.surfaces().log.iter().any(|l| l == "materializar shell raiz=0303"));

        // Salir a juegos recaptura el set rastreado y el pin del runtime avanza
        // a la instantánea que devolvió la superficie ([0xAB; 32]).
        m.switch("juegos", BringUp::Restore).await.unwrap();
        assert!(m.surfaces().log.iter().any(|l| l == "capturar shell"));
        assert_eq!(
            m.runtime.state("oficina").unwrap().dotfile_pins.get("shell"),
            Some(&[0xABu8; 32])
        );
    }

    #[tokio::test]
    async fn volver_de_background_muestra_sin_respawnear() {
        let mut m = Manager::new(cat(), Runtime::new(), Recorder::default());
        m.switch("oficina", BringUp::Restore).await.unwrap();
        m.switch("juegos", BringUp::Restore).await.unwrap();
        let before = m.surfaces().log.len();
        m.switch("oficina", BringUp::Restore).await.unwrap();
        let tail = &m.surfaces().log[before..];
        assert!(tail.iter().any(|l| l == "show pacha-oficina"));
        assert!(tail.iter().any(|l| l == "weight pacha-oficina.slice=100"));
        assert!(!tail.iter().any(|l| l.starts_with("spawn ")));
    }
}
