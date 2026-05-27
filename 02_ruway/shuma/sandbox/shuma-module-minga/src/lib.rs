//! `shuma-module-minga` — visualizador del repo Minga del cwd como tab
//! del shell.
//!
//! Muestra:
//! - Counts del repo: raíces (α-hashes), nodos del grafo CAS,
//!   atestaciones, claves del MST.
//! - Lista de las últimas raíces ingeridas con su α-hash y dialect.
//!
//! Diseño del tab:
//!
//! ```text
//!  Minga · local · /home/u/proyecto/.minga
//!  raíces: 14 · nodos: 1322 · atestaciones: 14 · mst: 14
//!  ────────────────────────────────────────────────────
//!  a1b2c3d4e5f6789a  rust
//!  f5e6a7b80c1d2e3f  python
//!  …
//! ```
//!
//! El módulo abre el `PersistentRepo` en read-only cada refresh. Si la
//! apertura falla (no hay `.minga` en el cwd, sled corrupto, etc.) el
//! tab muestra un mensaje informativo en lugar de los counts.
//!
//! Contribuciones:
//! - Monitor "minga · raíces": curva con la cantidad de raíces del MST.
//! - Shortcut "Refresh": fuerza un re-load del snapshot.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::Theme;
use minga_core::ContentHash;
use minga_store::PersistentRepo;
use shuma_module::{ModuleContributions, MonitorSpec, Rgb, Sample, ShortcutSpec, Source};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// `id` canónico — el chasis lo usa para enrutar el shumarc.
pub const ID: &str = "minga";

/// Subdirectorio del repo sled dentro del directorio Minga
/// (típicamente `<cwd>/.minga/repo`).
const REPO_SUBDIR: &str = "repo";

/// Cuántas raíces recientes mostrar en el tab — paridad con el
/// `RECENT_LIMIT` del explorer standalone.
pub const RECENT_LIMIT: usize = 10;

/// Snapshot del repo: counts + muestra de raíces. Inmutable una vez
/// construido por [`load_snapshot`].
#[derive(Debug, Clone, Default)]
pub struct RepoSnapshot {
    pub roots: usize,
    pub nodes: usize,
    pub attestations: usize,
    pub mst_keys: usize,
    /// Raíces recientes (orden lexicográfico de sled — no temporal).
    pub recent: Vec<RootRow>,
}

/// Una fila del listado de raíces, ya formateada para la vista.
#[derive(Debug, Clone)]
pub struct RootRow {
    pub alpha: ContentHash,
    pub dialect: Option<&'static str>,
    /// Resultado del último `Verify` sobre esta raíz: `Some(true)` si
    /// el α-hash es consistente con su contenido bajo algún dialect,
    /// `Some(false)` si no, `None` si nunca se verificó.
    pub verified: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct State {
    pub source: Source,
    /// Path del directorio Minga (típicamente `<cwd>/.minga`). El
    /// módulo lee `<repo_path>/repo/` para abrir sled.
    pub repo_path: PathBuf,
    pub snapshot: Option<RepoSnapshot>,
    pub error: Option<String>,
    /// Raíz seleccionada (último click en una fila). El chasis dispara
    /// `load_root_source` y reenvía el resultado.
    pub selected: Option<ContentHash>,
    /// Fuente reconstruida de la raíz seleccionada — o un mensaje de
    /// error si la reconstrucción falló. `None` mientras carga.
    pub selected_source: Option<Result<String, String>>,
    /// Counter de raíces compartido con el `sampler` del monitor.
    /// Mutex porque el sampler corre en un hilo del host.
    roots_count: Arc<Mutex<usize>>,
}

impl State {
    /// Estado por defecto: source local, repo_path = `<cwd>/.minga`.
    pub fn new(source: Source) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::with_repo_path(source, cwd.join(".minga"))
    }

    /// Estado apuntando a un `repo_path` específico — para tests o
    /// cuando el shumarc lo override en `options`.
    pub fn with_repo_path(source: Source, repo_path: PathBuf) -> Self {
        Self {
            source,
            repo_path,
            snapshot: None,
            error: None,
            selected: None,
            selected_source: None,
            roots_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Cuenta de raíces actual (alimenta el monitor de `contributions`).
    pub fn roots_count(&self) -> usize {
        *self.roots_count.lock().unwrap()
    }
}

/// Mensajes del módulo. El host enruta `Refresh` desde el shortcut
/// `minga.refresh`; `SnapshotReady` viene del worker que abrió sled.
#[derive(Debug, Clone)]
pub enum Msg {
    /// Pide releer el repo. El chasis debe llamar a [`load_snapshot`]
    /// en un thread aparte y reenviar el resultado como `SnapshotReady`.
    Refresh,
    /// Resultado de un refresh.
    SnapshotReady(Result<RepoSnapshot, String>),
    /// El usuario clickeó una raíz. El chasis carga el contenido en un
    /// thread y reenvía como `SourceLoaded`.
    SelectRoot(ContentHash),
    /// Fuente reconstruida (o error) para la raíz que `selected`
    /// señala. Se ignora si la `alpha` ya no coincide con `selected`
    /// (otro click llegó antes que este resultado).
    SourceLoaded {
        alpha: ContentHash,
        result: Result<String, String>,
    },
    /// Cierra el visor de fuente — deselecciona.
    DeselectRoot,
    /// Pide verificar todas las raíces visibles. El chasis spawnea el
    /// trabajo y reenvía `VerifyAllReady`.
    VerifyAll,
    /// Resultado de un VerifyAll: para cada α, `true` si la raíz es
    /// consistente bajo algún dialect, `false` si no.
    VerifyAllReady(Vec<(ContentHash, bool)>),
}

/// Mapea `action_id` de `ShortcutAction::ModuleAction` al `Msg`.
pub fn dispatch(action_id: &str) -> Option<Msg> {
    match action_id {
        "minga.refresh" => Some(Msg::Refresh),
        "minga.verify_all" => Some(Msg::VerifyAll),
        _ => None,
    }
}

/// Aplica un `Msg` al estado. `Refresh` es **declarativo**: marca el
/// estado pero NO hace IO. El chasis lanza el load fuera de `update`.
pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::Refresh => {
            s.error = None;
        }
        Msg::SnapshotReady(Ok(snap)) => {
            *s.roots_count.lock().unwrap() = snap.roots;
            s.snapshot = Some(snap);
            s.error = None;
        }
        Msg::SnapshotReady(Err(e)) => {
            s.error = Some(e);
        }
        Msg::SelectRoot(alpha) => {
            s.selected = Some(alpha);
            s.selected_source = None; // cargando…
        }
        Msg::SourceLoaded { alpha, result } => {
            // Race-protect: si el usuario clickeó otra raíz mientras
            // el thread cargaba la primera, descartamos el resultado.
            if s.selected == Some(alpha) {
                s.selected_source = Some(result);
            }
        }
        Msg::DeselectRoot => {
            s.selected = None;
            s.selected_source = None;
        }
        Msg::VerifyAll => {
            // Limpia las marcas previas; el chasis dispara el trabajo
            // y mandará VerifyAllReady.
            if let Some(snap) = &mut s.snapshot {
                for row in &mut snap.recent {
                    row.verified = None;
                }
            }
        }
        Msg::VerifyAllReady(results) => {
            if let Some(snap) = &mut s.snapshot {
                use std::collections::HashMap;
                let by_hash: HashMap<_, _> = results.into_iter().collect();
                for row in &mut snap.recent {
                    if let Some(ok) = by_hash.get(&row.alpha) {
                        row.verified = Some(*ok);
                    }
                }
            }
        }
    }
    s
}

/// Reconstruye cada raíz visible y la verifica con `verify_root_alpha`.
/// Bloqueante — corre en un thread del host disparado por `VerifyAll`.
pub fn verify_all_blocking(
    repo_path: &std::path::Path,
    alphas: &[ContentHash],
) -> Vec<(ContentHash, bool)> {
    let inner = repo_path.join(REPO_SUBDIR);
    let repo = match PersistentRepo::open(&inner) {
        Ok(r) => r,
        Err(_) => return alphas.iter().map(|a| (*a, false)).collect(),
    };
    let mut out = Vec::with_capacity(alphas.len());
    for &alpha in alphas {
        let ok = match repo.roots.get(&alpha) {
            Ok(Some((struct_hash, _))) => match repo.nodes.reconstruct(&struct_hash) {
                Ok(Some(node)) => {
                    minga_core::alpha::verify_root_alpha(&node, &alpha).is_some()
                }
                _ => false,
            },
            _ => false,
        };
        out.push((alpha, ok));
    }
    out
}

/// Lee el `StoredNode` raíz y devuelve la fuente reconstruida
/// (`render_source`). Bloqueante — pensado para correr en un thread
/// del host como respuesta a [`Msg::SelectRoot`].
pub fn load_root_source(
    repo_path: &std::path::Path,
    alpha: ContentHash,
) -> Result<String, String> {
    let inner = repo_path.join(REPO_SUBDIR);
    let repo = PersistentRepo::open(&inner).map_err(|e| format!("open sled: {e}"))?;
    let struct_hash = match repo.roots.get(&alpha).map_err(|e| e.to_string())? {
        Some((sh, _)) => sh,
        None => return Err(format!("α-hash {alpha} no es una raíz registrada")),
    };
    let node = repo
        .nodes
        .reconstruct(&struct_hash)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("struct-hash {struct_hash} no está en el grafo"))?;
    Ok(minga_vfs_render_source(&node))
}

/// Helper local — `minga-vfs::render_source` reexportado para no
/// agregar otra dep aquí. La función vive en `minga-vfs`.
fn minga_vfs_render_source(node: &minga_core::SemanticNode) -> String {
    minga_vfs::render_source(node)
}

/// Lee el repo Minga en `repo_path/<REPO_SUBDIR>` y devuelve counts +
/// últimas raíces. Bloqueante — pensado para correr en un thread del
/// host. Si el directorio no existe, devuelve `Err` con mensaje
/// explicativo (no panic).
pub fn load_snapshot(repo_path: &std::path::Path) -> Result<RepoSnapshot, String> {
    let inner = repo_path.join(REPO_SUBDIR);
    if !inner.exists() {
        return Err(format!(
            "no hay repo Minga en {} (esperaba {})",
            repo_path.display(),
            inner.display()
        ));
    }
    let repo = PersistentRepo::open(&inner).map_err(|e| format!("open sled: {e}"))?;
    let nodes = repo.nodes.len();
    let attestations = repo.attestations.len();
    let mst_keys = repo.mst.len();
    let roots = repo.roots.len();

    let recent: Vec<RootRow> = repo
        .roots
        .iter()
        .filter_map(|r| r.ok())
        .take(RECENT_LIMIT)
        .map(|(alpha, _struct, dialect)| RootRow {
            alpha,
            dialect: dialect.map(|d| d.name()),
            verified: None,
        })
        .collect();

    Ok(RepoSnapshot {
        roots,
        nodes,
        attestations,
        mst_keys,
        recent,
    })
}

// ─── Vista ──────────────────────────────────────────────────────────

/// Renderiza el tab del módulo. `lift` mapea `Msg` del módulo al
/// `ShellMsg` del chasis (cierre que el host construye según el slot).
pub fn view<ShellMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> ShellMsg + Clone + 'static,
) -> View<ShellMsg> {
    let mut children: Vec<View<ShellMsg>> = Vec::new();

    children.push(header_row(state, theme));

    if let Some(e) = &state.error {
        children.push(text_row(e, theme.fg_muted, theme));
    } else if let Some(snap) = &state.snapshot {
        children.push(counts_row(snap, theme));
        children.push(separator(theme));
        for row in &snap.recent {
            let is_selected = state.selected == Some(row.alpha);
            let lift_click = lift.clone();
            let alpha = row.alpha;
            children.push(root_row(
                row,
                theme,
                is_selected,
                move || lift_click(Msg::SelectRoot(alpha)),
            ));
        }
        if snap.recent.is_empty() {
            children.push(text_row(
                "(sin raíces — corré `minga ingest`)",
                theme.fg_muted,
                theme,
            ));
        }

        // Panel inferior con la fuente reconstruida de la raíz
        // seleccionada (si la hay).
        if state.selected.is_some() {
            children.push(separator(theme));
            children.push(selected_source_panel(state, theme, lift.clone()));
        }
    } else {
        children.push(text_row("cargando…", theme.fg_muted, theme));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(12.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(children)
}

fn header_row<M: Clone + 'static>(state: &State, theme: &Theme) -> View<M> {
    let title = format!(
        "Minga · {} · {}",
        state.source.label(),
        state.repo_path.display()
    );
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(title, 12.0, theme.fg_text, Alignment::Start)
}

fn counts_row<M: Clone + 'static>(snap: &RepoSnapshot, theme: &Theme) -> View<M> {
    let s = format!(
        "raíces: {} · nodos: {} · atestaciones: {} · mst: {}",
        snap.roots, snap.nodes, snap.attestations, snap.mst_keys
    );
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(s, 11.0, theme.fg_text, Alignment::Start)
}

fn root_row<M: Clone + 'static>(
    row: &RootRow,
    theme: &Theme,
    is_selected: bool,
    on_click: impl FnOnce() -> M,
) -> View<M> {
    let alpha_hex = row.alpha.to_string();
    let short: String = alpha_hex.chars().take(16).collect();
    let dialect = row.dialect.unwrap_or("?");
    let marker = if is_selected { "▶ " } else { "  " };
    // Marca de verificación: `·` = no verificado, `✓` = OK, `✘` = inconsistente.
    let v = match row.verified {
        None => "·",
        Some(true) => "✓",
        Some(false) => "✘",
    };
    let line = format!("{marker}{v} {short}  {dialect}");
    let bg = if is_selected {
        theme.bg_selected
    } else {
        theme.bg_panel
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(theme.bg_row_hover)
    .text_aligned(line, 11.0, theme.fg_text, Alignment::Start)
    .on_click(on_click())
}

/// Panel inferior con la fuente reconstruida de la raíz seleccionada.
/// Muestra "cargando…" mientras el thread del chasis trae el contenido,
/// o el render canónico cuando llega.
fn selected_source_panel<ShellMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> ShellMsg + Clone + 'static,
) -> View<ShellMsg> {
    let alpha_short: String = state
        .selected
        .map(|a| a.to_string().chars().take(16).collect())
        .unwrap_or_default();
    let close_lift = lift.clone();
    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .hover_fill(theme.bg_button_hover)
    .text_aligned(
        format!("· fuente de {alpha_short} — click para cerrar"),
        11.0,
        theme.fg_muted,
        Alignment::Start,
    )
    .on_click(close_lift(Msg::DeselectRoot));

    let body_text = match &state.selected_source {
        None => "cargando…".to_string(),
        Some(Ok(src)) => src.clone(),
        Some(Err(e)) => format!("✘ error: {e}"),
    };
    let body = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(4.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_input)
    .text_aligned(body_text, 11.0, theme.fg_text, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .children(vec![header, body])
}

fn text_row<M: Clone + 'static>(
    msg: &str,
    color: llimphi_ui::llimphi_raster::peniko::Color,
    _theme: &Theme,
) -> View<M> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(msg.to_string(), 11.0, color, Alignment::Start)
}

fn separator<M: Clone + 'static>(theme: &Theme) -> View<M> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.border)
}

// ─── contributions ──────────────────────────────────────────────────

pub fn contributions(state: &State) -> ModuleContributions {
    let counter = state.roots_count.clone();
    let monitor = MonitorSpec {
        id: "minga.roots",
        label: "minga · raíces".to_string(),
        accent: Rgb::new(0xB4, 0x8E, 0xAD),
        history_capacity: 60,
        period_secs: 5.0,
        sampler: Box::new(move || {
            let n = *counter.lock().unwrap();
            Sample::new(n as f32, format!("{n} raíces"))
        }),
    };

    ModuleContributions {
        monitors: vec![monitor],
        shortcuts: vec![
            ShortcutSpec::module_action("Refresh", "minga.refresh")
                .with_hint("Relee el repo Minga del cwd"),
            ShortcutSpec::module_action("Verify", "minga.verify_all")
                .with_hint("Recomputa el α-hash de cada raíz visible y marca consistencia"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable() {
        assert_eq!(ID, "minga");
    }

    #[test]
    fn dispatch_known_refresh() {
        assert!(matches!(dispatch("minga.refresh"), Some(Msg::Refresh)));
    }

    #[test]
    fn dispatch_unknown_returns_none() {
        assert!(dispatch("foo.bar").is_none());
        assert!(dispatch("matilda.refresh").is_none());
    }

    #[test]
    fn load_snapshot_errors_on_missing_repo() {
        let p = std::env::temp_dir().join(format!(
            "shuma-module-minga-missing-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let err = load_snapshot(&p).unwrap_err();
        assert!(err.contains("no hay repo"), "msg debe explicar: {err}");
    }

    #[test]
    fn snapshot_ready_updates_counts_and_clears_error() {
        let mut s = State::with_repo_path(Source::Local, PathBuf::from("/tmp/nope"));
        s.error = Some("anterior".into());
        let snap = RepoSnapshot {
            roots: 7,
            nodes: 100,
            attestations: 7,
            mst_keys: 7,
            recent: vec![],
        };
        let s2 = update(s, Msg::SnapshotReady(Ok(snap)));
        assert_eq!(s2.roots_count(), 7);
        assert!(s2.error.is_none());
        assert_eq!(s2.snapshot.unwrap().roots, 7);
    }

    #[test]
    fn snapshot_error_sets_error_only() {
        let s = State::with_repo_path(Source::Local, PathBuf::from("/tmp/nope"));
        let s2 = update(s, Msg::SnapshotReady(Err("boom".to_string())));
        assert_eq!(s2.error.as_deref(), Some("boom"));
        assert!(s2.snapshot.is_none());
    }
}
