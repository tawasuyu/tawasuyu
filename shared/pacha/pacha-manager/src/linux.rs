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
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use card_core::{Card, Payload};
use pacha_core::{AppSpec, FsHome, FsProfile, WawaOverlay};
use pacha_dotfiles::{Cifrador, ConjuntoDotfiles, Instantanea, StoreObjetos};
use pacha_llavero::Llavero;
use sandokan::{Engine, Intent};
use sandokan_monitor_core::reglas::ReglaMetrica;
use sandokan_vigilante::Vigilante;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use ulid::Ulid;

use crate::Surfaces;

/// Cada cuánto el `Vigilante` pollea el plano de control y evalúa las reglas de
/// métrica. 2 s: suficiente para reaccionar a un pico sostenido sin cargar el
/// sistema con `observe()` constante.
pub const VIGILANTE_INTERVALO: Duration = Duration::from_secs(2);

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
        Ok(Self::con_store(store, home, sets))
    }

    /// Igual que [`new`](Self::new) pero con el store **cifrado en reposo** (Fase
    /// 2): la clave se deriva de la `seed` de identidad del usuario — la que
    /// `agora-keystore` desbloquea (el *cómo* desbloquearla es Fase 3; acá la
    /// recibe ya desbloqueada de quien construye el contexto). Los secretos
    /// quedan opacos en disco; el destino efímero los descifra en RAM.
    pub fn new_cifrado(
        store_dir: impl Into<PathBuf>,
        home: impl Into<PathBuf>,
        sets: impl IntoIterator<Item = ConjuntoDotfiles>,
        seed: &[u8; 32],
    ) -> Result<Self, String> {
        let store = StoreObjetos::abrir_cifrado(store_dir.into(), Cifrador::derivar_de_seed(seed))
            .map_err(|e| e.to_string())?;
        Ok(Self::con_store(store, home, sets))
    }

    fn con_store(
        store: StoreObjetos,
        home: impl Into<PathBuf>,
        sets: impl IntoIterator<Item = ConjuntoDotfiles>,
    ) -> Self {
        let sets = sets.into_iter().map(|s| (s.id.clone(), s)).collect();
        Self { store, home: home.into(), sets, heads: BTreeMap::new() }
    }

    /// Abre el ctx **cifrado** tomando la seed del **llavero de sesión** (Fase
    /// 3) bajo `nombre`. `Ok(None)` si la seed no está cacheada todavía — hay que
    /// **desbloquear primero** (pedir passphrase, abrir `agora-keystore`) y luego
    /// [`cachear_seed`](Self::cachear_seed). Así no se re-pregunta en cada
    /// conmutación de contexto: el kernel retiene la seed mientras viva la sesión.
    pub fn desde_llavero(
        store_dir: impl Into<PathBuf>,
        home: impl Into<PathBuf>,
        sets: impl IntoIterator<Item = ConjuntoDotfiles>,
        llavero: &dyn Llavero,
        nombre: &str,
    ) -> Result<Option<Self>, String> {
        match llavero.recuperar(nombre).map_err(|e| e.to_string())? {
            Some(seed) => Ok(Some(Self::new_cifrado(store_dir, home, sets, &seed)?)),
            None => Ok(None),
        }
    }

    /// Desbloquea para la sesión: guarda la `seed` (ya obtenida de `agora`) en el
    /// llavero bajo `nombre`, para que [`desde_llavero`](Self::desde_llavero) la
    /// encuentre sin re-preguntar. El *cómo* obtener la seed (passphrase →
    /// keystore) es del orquestador; acá sólo la retenemos en la sesión.
    pub fn cachear_seed(
        llavero: &dyn Llavero,
        nombre: &str,
        seed: &[u8; 32],
    ) -> Result<(), String> {
        llavero.guardar(nombre, seed).map_err(|e| e.to_string())
    }

    /// **Publica** (Fase 4) el estado ACTUAL de `set_id` cifrado a `destinatarios`
    /// (claves públicas X25519). Verbo explícito: re-cifra a otros. Devuelve el
    /// sobre portable; el contenido en claro nunca lo deja.
    pub fn publicar_set(
        &self,
        set_id: &str,
        destinatarios: &[[u8; 32]],
    ) -> Result<pacha_dotfiles::SobreCompartido, String> {
        let set = self.sets.get(set_id).ok_or_else(|| format!("set desconocido: {set_id}"))?;
        let raiz = pacha_dotfiles::capturar(&self.store, set, &self.home).map_err(|e| e.to_string())?;
        pacha_dotfiles::publicar_para(&self.store, raiz, destinatarios).map_err(|e| e.to_string())
    }

    /// **Empuja** (Fase 4) el estado ACTUAL de `set_id` a un store `remoto` por
    /// set-difference de hashes (backup tipo `git push`). Devuelve cuántos copió.
    pub fn empujar_set(
        &self,
        set_id: &str,
        remoto: &StoreObjetos,
    ) -> Result<pacha_dotfiles::PushStats, String> {
        let set = self.sets.get(set_id).ok_or_else(|| format!("set desconocido: {set_id}"))?;
        let raiz = pacha_dotfiles::capturar(&self.store, set, &self.home).map_err(|e| e.to_string())?;
        pacha_dotfiles::empujar(&self.store, remoto, raiz).map_err(|e| e.to_string())
    }

    // ---- Operaciones de versionado para CLI/daemon (Fase 5) ----

    /// `add`: agrega una ruta gestionada a un set (creándolo si no existe). La
    /// DEFINICIÓN del set vive en la config de pacha; esto la muta en caliente.
    pub fn agregar_ruta(&mut self, set_id: &str, ruta: pacha_dotfiles::RutaGestionada) {
        self.sets
            .entry(set_id.to_string())
            .or_insert_with(|| ConjuntoDotfiles::new(set_id))
            .entradas
            .push(ruta);
    }

    /// `snapshot`: captura+commitea el set (honrando splicing por ruta) y avanza
    /// su cabeza. Devuelve la raíz. Pública sobre la recaptura interna.
    pub fn snapshot_set(&mut self, set_id: &str) -> Result<[u8; 32], String> {
        self.capturar(set_id)
    }

    /// `restore`: materializa en `$HOME` la cabeza actual del set (su último
    /// commit). Error si el set no tiene historial todavía.
    pub fn restaurar_set(&self, set_id: &str) -> Result<(), String> {
        let commit = self
            .heads
            .get(set_id)
            .ok_or_else(|| format!("set sin snapshot: {set_id}"))?;
        let inst = pacha_dotfiles::leer_instantanea(&self.store, commit).map_err(|e| e.to_string())?;
        pacha_dotfiles::materializar(&self.store, &self.home, inst.raiz).map_err(|e| e.to_string())
    }

    /// La cabeza (último commit) de un set, si tiene historial.
    pub fn cabeza(&self, set_id: &str) -> Option<[u8; 32]> {
        self.heads.get(set_id).copied()
    }

    /// Los sets definidos (para listar/persistir el catálogo desde la CLI).
    pub fn sets(&self) -> Vec<ConjuntoDotfiles> {
        self.sets.values().cloned().collect()
    }

    /// `$HOME` destino de materialización (para mensajes de la CLI).
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Persiste las **cabezas** (commit por set) a `path` en RON. Las definiciones
    /// de sets viven en la config; esto es el estado-runtime del versionado, que
    /// debe sobrevivir reinicios del daemon (sin él, `restore` perdería el linaje).
    pub fn guardar_estado(&self, path: &Path) -> Result<(), String> {
        let estado: BTreeMap<String, String> =
            self.heads.iter().map(|(k, v)| (k.clone(), hex32(v))).collect();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let ron = ron::ser::to_string(&estado).map_err(|e| e.to_string())?;
        std::fs::write(path, ron).map_err(|e| e.to_string())
    }

    /// Carga las cabezas persistidas (mergea sobre las actuales). Silencioso si el
    /// archivo no existe (primer arranque).
    pub fn cargar_estado(&mut self, path: &Path) -> Result<(), String> {
        let bytes = match std::fs::read_to_string(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        let estado: BTreeMap<String, String> = ron::from_str(&bytes).map_err(|e| e.to_string())?;
        for (k, v) in estado {
            self.heads.insert(k, deshex32(&v)?);
        }
        Ok(())
    }

    /// Recaptura un set y commitea sobre su cabeza previa. Devuelve la raíz del
    /// árbol (lo que el pin del runtime guarda y `materializar` consume).
    ///
    /// **Splicing por ruta (Fase 5):** sólo recaptura de `$HOME` las rutas
    /// `Rastreado`; las `Fijado` se conservan del snapshot pinneado (la raíz del
    /// commit cabeza). Así dejar el contexto guarda lo editado sin pisar lo clavado.
    fn capturar(&mut self, set_id: &str) -> Result<[u8; 32], String> {
        let set = self.sets.get(set_id).ok_or_else(|| format!("set desconocido: {set_id}"))?;
        // Base = la raíz del commit cabeza previo, si hay (para conservar Fijado).
        let base = match self.heads.get(set_id) {
            Some(commit) => Some(
                pacha_dotfiles::leer_instantanea(&self.store, commit)
                    .map_err(|e| e.to_string())?
                    .raiz,
            ),
            None => None,
        };
        let raiz = pacha_dotfiles::capturar_splice(&self.store, set, &self.home, base)
            .map_err(|e| e.to_string())?;
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

/// Hex de un hash de 32 bytes (para persistir cabezas legibles en RON).
fn hex32(h: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in h {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Inversa de [`hex32`]: parsea 64 hex a `[u8; 32]`.
fn deshex32(s: &str) -> Result<[u8; 32], String> {
    if s.len() != 64 {
        return Err(format!("hash hex de largo inválido: {} (esperaba 64)", s.len()));
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(out)
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
    /// El `Vigilante` que corre las reglas de métrica de la intención activa
    /// (capa 4). Lo **inyecta el daemon** (que es quien lo construye con su
    /// propio handle al Engine y spawnea su lazo de poll); si no se cableó,
    /// `armar_reglas` degrada con un warning en vez de fallar.
    vigilante: Option<Arc<Vigilante>>,
}

impl LinuxSurfaces {
    /// Conecta al orquestador disponible y usa `mirada-ctl` del PATH.
    pub async fn connect() -> Self {
        let socket = sandokan::default_socket_path();
        Self {
            engine: sandokan::auto(&socket).await,
            mirada_ctl: "mirada-ctl".into(),
            dotfiles: None,
            vigilante: None,
        }
    }

    /// Igual que [`connect`](Self::connect) pero con un `Engine` ya construido
    /// (para tests de humo o engines remotos).
    pub fn with_engine(engine: Box<dyn Engine>) -> Self {
        Self { engine, mirada_ctl: "mirada-ctl".into(), dotfiles: None, vigilante: None }
    }

    /// Arranque **completo** del daemon: como [`connect`](Self::connect) pero
    /// además enciende un [`Vigilante`] —lazo de reglas de métrica— sobre un
    /// handle propio al mismo orquestador (`sandokan::auto`), spawnea su
    /// `correr()` y lo cablea para `armar_reglas`. Requiere un runtime tokio.
    /// Los dos handles al Engine (éste y el de las superficies) son clientes
    /// stateless del mismo backend, así que no se pisan.
    pub async fn connect_vigilado() -> Self {
        let socket = sandokan::default_socket_path();
        let engine: Arc<dyn Engine> = Arc::from(sandokan::auto(&socket).await);
        let vigilante = Arc::new(Vigilante::new(engine, Vec::new(), VIGILANTE_INTERVALO));
        tokio::spawn(vigilante.clone().correr());
        Self::connect().await.with_vigilante(vigilante)
    }

    /// Cablea el `Vigilante` que armará las reglas de métrica por contexto. El
    /// daemon lo construye (con su Engine) y spawnea su `correr()`; acá sólo
    /// guardamos el handle para `armar_reglas`.
    pub fn with_vigilante(mut self, vigilante: Arc<Vigilante>) -> Self {
        self.vigilante = Some(vigilante);
        self
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
    async fn incarnate(
        &self,
        label: &str,
        command: &str,
        slice: &str,
        special: &str,
        profile: Option<&FsProfile>,
    ) -> Result<String, String> {
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

        // Aislamiento de FS por perfil (Fase 1): compila el FsProfile a un
        // MountPlan que el incarnator realiza dentro del mount namespace de la
        // app. Los secret_sets se materializan a un tmpfs en RAM (no a disco).
        if let Some(p) = profile.filter(|p| p.aisla()) {
            let home = self.home_real()?;
            let staging = if matches!(p.home, FsHome::Dotfiles) {
                Some(self.stage_secret_sets(label, &p.secret_sets)?)
            } else {
                None
            };
            if let Some(plan) = mount_plan_for(&home, p, staging.as_deref()) {
                // mount: el ns donde viven los montajes; user: para realizarlos
                // sin root (uid→root-in-userns), igual que en arje-incarnate.
                card.soma.namespaces.mount = true;
                card.soma.namespaces.user = true;
                card.soma.mounts = plan;
            }
        }

        let handle = self.engine.run(Intent::new(card)).await.map_err(|e| e.to_string())?;
        Ok(handle.card_id.to_string())
    }

    /// El `$HOME` real a aislar: el de la config de dotfiles si está, si no el
    /// `$HOME` del entorno. Error si ninguno es resoluble.
    fn home_real(&self) -> Result<PathBuf, String> {
        if let Some(ctx) = &self.dotfiles {
            return Ok(ctx.home.clone());
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "fs_profile sin dotfiles ni $HOME resoluble".into())
    }

    /// Materializa los `secret_sets` (estado ACTUAL del `$HOME`) en un tmpfs RAM
    /// bajo `XDG_RUNTIME_DIR` y devuelve la ruta de staging. Copia en RAM: los
    /// cambios de la app no persisten ni tocan el `$HOME` real.
    fn stage_secret_sets(&self, label: &str, sets: &[String]) -> Result<PathBuf, String> {
        let ctx = self.dotfiles.as_ref().ok_or("dotfiles no configurados para fs_profile")?;
        let base = runtime_dir().join("pacha").join("secrets").join(label);
        stage_into(ctx, &base, sets)?;
        Ok(base)
    }
}

/// Compila un [`FsProfile`] a un [`card_core::MountPlan`]. `home` es el `$HOME`
/// real (sobre el que se monta); `staging` es la ruta tmpfs con los secret_sets
/// ya materializados (sólo para [`FsHome::Dotfiles`]). `None` si el perfil no
/// pide aislamiento. Pura: testeable sin I/O.
fn mount_plan_for(
    home: &Path,
    profile: &FsProfile,
    staging: Option<&Path>,
) -> Option<card_core::MountPlan> {
    use card_core::{HomeSpec, MountPlan};
    let destino = home.display().to_string();
    let home = match profile.home {
        FsHome::Heredar => return None,
        FsHome::Tmpfs => HomeSpec::Tmpfs { destino, size_bytes: None },
        FsHome::Dotfiles => HomeSpec::Subdir {
            origen: staging?.display().to_string(),
            destino,
        },
    };
    Some(MountPlan { home, ..Default::default() })
}

/// Materializa cada set (snapshot del `$HOME` actual) dentro de `base`,
/// limpiando un staging previo. Libre (no método) para testearla sin `Engine`.
fn stage_into(ctx: &DotfilesCtx, base: &Path, sets: &[String]) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(base);
    std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
    for set_id in sets {
        let set = ctx.sets.get(set_id).ok_or_else(|| format!("set desconocido: {set_id}"))?;
        let raiz = pacha_dotfiles::capturar(&ctx.store, set, &ctx.home).map_err(|e| e.to_string())?;
        pacha_dotfiles::materializar(&ctx.store, base, raiz).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Directorio de runtime en RAM (`XDG_RUNTIME_DIR`, un tmpfs). Fallback al temp
/// del sistema si la env no está (entornos sin sesión de usuario).
fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
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
        self.incarnate(&spec.app_id, &spec.command, slice, special, spec.fs_profile.as_ref()).await
    }

    async fn respawn(&mut self, app_id: &str, slice: &str, special: &str) -> Result<String, String> {
        // Reabrir por app_id: sin la receta original, lanzamos el binario que
        // coincide con el app_id (convención: app_id == comando base). Si el
        // comando real difería, la receta (Fresh) es el camino fiable. Sin la
        // receta tampoco hay fs_profile: el restore aislado cae a Fresh.
        self.incarnate(app_id, app_id, slice, special, None).await
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

    async fn armar_reglas(&mut self, reglas: &[ReglaMetrica]) -> Result<(), String> {
        match &self.vigilante {
            Some(v) => {
                v.armar(reglas.to_vec()).await;
                Ok(())
            }
            // Sin Vigilante cableado (el daemon no lo inyectó): las reglas de
            // métrica no corren. Degrada — no es un error duro que deba
            // impedir el cambio de contexto.
            None => {
                if !reglas.is_empty() {
                    tracing::warn!(n = reglas.len(), "armar_reglas sin Vigilante: reglas de métrica ignoradas");
                }
                Ok(())
            }
        }
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

    #[test]
    fn mount_plan_for_traduce_cada_modo() {
        use card_core::HomeSpec;
        let home = Path::new("/home/u");

        // Heredar = sin aislamiento.
        assert!(mount_plan_for(home, &FsProfile::default(), None).is_none());

        // Tmpfs = $HOME privado vacío.
        let p = FsProfile { home: FsHome::Tmpfs, secret_sets: vec![] };
        let plan = mount_plan_for(home, &p, None).unwrap();
        assert_eq!(plan.home, HomeSpec::Tmpfs { destino: "/home/u".into(), size_bytes: None });
        assert!(plan.binds.is_empty());

        // Dotfiles = $HOME = el staging RAM.
        let p = FsProfile { home: FsHome::Dotfiles, secret_sets: vec!["correo".into()] };
        let staging = Path::new("/run/user/1000/pacha/secrets/paloma");
        let plan = mount_plan_for(home, &p, Some(staging)).unwrap();
        assert_eq!(
            plan.home,
            HomeSpec::Subdir {
                origen: "/run/user/1000/pacha/secrets/paloma".into(),
                destino: "/home/u".into()
            }
        );
        // Dotfiles sin staging resuelto ⇒ None (no se puede aislar sin RAM).
        assert!(mount_plan_for(home, &p, None).is_none());
    }

    #[test]
    fn ops_add_snapshot_restore_y_persistencia_de_cabezas() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let store_dir = tmp.path().join("obj");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".zshrc"), b"v1\n").unwrap();

        let mut ctx = DotfilesCtx::new(store_dir.clone(), home.clone(), []).unwrap();
        // add: el set no existía; se crea con la ruta.
        ctx.agregar_ruta("shell", pacha_dotfiles::RutaGestionada::rastreado(".zshrc"));
        // snapshot: captura v1 y avanza cabeza.
        ctx.snapshot_set("shell").unwrap();
        let cabeza_v1 = ctx.cabeza("shell").expect("debe tener cabeza tras snapshot");

        // Editar y restaurar: vuelve a v1.
        std::fs::write(home.join(".zshrc"), b"v2-EDITADO\n").unwrap();
        ctx.restaurar_set("shell").unwrap();
        assert_eq!(std::fs::read(home.join(".zshrc")).unwrap(), b"v1\n", "restore trae la cabeza");

        // Persistir cabezas y recargarlas en un ctx NUEVO (otro 'arranque').
        let estado = tmp.path().join("estado.ron");
        ctx.guardar_estado(&estado).unwrap();

        let mut ctx2 = DotfilesCtx::new(store_dir, home.clone(), [
            ConjuntoDotfiles::new("shell").con(pacha_dotfiles::RutaGestionada::rastreado(".zshrc")),
        ]).unwrap();
        assert!(ctx2.cabeza("shell").is_none(), "ctx nuevo arranca sin cabezas");
        ctx2.cargar_estado(&estado).unwrap();
        assert_eq!(ctx2.cabeza("shell"), Some(cabeza_v1), "la cabeza sobrevivió el reinicio");

        // Y restore desde el ctx recargado sigue funcionando.
        std::fs::write(home.join(".zshrc"), b"otra-cosa\n").unwrap();
        ctx2.restaurar_set("shell").unwrap();
        assert_eq!(std::fs::read(home.join(".zshrc")).unwrap(), b"v1\n");

        // cargar_estado sobre archivo ausente no rompe.
        let mut ctx3 = DotfilesCtx::new(tmp.path().join("obj3"), home, []).unwrap();
        ctx3.cargar_estado(&tmp.path().join("noexiste.ron")).unwrap();
        assert!(ctx3.cabeza("shell").is_none());
    }

    #[test]
    fn publicar_y_empujar_set_desde_el_manager() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".zshrc"), b"export FASE4=1\n").unwrap();
        let set = ConjuntoDotfiles::new("shell")
            .con(pacha_dotfiles::RutaGestionada::fijado(".zshrc"));
        let ctx = DotfilesCtx::new_cifrado(tmp.path().join("obj"), home.clone(), [set], &[7u8; 32]).unwrap();

        // Publicar a un destinatario y que SÓLO él abra.
        let seed_bob = [44u8; 32];
        let sobre = ctx.publicar_set("shell", &[pacha_dotfiles::clave_publica_de_seed(&seed_bob)]).unwrap();
        let (raiz, objs) = pacha_dotfiles::abrir_compartido(&sobre, &seed_bob).unwrap();
        let store_bob = StoreObjetos::abrir(tmp.path().join("bob")).unwrap();
        pacha_dotfiles::importar(&store_bob, &objs).unwrap();
        let dest = tmp.path().join("dest");
        pacha_dotfiles::materializar(&store_bob, &dest, raiz).unwrap();
        assert_eq!(std::fs::read(dest.join(".zshrc")).unwrap(), b"export FASE4=1\n");

        // Empujar a un remoto y reproducir.
        let remoto = StoreObjetos::abrir_cifrado(tmp.path().join("remoto"), Cifrador::con_clave([8u8; 32])).unwrap();
        let stats = ctx.empujar_set("shell", &remoto).unwrap();
        assert!(stats.copiados > 0);
        let dest2 = tmp.path().join("dest2");
        pacha_dotfiles::materializar(&remoto, &dest2, raiz).unwrap();
        assert_eq!(std::fs::read(dest2.join(".zshrc")).unwrap(), b"export FASE4=1\n");
    }

    #[test]
    fn desde_llavero_none_sin_cache_y_abre_el_mismo_store_tras_cachear() {
        use pacha_llavero::LlaveroMemoria;

        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let store_dir = tmp.path().join("obj");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".zshrc"), b"export OK=1\n").unwrap();
        let set = || {
            ConjuntoDotfiles::new("shell").con(pacha_dotfiles::RutaGestionada::fijado(".zshrc"))
        };

        let seed = [55u8; 32];
        // Escribir el store cifrado por la vía directa (seed en mano) y capturar.
        let directo = DotfilesCtx::new_cifrado(store_dir.clone(), home.clone(), [set()], &seed).unwrap();
        let raiz = pacha_dotfiles::capturar(&directo.store, directo.sets.get("shell").unwrap(), &home).unwrap();

        // Sin cachear, el llavero no entrega ctx.
        let ll = LlaveroMemoria::new();
        let vacio = DotfilesCtx::desde_llavero(
            store_dir.clone(), home.clone(), [set()], &ll, "id:default",
        ).unwrap();
        assert!(vacio.is_none(), "sin seed cacheada, desde_llavero = None");

        // Tras cachear LA MISMA seed, el ctx del llavero abre el MISMO store.
        DotfilesCtx::cachear_seed(&ll, "id:default", &seed).unwrap();
        let ctx = DotfilesCtx::desde_llavero(
            store_dir.clone(), home.clone(), [set()], &ll, "id:default",
        ).unwrap().expect("con seed cacheada debe dar Some");

        let dest = tmp.path().join("dest");
        pacha_dotfiles::materializar(&ctx.store, &dest, raiz).unwrap();
        assert_eq!(std::fs::read(dest.join(".zshrc")).unwrap(), b"export OK=1\n");

        // Una seed equivocada cacheada NO abre (AEAD falla al materializar).
        let ll_mal = LlaveroMemoria::new();
        DotfilesCtx::cachear_seed(&ll_mal, "id:default", &[1u8; 32]).unwrap();
        let ctx_mal = DotfilesCtx::desde_llavero(
            store_dir.clone(), home.clone(), [set()], &ll_mal, "id:default",
        ).unwrap().unwrap();
        let dest_mal = tmp.path().join("dest_mal");
        assert!(pacha_dotfiles::materializar(&ctx_mal.store, &dest_mal, raiz).is_err());
    }

    #[test]
    fn stage_into_con_store_cifrado_descifra_en_ram_y_deja_disco_opaco() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let store_dir = tmp.path().join("obj");
        let staging = tmp.path().join("ram");

        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/id_ed25519"), b"SECRETO-DE-CONTEXTO\n").unwrap();

        let set = ConjuntoDotfiles::new("claves")
            .con(pacha_dotfiles::RutaGestionada::fijado(".ssh/id_ed25519"));
        // Store cifrado, clave derivada de la seed de identidad.
        let ctx = DotfilesCtx::new_cifrado(store_dir.clone(), home.clone(), [set], &[3u8; 32]).unwrap();

        stage_into(&ctx, &staging, &["claves".into()]).unwrap();

        // Descifra correctamente al staging (RAM).
        assert_eq!(std::fs::read(staging.join(".ssh/id_ed25519")).unwrap(), b"SECRETO-DE-CONTEXTO\n");

        // Pero en el store de disco el secreto está OPACO.
        let mut crudo = Vec::new();
        for shard in std::fs::read_dir(&store_dir).unwrap() {
            let shard = shard.unwrap().path();
            if shard.is_dir() {
                for o in std::fs::read_dir(&shard).unwrap() {
                    crudo.extend(std::fs::read(o.unwrap().path()).unwrap());
                }
            }
        }
        assert!(!crudo.is_empty());
        assert!(
            !crudo.windows(19).any(|w| w == b"SECRETO-DE-CONTEXTO"),
            "el secreto no debe aparecer en claro en el store"
        );
    }

    #[test]
    fn stage_into_materializa_los_sets_en_ram_sin_tocar_el_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let store_dir = tmp.path().join("obj");
        let staging = tmp.path().join("ram"); // hace de XDG_RUNTIME_DIR

        // $HOME real con un "secreto".
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/id_ed25519"), b"SECRETO\n").unwrap();

        let set = ConjuntoDotfiles::new("claves").con(
            pacha_dotfiles::RutaGestionada::fijado(".ssh/id_ed25519"),
        );
        let ctx = DotfilesCtx::new(store_dir, home.clone(), [set]).unwrap();

        stage_into(&ctx, &staging, &["claves".into()]).unwrap();

        // El secreto aterrizó en el staging (RAM)...
        assert_eq!(std::fs::read(staging.join(".ssh/id_ed25519")).unwrap(), b"SECRETO\n");
        // ...y el plan de montaje usa ese staging como $HOME.
        let p = FsProfile { home: FsHome::Dotfiles, secret_sets: vec!["claves".into()] };
        let plan = mount_plan_for(&home, &p, Some(&staging)).unwrap();
        match plan.home {
            card_core::HomeSpec::Subdir { origen, destino } => {
                assert_eq!(origen, staging.display().to_string());
                assert_eq!(destino, home.display().to_string());
            }
            otro => panic!("esperaba Subdir, fue {otro:?}"),
        }
    }
}
