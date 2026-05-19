//! `minga-explorer` — dashboard GPUI del repo Minga (VCS semántico
//! P2P).
//!
//! Polling cada 2s contra `MINGA_REPO` (env, default `./.minga`),
//! abre el `PersistentRepo` (sled, sin passphrase porque los counts
//! son lectura pública) y muestra:
//! - Cantidad de nodos AST almacenados.
//! - Cantidad de atestaciones firmadas.
//! - Cantidad de claves del MST (Merkle Search Tree).
//!
//! No requiere keypair descifrado — eso se queda para el CLI
//! (`minga status`) cuando hace falta el DID. El explorer foco es
//! observabilidad rápida.
//!
//! Stack visual: nahual-theme + banner_themed + card_themed +
//! theme_switcher. Mismo patrón que `nakui-explorer` /
//! `akasha-explorer`.
//!
//! Uso:
//! ```sh
//! cargo run -p minga-explorer
//! # con repo custom:
//! MINGA_REPO=/path/to/.minga cargo run -p minga-explorer
//! ```

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    div, prelude::*, px, Context, IntoElement, Render, SharedString, Window,
};
use minga_store::PersistentRepo;
use nahual_launcher::launch_app;
use nahual_theme::Theme;
use nahual_widget_app_header::app_header;
use nahual_widget_banner::{banner_themed, Banner};
use nahual_widget_stat_card::stat_card;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const REPO_DIRNAME: &str = "repo";

fn main() {
    launch_app("Minga — Repo", (800., 560.), Explorer::new);
}

/// Cuántos items recientes mostrar por sección. Los stores no
/// tienen orden cronológico (sled ordena lexicográfico por hash);
/// los "recent" acá son simplemente los primeros del iter — sirve
/// como sample, no como log temporal. Para timeline real haría
/// falta agregar timestamp al schema.
const RECENT_LIMIT: usize = 5;

/// Snapshot de counts + sample de items recientes. Reemplaza el
/// completo en cada refresh — los stores no diff fácilmente y los
/// counts son baratos (sled tracks size).
#[derive(Clone, Default, Debug)]
struct RepoSnapshot {
    nodes: usize,
    attestations: usize,
    mst_keys: usize,
    /// Sample de nodos: `(hash_short, kind)`.
    recent_nodes: Vec<(String, String)>,
    /// Sample de atestaciones: `(content_hash_short, did_short)`.
    recent_attestations: Vec<(String, String)>,
    /// Sample de claves MST: `hash_short`.
    recent_mst_keys: Vec<String>,
}

struct Explorer {
    repo_path: PathBuf,
    snapshot: Option<RepoSnapshot>,
    error: Option<SharedString>,
    last_load_ms: u64,
}

impl Explorer {
    fn new(cx: &mut Context<Self>) -> Self {
        let repo_path = std::env::var("MINGA_REPO")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".minga"));
        let path_for_loop = repo_path.clone();
        cx.spawn(async move |this, cx| {
            let timer = cx.background_executor().clone();
            loop {
                let started = std::time::Instant::now();
                let result = load_snapshot(&path_for_loop);
                let elapsed = started.elapsed().as_millis() as u64;
                let _ = this.update(cx, |me, cx| {
                    match result {
                        Ok(snap) => {
                            me.snapshot = Some(snap);
                            me.error = None;
                        }
                        Err(e) => {
                            me.error = Some(SharedString::from(format!(
                                "no pude leer repo {}: {}",
                                me.repo_path.display(),
                                e
                            )));
                        }
                    }
                    me.last_load_ms = elapsed;
                    cx.notify();
                });
                timer.timer(REFRESH_INTERVAL).await;
            }
        })
        .detach();

        Self {
            repo_path,
            snapshot: None,
            error: None,
            last_load_ms: 0,
        }
    }
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

    // Counts: cheap (sled tracks size).
    let nodes = repo.nodes.len();
    let attestations = repo.attestations.len();
    let mst_keys = repo.mst.len();

    // Samples: tomar los primeros RECENT_LIMIT items del iter.
    // Errores per-item se silencian (filter_map) porque el dashboard
    // muestra lo que pueda; un par de items corruptos no debería
    // tirar el panel entero.
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

impl Render for Explorer {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let bg = theme.bg_app.clone();
        let text = theme.fg_text;
        let text_dim = theme.fg_muted;
        // Acentos por kind del dashboard: nodos azul, atestaciones
        // verde, MST purple. Señales semánticas del dominio Minga.
        let accent_nodes = gpui::rgb(0x88c0d0);
        let accent_attestations = gpui::rgb(0xa3be8c);
        let accent_mst = gpui::rgb(0xb48ead);

        let header_text = match &self.snapshot {
            Some(_) => format!(
                "Repo: {}  ·  reload {} ms",
                self.repo_path.display(),
                self.last_load_ms
            ),
            None => format!("Buscando repo en {}…", self.repo_path.display()),
        };

        // Header standard via widget compartido.
        let header = app_header(cx, header_text);

        let error_banner = self.error.as_ref().map(|e| {
            banner_themed(cx, Banner::Error, e.clone())
                .px(px(16.))
                .py(px(8.))
                .text_size(px(12.))
        });

        let body = match &self.snapshot {
            None => div()
                .px(px(16.))
                .py(px(20.))
                .text_color(text_dim)
                .text_size(px(13.))
                .child("Esperando primer refresh…"),
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

                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .px(px(16.))
                    .py(px(16.))
                    .child(stat_card(
                        cx,
                        "Nodos AST",
                        snap.nodes.to_string(),
                        "fragments parseados del código",
                        accent_nodes,
                        text,
                        text_dim,
                        &node_items,
                    ))
                    .child(stat_card(
                        cx,
                        "Atestaciones",
                        snap.attestations.to_string(),
                        "firmas Ed25519 sobre los nodos",
                        accent_attestations,
                        text,
                        text_dim,
                        &attestation_items,
                    ))
                    .child(stat_card(
                        cx,
                        "Claves MST",
                        snap.mst_keys.to_string(),
                        "entradas del Merkle Search Tree",
                        accent_mst,
                        text,
                        text_dim,
                        &mst_items,
                    ))
            }
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(bg)
            .child(header)
            .when_some(error_banner, |d, b| d.child(b))
            .child(body)
    }
}

// `stat_card` se promovió a `nahual-widget-stat-card` y se importa
// arriba. La fn local fue eliminada en la iter 15 del refactor.

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity: load_snapshot rebota si el dir no existe (mensaje
    /// claro). Es el path típico para "no inicializaste el repo".
    #[test]
    fn load_snapshot_errors_on_missing_dir() {
        let p = std::env::temp_dir().join(format!(
            "minga-explorer-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        // p NO existe.
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
