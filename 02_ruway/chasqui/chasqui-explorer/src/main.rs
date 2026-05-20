//! `chasqui-explorer` — panel GPUI que descubre al daemon `chasqui`
//! vía broker brahman y muestra sus Mónadas en vivo.
//!
//! Diseño: ventana standalone que cada N segundos consulta el query
//! socket del daemon (`chasqui_core::engine_socket::client::list_monads`).
//! El path del socket NO está hardcoded — se descubre vía
//! `brahman_sidecar::await_provider_blocking` para el flow
//! `monad-list:json`. Si el daemon cae, el socket cacheado se invalida
//! y la próxima iteración re-descubre.
//!
//! Sin integración con nahual-shell — es su propio binario para que el
//! ecosistema sea visible incluso sin la shell completa.
//!
//! Uso:
//! ```sh
//! cargo run -p chasqui-explorer
//! # con override del init socket (heredado de brahman-handshake):
//! BRAHMAN_INIT_SOCKET=/tmp/init.sock cargo run -p chasqui-explorer
//! ```

use std::path::PathBuf;
use std::time::Duration;

use brahman_sidecar::{await_provider_blocking, build_consumer_card, ConsumerError};
use gpui::{
    div, prelude::*, px, rgb, Context, IntoElement, Render, SharedString, Window,
};
use chasqui_card::query::client as query_client;
use chasqui_card::query::{transport, ListMonadsResponse, FLOW_MONAD_LIST, FLOW_TYPE_NAME};
use chasqui_card::Lens;
use nahual_launcher::launch_app;
use nahual_theme::Theme;
use nahual_widget_app_header::app_header;
use nahual_widget_banner::{banner_themed, Banner};
use nahual_widget_card::card_themed;

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);

fn main() {
    launch_app("Nouser — Mónadas", (900., 640.), Explorer::new);
}

/// Vista raíz: cachea el socket descubierto, el último snapshot y el
/// último error. El socket cacheado se invalida ante cualquier fallo
/// de query, forzando re-discovery en la próxima iteración.
struct Explorer {
    socket: Option<PathBuf>,
    snapshot: Option<ListMonadsResponse>,
    error: Option<SharedString>,
    /// Última fuente del socket activo: "discovery" (vía broker) o
    /// "cache" (reusando el de la iteración anterior). Sólo informativo.
    socket_source: Option<&'static str>,
}

impl Explorer {
    fn new(cx: &mut Context<Self>) -> Self {
        // Loop de refresh: cada `REFRESH_INTERVAL`:
        // 1. Si no tenemos socket cacheado → discovery vía broker.
        // 2. Si tenemos → query directo. Fallo → invalida cache.
        cx.spawn(async move |this, cx| {
            let timer = cx.background_executor().clone();
            loop {
                let prior_socket = this
                    .read_with(cx, |me, _| me.socket.clone())
                    .ok()
                    .flatten();

                let result = tick(prior_socket);

                let _ = this.update(cx, |me, cx| {
                    match result {
                        TickOutcome::Ok { socket, source, snapshot } => {
                            me.socket = Some(socket);
                            me.socket_source = Some(source);
                            me.snapshot = Some(snapshot);
                            me.error = None;
                        }
                        TickOutcome::DiscoveryFailed(msg) => {
                            me.socket = None;
                            me.socket_source = None;
                            me.error = Some(SharedString::from(msg));
                        }
                        TickOutcome::QueryFailed(msg) => {
                            // Invalidamos el socket cacheado: la
                            // próxima iteración re-descubre.
                            me.socket = None;
                            me.socket_source = None;
                            me.error = Some(SharedString::from(msg));
                        }
                    }
                    cx.notify();
                });
                timer.timer(REFRESH_INTERVAL).await;
            }
        })
        .detach();

        Self {
            socket: None,
            snapshot: None,
            error: None,
            socket_source: None,
        }
    }
}

enum TickOutcome {
    Ok {
        socket: PathBuf,
        source: &'static str,
        snapshot: ListMonadsResponse,
    },
    DiscoveryFailed(String),
    QueryFailed(String),
}

/// Resuelve el socket (cache → broker → default path) y consulta
/// `ListMonads`. Pensado para correr en background: no toca GPUI,
/// sólo I/O.
///
/// **Falla hacia la simplicidad**: si el broker brahman no está vivo
/// (init caído / no instalado), intentamos directo el path canónico
/// del daemon vía `transport::default_socket_path()`. El explorer
/// sigue funcionando contra un daemon "huérfano" que no se publicó
/// al broker — útil para correr la UI sin todo el stack.
fn tick(prior_socket: Option<PathBuf>) -> TickOutcome {
    let (socket, source) = match prior_socket {
        Some(p) => (p, "cache"),
        None => match resolve_socket() {
            Ok(found) => found,
            Err(e) => return TickOutcome::DiscoveryFailed(e),
        },
    };

    match query_client::list_monads(&socket, QUERY_TIMEOUT) {
        Ok(resp) => TickOutcome::Ok {
            socket,
            source,
            snapshot: resp,
        },
        Err(e) => TickOutcome::QueryFailed(format!(
            "query a {}: {e} — re-descubriendo en próxima iteración",
            socket.display()
        )),
    }
}

/// Resuelve el socket del daemon en dos pasos:
/// 1. **Broker**: consumer Card + `await_provider_blocking`. Path
///    "consciente" (ecosistema brahman activo).
/// 2. **Default path**: si el broker no responde, probamos
///    `transport::default_socket_path()` directo. Path "soberano"
///    (daemon corriendo solo, sin init).
///
/// Falla únicamente si ninguno responde.
fn resolve_socket() -> Result<(PathBuf, &'static str), String> {
    match discover_via_broker() {
        Ok(p) => Ok((p, "broker")),
        Err(broker_err) => {
            let fallback = transport::default_socket_path();
            if fallback.exists() {
                Ok((fallback, "default-path"))
            } else {
                Err(format!(
                    "broker: {broker_err}; fallback {} no existe",
                    fallback.display()
                ))
            }
        }
    }
}

/// Discovery del daemon vía broker brahman. Construye un consumer
/// Card con `flow.input = monad-list:json`, espera al primer
/// `MatchEvent::Available`, devuelve el `producer_service_socket`.
fn discover_via_broker() -> Result<PathBuf, ConsumerError> {
    let card = build_consumer_card("chasqui-explorer", FLOW_MONAD_LIST, FLOW_TYPE_NAME);
    await_provider_blocking(card, DISCOVERY_TIMEOUT)
}

impl Render for Explorer {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Chrome viene del Theme global; los acentos por kind
        // (engine cyan, data purple) son señales semánticas del
        // dominio chasqui y se mantienen locales.
        let theme = Theme::global(cx).clone();
        let bg = theme.bg_app.clone();
        let text = theme.fg_text;
        let text_dim = theme.fg_muted;
        let accent_engine = rgb(0x88c0d0);
        let accent_data = rgb(0xb48ead);

        let header_text = match (&self.snapshot, &self.socket, self.socket_source) {
            (Some(s), Some(sock), Some(src)) => format!(
                "Engine '{}'  ·  {} mónada(s)  ·  socket: {} ({}){}",
                s.engine.label,
                s.monads.len(),
                sock.display(),
                src,
                s.engine
                    .watching
                    .as_deref()
                    .map(|w| format!("  ·  watching: {}", w))
                    .unwrap_or_default()
            ),
            _ => "Buscando daemon chasqui vía brahman-broker…".to_string(),
        };

        // Header standard via widget compartido.
        let header = app_header(cx, header_text);

        let error_banner = self.error.as_ref().map(|e| {
            banner_themed(cx, Banner::Error, e.clone())
                .px(px(16.))
                .py(px(8.))
                .text_size(px(12.))
        });

        let cards: Vec<gpui::AnyElement> = match &self.snapshot {
            None => vec![],
            Some(snap) => {
                let mut out = Vec::with_capacity(snap.monads.len() + 1);

                // Engine card primero — el "ser" que owns las Mónadas.
                out.push(
                    card_themed(cx)
                        .border_l_4()
                        .border_color(accent_engine)
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(8.))
                                .items_center()
                                .child(
                                    div()
                                        .text_color(accent_engine)
                                        .text_size(px(11.))
                                        .child("[engine]"),
                                )
                                .child(
                                    div()
                                        .text_color(text)
                                        .text_size(px(15.))
                                        .child(snap.engine.label.clone()),
                                ),
                        )
                        .child(
                            div()
                                .text_color(text_dim)
                                .text_size(px(11.))
                                .child(format!("id: {}", snap.engine.id)),
                        )
                        .when_some(snap.engine.watching.clone(), |d, w| {
                            d.child(
                                div()
                                    .text_color(text_dim)
                                    .text_size(px(11.))
                                    .child(format!("watching: {w}")),
                            )
                        })
                        .into_any_element(),
                );

                // Mónadas (kind=Data por construcción).
                for m in &snap.monads {
                    let lens = lens_label(m.dominant_lens);
                    let keywords = m.keywords.join(", ");
                    let path_hint_line = m
                        .path_hint
                        .as_deref()
                        .filter(|p| !p.is_empty())
                        .map(|p| format!("path: {p}"));
                    let model_line = m
                        .centroid_model
                        .as_deref()
                        .filter(|m| !m.is_empty())
                        .map(|m| format!("model: {m}"));

                    out.push(
                        card_themed(cx)
                            .border_l_4()
                            .border_color(accent_data)
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(8.))
                                    .items_center()
                                    .child(
                                        div()
                                            .text_color(accent_data)
                                            .text_size(px(11.))
                                            .child("[monad]"),
                                    )
                                    .child(
                                        div()
                                            .text_color(text)
                                            .text_size(px(15.))
                                            .child(m.label.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_color(text_dim)
                                            .text_size(px(11.))
                                            .child(format!(
                                                "{} files · ent {:.2} · {}",
                                                m.cardinality, m.entropy, lens
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .text_color(text_dim)
                                    .text_size(px(11.))
                                    .child(format!("id: {}", m.id)),
                            )
                            .when(!m.summary.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_color(text)
                                        .text_size(px(12.))
                                        .child(m.summary.clone()),
                                )
                            })
                            .when(!keywords.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_color(text_dim)
                                        .text_size(px(11.))
                                        .child(format!("keywords: {keywords}")),
                                )
                            })
                            .when_some(path_hint_line, |d, line| {
                                d.child(
                                    div()
                                        .text_color(text_dim)
                                        .text_size(px(11.))
                                        .child(line),
                                )
                            })
                            .when_some(model_line, |d, line| {
                                d.child(
                                    div()
                                        .text_color(text_dim)
                                        .text_size(px(11.))
                                        .child(line),
                                )
                            })
                            .into_any_element(),
                    );
                }
                out
            }
        };

        let body = div()
            .flex()
            .flex_col()
            .p(px(16.))
            .overflow_hidden()
            .children(cards);

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

fn lens_label(l: Lens) -> &'static str {
    match l {
        Lens::Grid => "grid",
        Lens::Code => "code",
        Lens::Gallery => "gallery",
        Lens::Database => "database",
        Lens::Markdown => "markdown",
        Lens::Tree => "tree",
    }
}
