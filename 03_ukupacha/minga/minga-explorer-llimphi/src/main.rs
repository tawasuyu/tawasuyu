//! `minga-explorer-llimphi` — dashboard Llimphi del repo Minga (VCS
//! semántico P2P).
//!
//! Polling cada 2s contra `MINGA_REPO` (env, default `./.minga`), abre
//! el `PersistentRepo` (sled, sin passphrase porque los counts son
//! lectura pública) y muestra:
//! - Cantidad de nodos AST almacenados.
//! - Cantidad de atestaciones firmadas.
//! - Cantidad de claves del MST (Merkle Search Tree).
//!
//! No requiere keypair descifrado — eso queda para el CLI
//! (`minga status`) cuando hace falta el DID. El explorer foco es
//! observabilidad rápida.
//!
//! Stack visual: llimphi-theme + llimphi-widget-app-header +
//! llimphi-widget-banner + llimphi-widget-stat-card. Mismo patrón que
//! `nakui-explorer-llimphi`.
//!
//! Uso:
//! ```sh
//! cargo run -p minga-explorer-llimphi
//! # con repo custom:
//! MINGA_REPO=/path/to/.minga cargo run -p minga-explorer-llimphi
//! ```

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_stat_card::{stat_card_view, StatCardPalette};
use minga_store::PersistentRepo;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const REPO_DIRNAME: &str = "repo";

/// Cuántos items recientes mostrar por sección. Los stores no tienen
/// orden cronológico (sled ordena lexicográfico por hash); los
/// "recent" acá son simplemente los primeros del iter — sirve como
/// sample, no como log temporal.
const RECENT_LIMIT: usize = 5;

#[derive(Clone, Default, Debug)]
struct RepoSnapshot {
    nodes: usize,
    attestations: usize,
    mst_keys: usize,
    recent_nodes: Vec<(String, String)>,
    recent_attestations: Vec<(String, String)>,
    recent_mst_keys: Vec<String>,
}

struct Model {
    theme: Theme,
    repo_path: PathBuf,
    snapshot: Option<RepoSnapshot>,
    error: Option<String>,
    last_load_ms: u64,
}

#[derive(Clone)]
enum Msg {
    /// Tick del scheduler: corre `load_snapshot` y dispatcha el
    /// resultado como `Refresh`.
    Tick,
    /// Resultado de un refresh: snapshot exitoso o mensaje de error,
    /// junto al tiempo que tardó el load.
    Refresh {
        result: Result<RepoSnapshot, String>,
        elapsed_ms: u64,
    },
}

struct Explorer;

impl App for Explorer {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Minga — Repo"
    }

    fn initial_size() -> (u32, u32) {
        (800, 560)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let repo_path = std::env::var("MINGA_REPO")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".minga"));

        // Primer refresh inmediato + ticks periódicos. El `Tick` dispara
        // el load en un thread aparte (vía `Handle::spawn` desde update);
        // así el sled no bloquea el hilo de UI.
        handle.dispatch(Msg::Tick);
        handle.spawn_periodic(REFRESH_INTERVAL, || Msg::Tick);

        Model {
            theme: Theme::dark(),
            repo_path,
            snapshot: None,
            error: None,
            last_load_ms: 0,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                let path = m.repo_path.clone();
                handle.spawn(move || {
                    let started = std::time::Instant::now();
                    let result = load_snapshot(&path);
                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    Msg::Refresh { result, elapsed_ms }
                });
            }
            Msg::Refresh { result, elapsed_ms } => {
                match result {
                    Ok(snap) => {
                        m.snapshot = Some(snap);
                        m.error = None;
                    }
                    Err(e) => {
                        m.error = Some(format!(
                            "no pude leer repo {}: {e}",
                            m.repo_path.display()
                        ));
                    }
                }
                m.last_load_ms = elapsed_ms;
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        let header_palette = AppHeaderPalette::from_theme(theme);
        let stat_palette = StatCardPalette::from_theme(theme);

        // Acentos por kind del dashboard: nodos azul, atestaciones
        // verde, MST purple. Señales semánticas del dominio Minga.
        let accent_nodes = Color::from_rgba8(0x88, 0xc0, 0xd0, 0xff);
        let accent_attestations = Color::from_rgba8(0xa3, 0xbe, 0x8c, 0xff);
        let accent_mst = Color::from_rgba8(0xb4, 0x8e, 0xad, 0xff);

        let header_text = match &model.snapshot {
            Some(_) => format!(
                "Repo: {}  ·  reload {} ms",
                model.repo_path.display(),
                model.last_load_ms
            ),
            None => format!("Buscando repo en {}…", model.repo_path.display()),
        };

        let header = app_header::<Msg>(header_text, vec![], &header_palette);

        let mut body_children: Vec<View<Msg>> = Vec::new();

        if let Some(ref e) = model.error {
            body_children.push(banner_view::<Msg>(BannerKind::Error, e.clone()));
        }

        match &model.snapshot {
            None => {
                body_children.push(empty_message(theme));
            }
            Some(snap) => {
                let node_items: Vec<String> = snap
                    .recent_nodes
                    .iter()
                    .map(|(h, k)| format!("{h}  {k}"))
                    .collect();
                let attestation_items: Vec<String> = snap
                    .recent_attestations
                    .iter()
                    .map(|(h, did)| format!("{h}  ←  {did}"))
                    .collect();
                let mst_items: Vec<String> = snap.recent_mst_keys.clone();

                body_children.push(stat_card_view::<Msg>(
                    "Nodos AST",
                    snap.nodes.to_string(),
                    "fragments parseados del código",
                    accent_nodes,
                    &node_items,
                    &stat_palette,
                ));
                body_children.push(stat_card_view::<Msg>(
                    "Atestaciones",
                    snap.attestations.to_string(),
                    "firmas Ed25519 sobre los nodos",
                    accent_attestations,
                    &attestation_items,
                    &stat_palette,
                ));
                body_children.push(stat_card_view::<Msg>(
                    "Claves MST",
                    snap.mst_keys.to_string(),
                    "entradas del Merkle Search Tree",
                    accent_mst,
                    &mst_items,
                    &stat_palette,
                ));
            }
        }

        let body = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            padding: Rect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(12.0_f32),
                bottom: length(16.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(8.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(body_children);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, body])
    }
}

fn empty_message(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        "Esperando primer refresh…".to_string(),
        13.0,
        theme.fg_muted,
        Alignment::Start,
    )
}

/// Lee el repo sled `<repo_path>/repo` y devuelve los 3 counts.
/// Falla si: el dir no existe, sled rebota al abrir, o cualquier
/// store falla a `len()`. Ningún error es fatal — la UI muestra el
/// banner y mantiene el último snapshot bueno.
fn load_snapshot(repo_path: &std::path::Path) -> Result<RepoSnapshot, String> {
    let inner = repo_path.join(REPO_DIRNAME);
    if !inner.exists() {
        return Err(format!(
            "directorio del repo sled no existe: {}",
            inner.display()
        ));
    }
    let repo = PersistentRepo::open(&inner).map_err(|e| format!("open: {e}"))?;

    let nodes = repo.nodes.len();
    let attestations = repo.attestations.len();
    let mst_keys = repo.mst.len();

    let recent_nodes: Vec<(String, String)> = repo
        .nodes
        .iter()
        .filter_map(|r| r.ok())
        .take(RECENT_LIMIT)
        .map(|(hash, stored)| (short_hash(&hash.to_string()), stored.kind))
        .collect();

    let recent_attestations: Vec<(String, String)> = repo
        .attestations
        .iter()
        .filter_map(|r| r.ok())
        .take(RECENT_LIMIT)
        .map(|att| {
            (
                short_hash(&att.content.to_string()),
                short_hash(&att.author.to_string()),
            )
        })
        .collect();

    let recent_mst_keys: Vec<String> = repo
        .mst
        .iter()
        .filter_map(|r| r.ok())
        .take(RECENT_LIMIT)
        .map(|h| short_hash(&h.to_string()))
        .collect();

    Ok(RepoSnapshot {
        nodes,
        attestations,
        mst_keys,
        recent_nodes,
        recent_attestations,
        recent_mst_keys,
    })
}

/// Trunca un hex string a sus primeros 12 chars. Convención cross-app
/// para mostrar hashes/dids/contenthash compactos sin perder
/// distintividad práctica (12 hex = 48 bits, colisión improbable
/// dentro de un repo single-machine).
fn short_hash(s: &str) -> String {
    s.chars().take(12).collect()
}

fn main() {
    llimphi_ui::run::<Explorer>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_snapshot_errors_on_missing_dir() {
        let p = std::env::temp_dir().join(format!(
            "minga-explorer-llimphi-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let err = load_snapshot(&p).unwrap_err();
        assert!(
            err.contains("no existe"),
            "msg debe explicar el missing: {err}"
        );
    }

    #[test]
    fn snapshot_default_is_zeros_and_empty_lists() {
        let s = RepoSnapshot::default();
        assert_eq!(s.nodes, 0);
        assert_eq!(s.attestations, 0);
        assert_eq!(s.mst_keys, 0);
        assert!(s.recent_nodes.is_empty());
        assert!(s.recent_attestations.is_empty());
        assert!(s.recent_mst_keys.is_empty());
    }

    #[test]
    fn short_hash_takes_first_12_chars() {
        let s = "a1b2c3d4e5f6789012345678901234567890123456789012345678901234abcd";
        assert_eq!(short_hash(s), "a1b2c3d4e5f6");
        assert_eq!(short_hash(s).len(), 12);
    }

    #[test]
    fn short_hash_handles_empty_or_shorter() {
        assert_eq!(short_hash(""), "");
        assert_eq!(short_hash("abc"), "abc");
    }
}
