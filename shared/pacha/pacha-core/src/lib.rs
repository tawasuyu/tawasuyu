//! `pacha-core` — **contextos de usuario**: el modelo puro de un *modo de
//! uso* con nombre ("oficina", "juegos", "particular", "pron").
//!
//! Un `pacha` es una **capa por encima de los perfiles** (config del SO,
//! vista del compositor, perfil de navegador) y **dentro** de una sola
//! sesión de usuario — no es una sesión FUS (mismo uid, sin re-login). Al
//! activarse compone esos perfiles, reabre/persiste un set de apps y aplica
//! una **política de orquestación de procesos** (modo juegos = más CPU al
//! juego, el resto deprioritizado).
//!
//! Este crate es **política pura y testeable**, igual que
//! `mirada-brain::fus` o `sandokan-lifecycle`: no toca disco como efecto, no
//! habla con mirada/sandokan/wawa-config. Decide **qué efectos** hay que
//! emitir y deja que `pacha-manager` los ejecute contra las superficies
//! reales. Eso vuelve la transición de contexto determinista y verificable
//! sin levantar una pantalla ni un daemon.
//!
//! El reparto de responsabilidades:
//!
//! * [`Catalog`] — las **definiciones** ([`Pacha`]) que el usuario edita y
//!   se persisten en `pachas.ron`.
//! * [`Runtime`] — el **estado vivo**: cuál está activo, cuáles quedaron en
//!   background/pausa, qué unidades y ventanas rastrea cada uno.
//! * [`Runtime::plan_switch`] — la **máquina de transición**: muta el estado
//!   y devuelve la lista ordenada de [`Effect`] a ejecutar.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// =====================================================================
// Definiciones (lo que el usuario edita; se persiste en pachas.ron)
// =====================================================================

/// Un contexto de usuario: un modo de uso con nombre que, al activarse,
/// compone perfiles + apps + política de recursos.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pacha {
    /// Slug estable y único (`"oficina"`, `"juegos"`). Es la identidad: de
    /// él se derivan el slice cgroup (`pacha-<id>.slice`) y el
    /// special-workspace (`pacha-<id>`) — ver [`Pacha::slice`] /
    /// [`Pacha::special`].
    pub id: String,
    /// Nombre visible en la UI.
    pub label: String,
    /// Overlay de config del SO a forzar mientras el contexto está activo
    /// (theme/accent/lang/…). `None` = no tocar la config base del usuario.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<WawaOverlay>,
    /// Vista/keymap del compositor a aplicar (`mirada-ctl vista use`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vista: Option<String>,
    /// Receta de apps a abrir cuando el contexto arranca desde cero.
    #[serde(default)]
    pub apps: Vec<AppSpec>,
    /// Si `true`, al dejar el contexto se snapshotean los `app_id` vivos
    /// para reabrir exactamente eso la próxima vez (en lugar de la receta).
    #[serde(default)]
    pub persist: bool,
    /// Política de recursos del contexto (cgroups v2). Se aplica a su slice.
    #[serde(default)]
    pub resources: ResourcePolicy,
    /// Qué hacer con **este** contexto cuando se lo deja (el default
    /// configurable por contexto que pidió el diseño).
    #[serde(default)]
    pub on_leave: OnLeave,
}

impl Pacha {
    /// Constructor mínimo: id + label, todo lo demás en su default.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            overlay: None,
            vista: None,
            apps: Vec::new(),
            persist: false,
            resources: ResourcePolicy::default(),
            on_leave: OnLeave::default(),
        }
    }

    /// Nombre del slice cgroup donde viven las unidades del contexto. El
    /// freezer v2 es jerárquico, así que freeze/weight sobre este slice
    /// gobiernan todo el subárbol del contexto.
    pub fn slice(&self) -> String {
        slice_for(&self.id)
    }

    /// Nombre del special-workspace de mirada que agrupa las ventanas del
    /// contexto (para ocultarlas/mostrarlas como bloque).
    pub fn special(&self) -> String {
        special_for(&self.id)
    }

    /// Peso de CPU efectivo cuando el contexto está **activo** (lo que el
    /// usuario fija para "modo juegos"; default 100 = neutro de cgroup v2).
    pub fn active_weight(&self) -> u32 {
        self.resources.cpu_weight.unwrap_or(DEFAULT_ACTIVE_WEIGHT)
    }
}

/// Slice cgroup canónico de un contexto por id.
pub fn slice_for(id: &str) -> String {
    format!("pacha-{id}.slice")
}

/// Special-workspace canónico de un contexto por id.
pub fn special_for(id: &str) -> String {
    format!("pacha-{id}")
}

/// Inversa de [`slice_for`]: extrae el id de un slice (`pacha-<id>.slice`).
/// `None` si el string no tiene esa forma. La usa el manager para mapear las
/// unidades que encarnó (etiquetadas por slice) de vuelta a su contexto.
pub fn id_from_slice(slice: &str) -> Option<&str> {
    slice.strip_prefix("pacha-")?.strip_suffix(".slice")
}

/// Una app de la receta de un contexto.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSpec {
    /// Comando completo, ej. `"puriy --profile oficina"`.
    pub command: String,
    /// `app_id` Wayland — para placement en mirada y para reconocer la
    /// ventana al moverla al special-workspace del contexto.
    pub app_id: String,
    /// Workspace destino dentro del contexto (1-based; `None` = no fijar).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<usize>,
}

impl AppSpec {
    pub fn new(command: impl Into<String>, app_id: impl Into<String>) -> Self {
        Self { command: command.into(), app_id: app_id.into(), workspace: None }
    }
}

/// Qué pasa con un contexto cuando se lo deja. Es el **default configurable
/// por contexto**: cada `Pacha` declara el suyo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OnLeave {
    /// Sigue corriendo con `cpu.weight` rebajado y ventanas ocultas. Vuelta
    /// instantánea. Es el default sensato.
    #[default]
    Background,
    /// Congelado vía `cgroup.freeze` (0% CPU, RAM retenida). Más ahorro,
    /// pero apps de red/descargas se cuelgan mientras.
    Pause,
    /// Se detienen todas sus unidades. Máximo ahorro; reabrir cuesta.
    Close,
}

/// Política de recursos de un contexto (espeja `CgroupSpec`/`ResourceLimits`
/// de `card-core`, con tipos planos para no acoplar este crate a card).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcePolicy {
    /// `cpu.weight` cuando está activo (1..=10000; cgroup v2). Modo juegos =
    /// alto. `None` = neutro (100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_weight: Option<u32>,
    /// `io.weight` (1..=10000). `None` = no fijar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub io_weight: Option<u32>,
    /// `memory.max` en bytes. `None` = sin límite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_max: Option<u64>,
    /// Afinidad de CPU (cores). `None` = sin pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_affinity: Option<Vec<u32>>,
}

/// Overlay parcial de la config del SO. Cada campo `None`/ausente NO toca la
/// capa base del usuario. `pacha-manager` lo serializa a `context.json`
/// (la tercera capa de `wawa-config`); los campos coinciden con los de
/// `WawaConfig` para que el merge sea directo.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WawaOverlay {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timefmt_24h: Option<bool>,
    /// Override key-by-key de módulos (igual semántica que el merge profundo
    /// de wawa-config).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub modules: BTreeMap<String, bool>,
}

impl WawaOverlay {
    /// `true` si el overlay no fija nada — el manager puede saltarse escribir
    /// el archivo y directamente limpiarlo.
    pub fn is_empty(&self) -> bool {
        self.theme_variant.is_none()
            && self.accent.is_none()
            && self.lang.is_none()
            && self.timefmt_24h.is_none()
            && self.modules.is_empty()
    }
}

/// `cpu.weight` por defecto de un contexto activo: el neutro de cgroup v2.
pub const DEFAULT_ACTIVE_WEIGHT: u32 = 100;
/// `cpu.weight` de un contexto en background: deprioritizado, no inanición
/// total (sigue progresando si nadie más compite).
pub const BACKGROUND_CPU_WEIGHT: u32 = 10;

// =====================================================================
// Catálogo de definiciones
// =====================================================================

/// El conjunto de contextos definidos por el usuario. Se persiste en
/// `~/.config/pacha/pachas.ron`. Orden estable (BTreeMap) → diffs limpios.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pachas: BTreeMap<String, Pacha>,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Da de alta (o reemplaza) una definición. La clave es su `id`.
    pub fn upsert(&mut self, p: Pacha) {
        self.pachas.insert(p.id.clone(), p);
    }

    /// Borra una definición. Devuelve la removida si existía.
    pub fn remove(&mut self, id: &str) -> Option<Pacha> {
        self.pachas.remove(id)
    }

    pub fn get(&self, id: &str) -> Option<&Pacha> {
        self.pachas.get(id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.pachas.contains_key(id)
    }

    pub fn len(&self) -> usize {
        self.pachas.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pachas.is_empty()
    }

    /// Itera las definiciones en orden de id.
    pub fn iter(&self) -> impl Iterator<Item = &Pacha> + '_ {
        self.pachas.values()
    }

    /// Serializa a RON con `pretty` (para escribir `pachas.ron`).
    pub fn to_ron(&self) -> Result<String, PachaError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| PachaError::Ron(e.to_string()))
    }

    /// Parsea desde RON.
    pub fn from_ron(s: &str) -> Result<Self, PachaError> {
        ron::from_str(s).map_err(|e| PachaError::Ron(e.to_string()))
    }
}

// =====================================================================
// Estado runtime + máquina de transición
// =====================================================================

/// Ciclo de vida de un contexto en el runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Lifecycle {
    /// Nunca arrancado, o cerrado: no tiene unidades vivas.
    #[default]
    Closed,
    /// El contexto que el usuario está usando ahora.
    Active,
    /// Corriendo, deprioritizado, ventanas ocultas.
    Background,
    /// Congelado (`cgroup.freeze`).
    Paused,
}

/// Estado vivo de un contexto concreto.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeState {
    pub lifecycle: Lifecycle,
    /// Ids de unidades (cards) que el manager encarnó para este contexto —
    /// para poder pararlas como grupo sin enumerar `cgroup.procs`.
    #[serde(default)]
    pub units: Vec<String>,
    /// `app_id`s capturados la última vez que se dejó el contexto (si
    /// `persist`). El manager los reabre en vez de la receta.
    #[serde(default)]
    pub last_session: Vec<String>,
}

/// El roster vivo: qué contexto está activo y el estado de cada uno. Es el
/// análogo intra-usuario de `SessionRoster` (FUS), pero las identidades son
/// los slugs de las definiciones (estables por construcción).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Runtime {
    active: Option<String>,
    states: BTreeMap<String, RuntimeState>,
}

/// Un efecto a ejecutar contra las superficies reales. `pacha-core` los
/// **decide**; `pacha-manager` los **ejecuta** (mirada-ctl, Engine de
/// sandokan, escritura de cgroups, overlay de wawa-config).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    // --- dejar el contexto saliente ---
    /// Capturar los `app_id` vivos del contexto a `last_session` (persist).
    SnapshotApps { pacha: String, special: String },
    /// Rebajar el `cpu.weight` del slice (background).
    SetCpuWeight { slice: String, cpu_weight: u32 },
    /// Congelar el slice (`cgroup.freeze=1`).
    Freeze { slice: String },
    /// Parar las unidades del contexto (close).
    StopUnits { units: Vec<String> },
    /// Ocultar las ventanas del contexto (toggle-special para esconder).
    HideWindows { special: String },

    // --- aplicar la config del entrante ---
    /// Escribir el overlay de config (`context.json`).
    WriteOverlay { overlay: WawaOverlay },
    /// Borrar el overlay → volver a la config base del usuario.
    ClearOverlay,
    /// Aplicar la vista/keymap del compositor.
    ApplyVista { vista: String },

    // --- traer el contexto entrante ---
    /// Descongelar el slice (`cgroup.freeze=0`).
    Unfreeze { slice: String },
    /// Mostrar las ventanas del contexto.
    ShowWindows { special: String },
    /// Encarnar una app de la receta bajo el slice del contexto y moverla a
    /// su special-workspace.
    SpawnApp { spec: AppSpec, slice: String, special: String },
    /// Reabrir una app persistida (de `last_session`) por su `app_id`.
    RespawnApp { app_id: String, slice: String, special: String },
}

/// Cómo traer el contexto entrante respecto de sus apps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BringUp {
    /// Si hay `last_session` y el contexto persiste, reabrir eso; si no, la
    /// receta. (Comportamiento normal.)
    Restore,
    /// Ignorar `last_session`: abrir la receta desde cero.
    Fresh,
}

impl Runtime {
    pub fn new() -> Self {
        Self::default()
    }

    /// El contexto activo, o `None` si ninguno (arranque).
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Estado de un contexto (default `Closed` si nunca se tocó).
    pub fn lifecycle(&self, id: &str) -> Lifecycle {
        self.states.get(id).map(|s| s.lifecycle).unwrap_or(Lifecycle::Closed)
    }

    pub fn state(&self, id: &str) -> Option<&RuntimeState> {
        self.states.get(id)
    }

    /// Registra las unidades que el manager encarnó para un contexto (las
    /// necesita para pararlas como grupo).
    pub fn set_units(&mut self, id: &str, units: Vec<String>) {
        self.states.entry(id.to_string()).or_default().units = units;
    }

    /// Guarda el snapshot de apps tras un Close/Background con persist.
    pub fn set_last_session(&mut self, id: &str, app_ids: Vec<String>) {
        self.states.entry(id.to_string()).or_default().last_session = app_ids;
    }

    /// Itera `(id, &RuntimeState)` en orden de id.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &RuntimeState)> + '_ {
        self.states.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// **La transición.** Cambia el foco al contexto `to` (que debe existir
    /// en `cat`). Muta el estado al resultado y devuelve la lista ordenada
    /// de efectos a ejecutar. `bring` controla si se respeta `last_session`.
    ///
    /// Si `to` ya es el activo, no hace nada (lista vacía). Si no había
    /// activo (arranque), sólo emite los efectos de traída del entrante.
    pub fn plan_switch(
        &mut self,
        cat: &Catalog,
        to: &str,
        bring: BringUp,
    ) -> Result<Vec<Effect>, PachaError> {
        let to_def = cat.get(to).ok_or_else(|| PachaError::Unknown(to.to_string()))?;
        if self.active.as_deref() == Some(to) {
            return Ok(Vec::new());
        }

        let mut fx = Vec::new();

        // 1) Dejar el contexto saliente según SU on_leave.
        if let Some(from) = self.active.clone() {
            if let Some(from_def) = cat.get(&from) {
                self.leave(from_def, &mut fx);
            }
        }

        // 2) Aplicar la config del entrante.
        match &to_def.overlay {
            Some(ov) if !ov.is_empty() => fx.push(Effect::WriteOverlay { overlay: ov.clone() }),
            _ => fx.push(Effect::ClearOverlay),
        }
        if let Some(v) = &to_def.vista {
            fx.push(Effect::ApplyVista { vista: v.clone() });
        }

        // 3) Traer el entrante según en qué estado estaba.
        self.bring_up(to_def, bring, &mut fx);

        // 4) Marcar activo.
        self.active = Some(to.to_string());
        self.states.entry(to.to_string()).or_default().lifecycle = Lifecycle::Active;

        Ok(fx)
    }

    /// Efectos para dejar el contexto saliente, según su `on_leave`.
    fn leave(&mut self, from: &Pacha, fx: &mut Vec<Effect>) {
        let st = self.states.entry(from.id.clone()).or_default();
        match from.on_leave {
            OnLeave::Background => {
                fx.push(Effect::SetCpuWeight {
                    slice: from.slice(),
                    cpu_weight: BACKGROUND_CPU_WEIGHT,
                });
                fx.push(Effect::HideWindows { special: from.special() });
                st.lifecycle = Lifecycle::Background;
            }
            OnLeave::Pause => {
                fx.push(Effect::HideWindows { special: from.special() });
                fx.push(Effect::Freeze { slice: from.slice() });
                st.lifecycle = Lifecycle::Paused;
            }
            OnLeave::Close => {
                if from.persist {
                    fx.push(Effect::SnapshotApps {
                        pacha: from.id.clone(),
                        special: from.special(),
                    });
                }
                if !st.units.is_empty() {
                    fx.push(Effect::StopUnits { units: st.units.clone() });
                }
                fx.push(Effect::HideWindows { special: from.special() });
                st.lifecycle = Lifecycle::Closed;
            }
        }
    }

    /// Efectos para traer el contexto entrante, según en qué estado estaba.
    fn bring_up(&mut self, to: &Pacha, bring: BringUp, fx: &mut Vec<Effect>) {
        let prev = self.lifecycle(&to.id);
        match prev {
            Lifecycle::Paused => {
                fx.push(Effect::Unfreeze { slice: to.slice() });
                fx.push(Effect::SetCpuWeight { slice: to.slice(), cpu_weight: to.active_weight() });
                fx.push(Effect::ShowWindows { special: to.special() });
            }
            Lifecycle::Background => {
                fx.push(Effect::SetCpuWeight { slice: to.slice(), cpu_weight: to.active_weight() });
                fx.push(Effect::ShowWindows { special: to.special() });
            }
            // Closed o Active(imposible aquí) → arrancar desde cero.
            _ => {
                let last = self.states.get(&to.id).map(|s| s.last_session.clone()).unwrap_or_default();
                let use_last = bring == BringUp::Restore && to.persist && !last.is_empty();
                if use_last {
                    for app_id in last {
                        fx.push(Effect::RespawnApp {
                            app_id,
                            slice: to.slice(),
                            special: to.special(),
                        });
                    }
                } else {
                    for spec in &to.apps {
                        fx.push(Effect::SpawnApp {
                            spec: spec.clone(),
                            slice: to.slice(),
                            special: to.special(),
                        });
                    }
                }
                // El slice arranca con el peso activo del contexto.
                fx.push(Effect::SetCpuWeight { slice: to.slice(), cpu_weight: to.active_weight() });
            }
        }
    }

    /// Cierra explícitamente un contexto (acción `pacha close`), sin cambiar
    /// el foco. Útil para liberar recursos de algo en background.
    pub fn plan_close(&mut self, cat: &Catalog, id: &str) -> Result<Vec<Effect>, PachaError> {
        let def = cat.get(id).ok_or_else(|| PachaError::Unknown(id.to_string()))?;
        let mut fx = Vec::new();
        let st = self.states.entry(id.to_string()).or_default();
        if def.persist {
            fx.push(Effect::SnapshotApps { pacha: id.to_string(), special: def.special() });
        }
        if !st.units.is_empty() {
            fx.push(Effect::StopUnits { units: st.units.clone() });
        }
        fx.push(Effect::HideWindows { special: def.special() });
        st.lifecycle = Lifecycle::Closed;
        if self.active.as_deref() == Some(id) {
            self.active = None;
        }
        Ok(fx)
    }

    /// Serializa el runtime (para `state.ron` en `$XDG_RUNTIME_DIR`).
    pub fn to_ron(&self) -> Result<String, PachaError> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| PachaError::Ron(e.to_string()))
    }

    pub fn from_ron(s: &str) -> Result<Self, PachaError> {
        ron::from_str(s).map_err(|e| PachaError::Ron(e.to_string()))
    }
}

/// Errores del modelo. La (de)serialización RON se aplana a string para no
/// exponer el tipo de `ron` en la API pública.
#[derive(Debug, Error)]
pub enum PachaError {
    #[error("contexto desconocido: {0}")]
    Unknown(String),
    #[error("ron: {0}")]
    Ron(String),
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn cat_basico() -> Catalog {
        let mut c = Catalog::new();
        // oficina: background al salir, con una app y un overlay de theme.
        let mut oficina = Pacha::new("oficina", "Trabajo de oficina");
        oficina.on_leave = OnLeave::Background;
        oficina.overlay = Some(WawaOverlay { theme_variant: Some("light".into()), ..Default::default() });
        oficina.apps = vec![AppSpec::new("puriy --profile oficina", "puriy")];
        c.upsert(oficina);
        // juegos: cierra al salir, peso alto (modo juegos).
        let mut juegos = Pacha::new("juegos", "Juegos");
        juegos.on_leave = OnLeave::Close;
        juegos.resources.cpu_weight = Some(10000);
        juegos.apps = vec![AppSpec::new("steam", "steam")];
        c.upsert(juegos);
        c
    }

    #[test]
    fn slice_y_special_derivan_del_id() {
        let p = Pacha::new("juegos", "Juegos");
        assert_eq!(p.slice(), "pacha-juegos.slice");
        assert_eq!(p.special(), "pacha-juegos");
    }

    #[test]
    fn id_from_slice_es_inversa_de_slice_for() {
        assert_eq!(id_from_slice("pacha-juegos.slice"), Some("juegos"));
        assert_eq!(id_from_slice(&slice_for("oficina")), Some("oficina"));
        assert_eq!(id_from_slice("otra-cosa"), None);
        assert_eq!(id_from_slice("pacha-x"), None);
    }

    #[test]
    fn primer_switch_sin_activo_solo_trae_el_entrante() {
        let cat = cat_basico();
        let mut rt = Runtime::new();
        let fx = rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        // No hay saliente: arranca con overlay + vista(none) + spawn + weight.
        assert_eq!(
            fx,
            vec![
                Effect::WriteOverlay {
                    overlay: WawaOverlay { theme_variant: Some("light".into()), ..Default::default() }
                },
                Effect::SpawnApp {
                    spec: AppSpec::new("puriy --profile oficina", "puriy"),
                    slice: "pacha-oficina.slice".into(),
                    special: "pacha-oficina".into(),
                },
                Effect::SetCpuWeight { slice: "pacha-oficina.slice".into(), cpu_weight: 100 },
            ]
        );
        assert_eq!(rt.active(), Some("oficina"));
        assert_eq!(rt.lifecycle("oficina"), Lifecycle::Active);
    }

    #[test]
    fn switch_al_mismo_es_noop() {
        let cat = cat_basico();
        let mut rt = Runtime::new();
        rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        let fx = rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        assert!(fx.is_empty());
    }

    #[test]
    fn saliente_background_baja_peso_y_oculta() {
        let cat = cat_basico();
        let mut rt = Runtime::new();
        rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        let fx = rt.plan_switch(&cat, "juegos", BringUp::Restore).unwrap();
        // oficina (Background): baja peso + oculta. Luego entra juegos.
        assert_eq!(fx[0], Effect::SetCpuWeight { slice: "pacha-oficina.slice".into(), cpu_weight: BACKGROUND_CPU_WEIGHT });
        assert_eq!(fx[1], Effect::HideWindows { special: "pacha-oficina".into() });
        // juegos no tiene overlay → ClearOverlay.
        assert!(fx.contains(&Effect::ClearOverlay));
        // peso activo de juegos = 10000.
        assert!(fx.contains(&Effect::SetCpuWeight { slice: "pacha-juegos.slice".into(), cpu_weight: 10000 }));
        assert_eq!(rt.lifecycle("oficina"), Lifecycle::Background);
        assert_eq!(rt.lifecycle("juegos"), Lifecycle::Active);
    }

    #[test]
    fn volver_de_background_no_respawnea_solo_muestra() {
        let cat = cat_basico();
        let mut rt = Runtime::new();
        rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        rt.plan_switch(&cat, "juegos", BringUp::Restore).unwrap();
        let fx = rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        // oficina estaba en Background: subir peso + mostrar; NADA de spawn.
        assert!(fx.contains(&Effect::SetCpuWeight { slice: "pacha-oficina.slice".into(), cpu_weight: 100 }));
        assert!(fx.contains(&Effect::ShowWindows { special: "pacha-oficina".into() }));
        assert!(!fx.iter().any(|e| matches!(e, Effect::SpawnApp { .. })));
    }

    #[test]
    fn saliente_close_para_unidades_y_snapshotea_si_persiste() {
        let mut cat = cat_basico();
        // Hacemos que juegos persista para ejercitar el snapshot.
        let mut juegos = cat.get("juegos").unwrap().clone();
        juegos.persist = true;
        cat.upsert(juegos);

        let mut rt = Runtime::new();
        rt.plan_switch(&cat, "juegos", BringUp::Restore).unwrap();
        rt.set_units("juegos", vec!["unit-steam".into()]);

        let fx = rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        // juegos (Close + persist): snapshot, stop, hide.
        assert_eq!(fx[0], Effect::SnapshotApps { pacha: "juegos".into(), special: "pacha-juegos".into() });
        assert_eq!(fx[1], Effect::StopUnits { units: vec!["unit-steam".into()] });
        assert_eq!(fx[2], Effect::HideWindows { special: "pacha-juegos".into() });
        assert_eq!(rt.lifecycle("juegos"), Lifecycle::Closed);
    }

    #[test]
    fn pausa_congela_el_slice() {
        let mut cat = cat_basico();
        let mut oficina = cat.get("oficina").unwrap().clone();
        oficina.on_leave = OnLeave::Pause;
        cat.upsert(oficina);

        let mut rt = Runtime::new();
        rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        let fx = rt.plan_switch(&cat, "juegos", BringUp::Restore).unwrap();
        assert_eq!(fx[0], Effect::HideWindows { special: "pacha-oficina".into() });
        assert_eq!(fx[1], Effect::Freeze { slice: "pacha-oficina.slice".into() });
        assert_eq!(rt.lifecycle("oficina"), Lifecycle::Paused);
    }

    #[test]
    fn volver_de_pausa_descongela() {
        let mut cat = cat_basico();
        let mut oficina = cat.get("oficina").unwrap().clone();
        oficina.on_leave = OnLeave::Pause;
        cat.upsert(oficina);

        let mut rt = Runtime::new();
        rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        rt.plan_switch(&cat, "juegos", BringUp::Restore).unwrap();
        let fx = rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        assert!(fx.contains(&Effect::Unfreeze { slice: "pacha-oficina.slice".into() }));
        assert!(fx.contains(&Effect::ShowWindows { special: "pacha-oficina".into() }));
    }

    #[test]
    fn persist_restore_reabre_last_session_en_vez_de_receta() {
        let mut cat = cat_basico();
        let mut oficina = cat.get("oficina").unwrap().clone();
        oficina.on_leave = OnLeave::Close;
        oficina.persist = true;
        cat.upsert(oficina);

        let mut rt = Runtime::new();
        rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        // Simulamos que el manager capturó una sesión distinta a la receta.
        rt.set_last_session("oficina", vec!["puriy".into(), "nada".into()]);
        rt.plan_switch(&cat, "juegos", BringUp::Restore).unwrap(); // cierra oficina
        let fx = rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        // Reabre por app_id (last_session), no por la receta (SpawnApp).
        let respawns: Vec<_> = fx.iter().filter(|e| matches!(e, Effect::RespawnApp { .. })).collect();
        assert_eq!(respawns.len(), 2);
        assert!(!fx.iter().any(|e| matches!(e, Effect::SpawnApp { .. })));
    }

    #[test]
    fn fresh_ignora_last_session_y_usa_receta() {
        let mut cat = cat_basico();
        let mut oficina = cat.get("oficina").unwrap().clone();
        oficina.on_leave = OnLeave::Close;
        oficina.persist = true;
        cat.upsert(oficina);

        let mut rt = Runtime::new();
        rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        rt.set_last_session("oficina", vec!["puriy".into(), "nada".into()]);
        rt.plan_switch(&cat, "juegos", BringUp::Restore).unwrap();
        let fx = rt.plan_switch(&cat, "oficina", BringUp::Fresh).unwrap();
        // Fresh: la receta (SpawnApp), no last_session.
        assert!(fx.iter().any(|e| matches!(e, Effect::SpawnApp { .. })));
        assert!(!fx.iter().any(|e| matches!(e, Effect::RespawnApp { .. })));
    }

    #[test]
    fn switch_a_desconocido_es_error() {
        let cat = cat_basico();
        let mut rt = Runtime::new();
        let err = rt.plan_switch(&cat, "inexistente", BringUp::Restore).unwrap_err();
        assert!(matches!(err, PachaError::Unknown(_)));
    }

    #[test]
    fn catalogo_round_trip_ron() {
        let cat = cat_basico();
        let s = cat.to_ron().unwrap();
        let back = Catalog::from_ron(&s).unwrap();
        assert_eq!(cat, back);
    }

    #[test]
    fn runtime_round_trip_ron() {
        let cat = cat_basico();
        let mut rt = Runtime::new();
        rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap();
        rt.set_units("oficina", vec!["unit-puriy".into()]);
        let s = rt.to_ron().unwrap();
        let back = Runtime::from_ron(&s).unwrap();
        assert_eq!(rt, back);
    }

    #[test]
    fn plan_close_libera_sin_cambiar_foco_si_no_es_activo() {
        let cat = cat_basico();
        let mut rt = Runtime::new();
        rt.plan_switch(&cat, "oficina", BringUp::Restore).unwrap(); // activo oficina
        rt.plan_switch(&cat, "juegos", BringUp::Restore).unwrap(); // activo juegos, oficina bg
        rt.set_units("oficina", vec!["unit-puriy".into()]);
        let fx = rt.plan_close(&cat, "oficina").unwrap();
        assert!(fx.contains(&Effect::StopUnits { units: vec!["unit-puriy".into()] }));
        assert_eq!(rt.lifecycle("oficina"), Lifecycle::Closed);
        // El foco sigue en juegos.
        assert_eq!(rt.active(), Some("juegos"));
    }
}
