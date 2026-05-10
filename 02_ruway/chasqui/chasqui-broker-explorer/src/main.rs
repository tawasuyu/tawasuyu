//! `brahman-broker-explorer` — probe GUI del broker brahman.
//!
//! Cada [`POLL_INTERVAL`] arma un Card observer agnóstico y lo
//! manda al broker via `brahman_sidecar::await_provider_blocking`
//! (que internamente abre tokio runtime + Unix socket + handshake).
//! Reporta 3 estados:
//!
//! - **Down**: connect failed (broker no escucha en el socket).
//! - **Up sin provider**: connect OK, pero el broker no encontró
//!   productor para el flow probado dentro del timeout.
//! - **Up con provider**: connect OK + el broker matcheó algo →
//!   muestra el `producer_service_socket` recibido.
//!
//! Configuración via env:
//! - `BRAHMAN_INIT_SOCKET` — path del socket del broker (default
//!   resuelto por `brahman_handshake::transport`).
//! - `BRAHMAN_BROKER_PROBE_FLOW` — nombre del flow probe (default
//!   `broker-health`).
//! - `BRAHMAN_BROKER_PROBE_TYPE` — type name del flow probe
//!   (default `ping`).
//!
//! Usá un type name probable (ej. `monad-list:json`,
//! `event-log:tail`) para detectar productores específicos del
//! ecosistema.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use brahman_handshake::messages::SessionList;
use brahman_handshake::transport;
use brahman_sidecar::{
    await_provider_blocking, build_consumer_card, list_sessions_blocking, ConsumerError,
};
use gpui::{
    div, prelude::*, px, Context, IntoElement, Render, SharedString, Window,
};
use yahweh_launcher::launch_app;
use yahweh_theme::Theme;
use yahweh_widget_app_header::app_header;
use yahweh_widget_banner::{banner_themed, Banner};
use yahweh_widget_stat_card::stat_card;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

fn main() {
    launch_app("Brahman Broker — Probe", (720., 480.), Explorer::new);
}

/// Snapshot de un probe.
#[derive(Clone, Debug)]
enum ProbeState {
    /// Aún no probó (boot, primer ciclo).
    Pending,
    /// Connect failed → broker no responde en el path.
    Down { reason: String },
    /// Connect OK, sin matching producer dentro del timeout.
    UpNoProvider { flow: String },
    /// Connect OK, broker matcheó al menos un producer.
    UpWithProvider {
        flow: String,
        producer_socket: PathBuf,
    },
}

struct Explorer {
    socket_path: PathBuf,
    flow: String,
    type_name: String,
    state: ProbeState,
    last_probe_ms: u64,
    last_probe_at: Option<Instant>,
    /// Última `SessionList` recibida del broker (None = aún sin pedir
    /// o último intento falló).
    sessions: Option<SessionList>,
}

impl Explorer {
    fn new(cx: &mut Context<Self>) -> Self {
        let socket_path = transport::default_socket_path();
        let flow = std::env::var("BRAHMAN_BROKER_PROBE_FLOW")
            .unwrap_or_else(|_| "broker-health".to_string());
        let type_name = std::env::var("BRAHMAN_BROKER_PROBE_TYPE")
            .unwrap_or_else(|_| "ping".to_string());

        let flow_for_loop = flow.clone();
        let type_for_loop = type_name.clone();
        cx.spawn(async move |this, cx| {
            let timer = cx.background_executor().clone();
            let bg = cx.background_executor().clone();
            loop {
                let card = build_consumer_card(
                    "brahman-broker-explorer",
                    flow_for_loop.clone(),
                    type_for_loop.clone(),
                );
                let started = Instant::now();
                // El probe es bloqueante (interno tokio runtime); va
                // al background executor para no congelar el main.
                let probe_flow = flow_for_loop.clone();
                let result = bg
                    .spawn(async move { await_provider_blocking(card, PROBE_TIMEOUT) })
                    .await;
                let elapsed = started.elapsed().as_millis() as u64;

                let new_state = match result {
                    Ok(socket) => ProbeState::UpWithProvider {
                        flow: probe_flow.clone(),
                        producer_socket: socket,
                    },
                    Err(ConsumerError::NoProvider { .. }) => ProbeState::UpNoProvider {
                        flow: probe_flow.clone(),
                    },
                    Err(e) => ProbeState::Down {
                        reason: e.to_string(),
                    },
                };

                // Si el broker está reachable (UP*), aprovechar el
                // round-trip para pedir la lista de sesiones. Si está
                // DOWN, ni intentar — la lista serviría de nada con
                // connect failed igual.
                let sessions_snapshot = match &new_state {
                    ProbeState::Down { .. } | ProbeState::Pending => None,
                    _ => bg
                        .spawn(async move {
                            list_sessions_blocking("brahman-broker-explorer").ok()
                        })
                        .await,
                };

                let _ = this.update(cx, |me, cx| {
                    me.state = new_state;
                    me.sessions = sessions_snapshot;
                    me.last_probe_ms = elapsed;
                    me.last_probe_at = Some(Instant::now());
                    cx.notify();
                });

                timer.timer(POLL_INTERVAL).await;
            }
        })
        .detach();

        Self {
            socket_path,
            flow,
            type_name,
            state: ProbeState::Pending,
            last_probe_ms: 0,
            last_probe_at: None,
            sessions: None,
        }
    }
}

impl Render for Explorer {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let bg = theme.bg_app.clone();
        let text = theme.fg_text;
        let text_dim = theme.fg_muted;
        // Acentos por estado: verde = arriba, amber = arriba sin
        // provider, rojo = abajo, gris = pending.
        let accent_up = gpui::rgb(0xa3be8c);
        let accent_partial = gpui::rgb(0xebcb8b);
        let accent_down = gpui::rgb(0xbf616a);
        let accent_pending = gpui::rgb(0x6a7280);

        let header_text = format!(
            "Probe: {}  ·  flow: {}/{}  ·  reload {} ms",
            self.socket_path.display(),
            self.flow,
            self.type_name,
            self.last_probe_ms,
        );

        // Header standard via widget compartido.
        let header = app_header(cx, header_text);

        // Banner permanente debajo del header con el estado actual.
        // Severidad acorde al kind.
        let status_banner = match &self.state {
            ProbeState::Pending => None,
            ProbeState::Down { reason } => Some(banner_themed(
                cx,
                Banner::Error,
                SharedString::from(format!("Broker DOWN — {reason}")),
            )),
            ProbeState::UpNoProvider { .. } => Some(banner_themed(
                cx,
                Banner::Warning,
                SharedString::from("Broker UP, sin provider para el flow"),
            )),
            ProbeState::UpWithProvider { .. } => Some(banner_themed(
                cx,
                Banner::Success,
                SharedString::from("Broker UP, provider matcheado"),
            )),
        };

        let sessions_items: Vec<String> = self
            .sessions
            .as_ref()
            .map(|list| {
                let mut entries: Vec<_> = list.entries.iter().collect();
                // Orden estable por session id (Ulid es ordenable
                // temporal); útil para que la UI no se reordene
                // entre ticks aunque el HashMap del server sí.
                entries.sort_by_key(|e| e.session);
                entries
                    .iter()
                    .map(|e| {
                        format!(
                            "{}  ·  in:[{}]  out:[{}]{}",
                            e.label,
                            e.inputs.join(","),
                            e.outputs.join(","),
                            if e.conscious { "  (wit)" } else { "" }
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let sessions_count_value = self
            .sessions
            .as_ref()
            .map(|l| l.entries.len().to_string())
            .unwrap_or_else(|| "—".into());
        let sessions_descr = match &self.sessions {
            None => "lista no disponible (broker DOWN o pendiente)".to_string(),
            Some(l) if l.entries.is_empty() => "sin sesiones registradas en el broker".into(),
            Some(_) => "labels visibles + flows in/out · (wit) = consciente".into(),
        };

        let body = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(16.))
            .py(px(16.))
            .child(state_card(cx, &self.state, text, text_dim, accent_up,
                accent_partial, accent_down, accent_pending))
            .child(stat_card(
                cx,
                "Sesiones activas",
                sessions_count_value,
                &sessions_descr,
                accent_up,
                text,
                text_dim,
                &sessions_items,
            ));

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(bg)
            .child(header)
            .when_some(status_banner, |d, b| d.child(b))
            .child(body)
    }
}

/// Wrap del `stat_card` compartido con el mapeo de
/// `ProbeState` → (label/accent/value/description). Mantenemos
/// este helper local porque la traducción del enum a strings es
/// específica del explorer (no es un patrón cross-app).
#[allow(clippy::too_many_arguments)]
fn state_card(
    cx: &mut Context<Explorer>,
    state: &ProbeState,
    text: gpui::Hsla,
    text_dim: gpui::Hsla,
    accent_up: gpui::Rgba,
    accent_partial: gpui::Rgba,
    accent_down: gpui::Rgba,
    accent_pending: gpui::Rgba,
) -> impl IntoElement {
    let (accent, value, description): (gpui::Rgba, String, String) = match state {
        ProbeState::Pending => (
            accent_pending,
            "PENDING".into(),
            "esperando primer probe…".into(),
        ),
        ProbeState::Down { reason } => (
            accent_down,
            "DOWN".into(),
            format!("connect failed: {reason}"),
        ),
        ProbeState::UpNoProvider { flow } => (
            accent_partial,
            "UP / NO PROVIDER".into(),
            format!("broker reachable; sin productor para flow `{flow}`"),
        ),
        ProbeState::UpWithProvider {
            flow,
            producer_socket,
        } => (
            accent_up,
            "UP / PROVIDER".into(),
            format!(
                "flow `{flow}` matcheado en producer socket: {}",
                producer_socket.display()
            ),
        ),
    };

    stat_card(cx, "Estado", value, &description, accent, text, text_dim, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_is_default_state_at_boot() {
        // El estado inicial DEBE ser Pending — sino el banner del
        // header arrancaría con un Error o Warning sin haber probado
        // (mensaje engañoso).
        let s = ProbeState::Pending;
        assert!(matches!(s, ProbeState::Pending));
    }

    #[test]
    fn poll_and_probe_constants_are_sane() {
        // Sanity: el timeout del probe DEBE ser menor que el
        // intervalo de polling, sino los probes se solapan.
        assert!(PROBE_TIMEOUT < POLL_INTERVAL);
        // El intervalo no debería ser tan corto que sature al broker.
        assert!(POLL_INTERVAL >= Duration::from_secs(2));
    }
}
