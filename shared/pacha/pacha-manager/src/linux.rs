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

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use card_core::{Card, Payload};
use pacha_core::{AppSpec, WawaOverlay};
use pacha_dotfiles::{ConjuntoDotfiles, Instantanea, StoreObjetos};
use sandokan::{Engine, Intent};
use tokio::process::Command;
use ulid::Ulid;

use crate::Surfaces;

/// El contexto de versionado de dotfiles que `LinuxSurfaces` usa para
/// materializar/recapturar sets. Embebe el almacén de objetos, el `$HOME`
/// destino, el catálogo de [`ConjuntoDotfiles`] por id, y la cabeza (último
/// commit) de cada set para encadenar la historia.
pub struct DotfilesCtx {
    store: StoreObjetos,
    home: PathBuf,
    sets: BTreeMap<String, ConjuntoDotfiles>,
    heads: BTreeMap<String, [u8; 32]>,
}

impl DotfilesCtx {
    /// Arma el contexto con un almacén en `store_dir`, materializando hacia
    /// `home`, y el catálogo de sets indexado por su id.
    pub fn new(
        store_dir: impl Into<PathBuf>,
        home: impl Into<PathBuf>,
        sets: impl IntoIterator<Item = ConjuntoDotfiles>,
    ) -> Result<Self, String> {
        let store = StoreObjetos::abrir(store_dir.into()).map_err(|e| e.to_string())?;
        let sets = sets.into_iter().map(|s| (s.id.clone(), s)).collect();
        Ok(Self { store, home: home.into(), sets, heads: BTreeMap::new() })
    }

    /// Recaptura un set y commitea sobre su cabeza previa. Devuelve la raíz del
    /// árbol (lo que el pin del runtime guarda y `materializar` consume).
    fn capturar(&mut self, set_id: &str) -> Result<[u8; 32], String> {
        let set = self.sets.get(set_id).ok_or_else(|| format!("set desconocido: {set_id}"))?;
        let raiz = pacha_dotfiles::capturar(&self.store, set, &self.home).map_err(|e| e.to_string())?;
        let inst = Instantanea {
            raiz,
            padre: self.heads.get(set_id).copied(),
            etiqueta: format!("auto: dejar contexto ({set_id})"),
            creada_ms: ahora_ms(),
        };
        let commit = pacha_dotfiles::commitear(&self.store, &inst).map_err(|e| e.to_string())?;
        self.heads.insert(set_id.to_string(), commit);
        Ok(raiz)
    }
}

/// Milisegundos desde el epoch (real-side: aquí sí se consulta el reloj, a
/// diferencia del núcleo puro de `pacha-core`).
fn ahora_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Superficies reales. Embebe el `Engine` (elegido por `sandokan::auto`: init
/// de sistema → daemon → in-process) y conoce el binario de `mirada-ctl`.
pub struct LinuxSurfaces {
    engine: Box<dyn Engine>,
    mirada_ctl: String,
    dotfiles: Option<DotfilesCtx>,
}

impl LinuxSurfaces {
    /// Conecta al orquestador disponible y usa `mirada-ctl` del PATH.
    pub async fn connect() -> Self {
        let socket = sandokan::default_socket_path();
        Self {
            engine: sandokan::auto(&socket).await,
            mirada_ctl: "mirada-ctl".into(),
            dotfiles: None,
        }
    }

    /// Igual que [`connect`](Self::connect) pero con un `Engine` ya construido
    /// (para tests de humo o engines remotos).
    pub fn with_engine(engine: Box<dyn Engine>) -> Self {
        Self { engine, mirada_ctl: "mirada-ctl".into(), dotfiles: None }
    }

    /// Habilita el versionado de dotfiles (si no se llama, los efectos
    /// `Materializar`/`Capturar` fallan best-effort con un warning).
    pub fn with_dotfiles(mut self, ctx: DotfilesCtx) -> Self {
        self.dotfiles = Some(ctx);
        self
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

    /// Encarna `command` bajo `slice/<unit_label>`, agrupando su ventana en el
    /// special-workspace `special`. Devuelve el card-id como string.
    ///
    /// El agrupamiento es **sin race de foco**: registramos la membresía
    /// `app_id → special` en mirada ANTES de encarnar (`place-app-special`), así
    /// la ventana nace ya etiquetada cuando aparezca, sin depender de cuál esté
    /// enfocada. La ventana nace visible; `stash`/`summon` la ocultan/traen con
    /// sus compañeras al cambiar de contexto.
    async fn incarnate(&self, label: &str, command: &str, slice: &str, special: &str) -> Result<String, String> {
        let (exec, argv) = split_cmd(command);
        if exec.is_empty() {
            return Err(format!("comando vacío para `{label}`"));
        }
        // Registrar la membresía de contexto antes de lanzar (best-effort: si
        // mirada no corre, la encarnación igual procede).
        let _ = self.mirada(&["place-app-special", label, special]).await;
        let mut card = Card::new(format!("pacha:{label}"));
        card.payload = Payload::Native { exec, argv, envp: vec![] };
        // Bajo el subárbol del contexto: reweight/freeze del slice lo cubre.
        card.soma.cgroup.path = format!("{slice}/{label}");
        let handle = self.engine.run(Intent::new(card)).await.map_err(|e| e.to_string())?;
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
        // stash-special oculta TODAS las ventanas etiquetadas del contexto,
        // estén donde estén (idempotente: si no hay ninguna, no hace nada).
        self.mirada(&["stash-special", special]).await.map(|_| ())
    }

    async fn show_windows(&mut self, special: &str) -> Result<(), String> {
        // summon-special las trae teseladas al escritorio activo.
        self.mirada(&["summon-special", special]).await.map(|_| ())
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

    async fn snapshot_apps(&mut self, _special: &str) -> Result<Vec<String>, String> {
        // Con el modelo de membresía por `app_id`, las ventanas del contexto
        // están VISIBLES en un escritorio normal (no en un workspace llamado
        // como el especial), así que `mirada-ctl windows` no las distingue de
        // las demás. Capturar exactamente las del contexto requiere que mirada
        // exponga una consulta de membresía (`window_special`) — pendiente.
        // Hasta entonces devolvemos vacío: el restore cae a la receta del
        // contexto (degradación documentada en el plan).
        Ok(Vec::new())
    }

    async fn materialize_dotfiles(&mut self, set_id: &str, raiz: [u8; 32]) -> Result<(), String> {
        let ctx = self.dotfiles.as_ref().ok_or("dotfiles no configurados")?;
        pacha_dotfiles::materializar(&ctx.store, &ctx.home, raiz).map_err(|e| format!("{set_id}: {e}"))
    }

    async fn capture_dotfiles(&mut self, set_id: &str) -> Result<[u8; 32], String> {
        let ctx = self.dotfiles.as_mut().ok_or("dotfiles no configurados")?;
        ctx.capturar(set_id)
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
