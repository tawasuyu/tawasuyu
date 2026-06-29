//! `LinuxSurfaces` — la implementación real de [`Surfaces`] sobre Linux:
//!
//! * **config** → escribe/borra `context.json` (la capa Context de
//!   `wawa-config`); todas las apps Llimphi hot-reload.
//! * **compositor** → `mirada-ctl` (vista, move-to-special, toggle-special).
//! * **procesos + cgroups** → el `Engine` de sandokan (`run`/`stop` +
//!   `set_cpu_weight`/`freeze` sobre el slice del contexto).
//!
//! Las apps se encarnan con `Card.soma.cgroup.path = "<slice>/<app_id>"`, así
//! quedan bajo el subárbol cgroup del contexto y el reweight/freeze del slice
//! las gobierna a todas de una.

use async_trait::async_trait;
use card_core::{Card, Payload};
use pacha_core::{AppSpec, WawaOverlay};
use sandokan::{Engine, Intent};
use tokio::process::Command;
use ulid::Ulid;

use crate::Surfaces;

/// Superficies reales. Embebe el `Engine` (elegido por `sandokan::auto`: init
/// de sistema → daemon → in-process) y conoce el binario de `mirada-ctl`.
pub struct LinuxSurfaces {
    engine: Box<dyn Engine>,
    mirada_ctl: String,
}

impl LinuxSurfaces {
    /// Conecta al orquestador disponible y usa `mirada-ctl` del PATH.
    pub async fn connect() -> Self {
        let socket = sandokan::default_socket_path();
        Self { engine: sandokan::auto(&socket).await, mirada_ctl: "mirada-ctl".into() }
    }

    /// Igual que [`connect`](Self::connect) pero con un `Engine` ya construido
    /// (para tests de humo o engines remotos).
    pub fn with_engine(engine: Box<dyn Engine>) -> Self {
        Self { engine, mirada_ctl: "mirada-ctl".into() }
    }

    /// Corre `mirada-ctl <args...>`, devolviendo el stdout en éxito.
    async fn mirada(&self, args: &[&str]) -> Result<String, String> {
        let out = Command::new(&self.mirada_ctl)
            .args(args)
            .output()
            .await
            .map_err(|e| format!("mirada-ctl {}: {e}", args.join(" ")))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(format!("mirada-ctl {} → {}", args.join(" "), String::from_utf8_lossy(&out.stderr)))
        }
    }

    /// Encarna `command` bajo `slice/<unit_label>` y lo mueve al
    /// special-workspace `special`. Devuelve el card-id como string.
    async fn incarnate(&self, label: &str, command: &str, slice: &str, special: &str) -> Result<String, String> {
        let (exec, argv) = split_cmd(command);
        if exec.is_empty() {
            return Err(format!("comando vacío para `{label}`"));
        }
        let mut card = Card::new(format!("pacha:{label}"));
        card.payload = Payload::Native { exec, argv, envp: vec![] };
        // Bajo el subárbol del contexto: reweight/freeze del slice lo cubre.
        card.soma.cgroup.path = format!("{slice}/{label}");
        let handle = self.engine.run(Intent::new(card)).await.map_err(|e| e.to_string())?;
        // Agrupar la ventana en el special-workspace del contexto. NOTA: hoy
        // `move-to-special` actúa sobre la ventana enfocada; agrupar de forma
        // robusta por app_id es el spike de ventanas (ver plan). Best-effort.
        let _ = self.mirada(&["move-to-special", special]).await;
        Ok(handle.card_id.to_string())
    }
}

#[async_trait]
impl Surfaces for LinuxSurfaces {
    async fn write_overlay(&mut self, overlay: &WawaOverlay) -> Result<(), String> {
        let path = wawa_config::context_config_path().ok_or("sin config dir para context.json")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(overlay).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    async fn clear_overlay(&mut self) -> Result<(), String> {
        let Some(path) = wawa_config::context_config_path() else { return Ok(()) };
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    async fn apply_vista(&mut self, vista: &str) -> Result<(), String> {
        self.mirada(&["vista", "use", vista]).await.map(|_| ())
    }

    async fn spawn(&mut self, spec: &AppSpec, slice: &str, special: &str) -> Result<String, String> {
        self.incarnate(&spec.app_id, &spec.command, slice, special).await
    }

    async fn respawn(&mut self, app_id: &str, slice: &str, special: &str) -> Result<String, String> {
        // Reabrir por app_id: sin la receta original, lanzamos el binario que
        // coincide con el app_id (convención: app_id == comando base). Si el
        // comando real difería, la receta (Fresh) es el camino fiable.
        self.incarnate(app_id, app_id, slice, special).await
    }

    async fn hide_windows(&mut self, special: &str) -> Result<(), String> {
        // toggle-special esconde el cajón si está visible. Idempotencia real
        // depende del estado del compositor — best-effort para MVP.
        self.mirada(&["toggle-special", special]).await.map(|_| ())
    }

    async fn show_windows(&mut self, special: &str) -> Result<(), String> {
        self.mirada(&["toggle-special", special]).await.map(|_| ())
    }

    async fn set_cpu_weight(&mut self, slice: &str, weight: u32) -> Result<(), String> {
        self.engine.set_cpu_weight(slice.to_string(), weight).await.map_err(|e| e.to_string())
    }

    async fn freeze(&mut self, slice: &str, frozen: bool) -> Result<(), String> {
        self.engine.freeze(slice.to_string(), frozen).await.map_err(|e| e.to_string())
    }

    async fn stop_units(&mut self, units: &[String]) -> Result<(), String> {
        for u in units {
            if let Ok(id) = Ulid::from_string(u) {
                let _ = self.engine.stop(id, std::time::Duration::from_millis(1000)).await;
            }
        }
        Ok(())
    }

    async fn snapshot_apps(&mut self, special: &str) -> Result<Vec<String>, String> {
        // `mirada-ctl windows --porcelain`: ID \t workspace \t app_id \t title.
        // Filtramos por las que estén en el special-workspace del contexto.
        // (Spike de ventanas: si mirada no etiqueta el special en `workspace`,
        // esto devuelve vacío y el restore cae a la receta — degradación OK.)
        let out = self.mirada(&["windows", "--porcelain"]).await.unwrap_or_default();
        let mut ids = Vec::new();
        for line in out.lines() {
            let mut f = line.split('\t');
            let (_id, ws, app_id) = (f.next(), f.next(), f.next());
            if let (Some(ws), Some(app_id)) = (ws, app_id) {
                if ws == special && !app_id.is_empty() {
                    ids.push(app_id.to_string());
                }
            }
        }
        Ok(ids)
    }
}

/// Parte un comando en (exec, argv) por espacios. MVP sin quoting de shell:
/// los comandos de receta son simples (`puriy --profile oficina`).
fn split_cmd(command: &str) -> (String, Vec<String>) {
    let mut it = command.split_whitespace().map(str::to_string);
    let exec = it.next().unwrap_or_default();
    (exec, it.collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_cmd_separa_exec_y_args() {
        let (e, a) = split_cmd("puriy --profile oficina");
        assert_eq!(e, "puriy");
        assert_eq!(a, vec!["--profile", "oficina"]);
        let (e, a) = split_cmd("steam");
        assert_eq!(e, "steam");
        assert!(a.is_empty());
        let (e, _) = split_cmd("   ");
        assert_eq!(e, "");
    }
}
