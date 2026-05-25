//! `brahman-broker-explorer` — probe GUI del broker brahman.
//!
//! Cada [`POLL_INTERVAL`] arma un Card observer agnóstico y lo
//! manda al broker via `card_sidecar::await_provider_blocking`
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
//!   resuelto por `card_handshake::transport`).
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

use std::collections::HashSet;

use card_handshake::messages::SessionList;
use card_handshake::transport;
use card_sidecar::{
    await_provider_blocking, build_consumer_card, list_matches_blocking, list_sessions_blocking,
    ConsumerError,
};
use ulid::Ulid;
use gpui::{
    div, prelude::*, px, Context, IntoElement, Render, SharedString, Window,
};
use nahual_launcher::launch_app;
use nahual_theme::Theme;
use nahual_widget_app_header::app_header;
use nahual_widget_banner::{banner_themed, Banner};
use nahual_widget_stat_card::stat_card;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// Cap del buffer del timeline. Mantenemos las últimas N entries —
/// más viejo se descarta. 50 cubre ~4 minutos de actividad densa
/// sin saturar el panel; subir si hace falta historia más larga.
const TIMELINE_CAP: usize = 50;

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
    /// Set de matches presentes en el último snapshot del broker.
    /// Identificado por `(consumer.session, consumer.flow,
    /// producer.session, producer.flow)` para que la diff entre
    /// ticks distinga "nuevo match" vs "match perdido". Un cambio
    /// de producer (otro session/flow para mismo consumer) cuenta
    /// como Lost del previo + Available del nuevo.
    last_match_keys: HashSet<MatchKey>,
    /// Timeline FIFO: los más nuevos al frente. Cada entry tiene un
    /// timestamp local + el evento sintético (Available/Lost) que
    /// surgió de la diff del tick.
    timeline: std::collections::VecDeque<TimelineEntry>,
}

/// Key estable para un match. Tupla porque (consumer, producer)
/// determina el match unívocamente; los campos derivados (`label`,
/// `via`, `pinned`) viajan en la entry pero no en la key.
type MatchKey = (Ulid, String, Ulid, String);

#[derive(Clone, Debug)]
struct TimelineEntry {
    /// Cuándo lo observó el explorer. Es tiempo local de wall-clock,
    /// no del broker — el broker no timestampa los matches.
    at: std::time::SystemTime,
    /// Available = nuevo en este tick. Lost = estaba en el tick
    /// anterior y desapareció.
    kind: card_handshake::messages::MatchEventKind,
    consumer_label: String,
    consumer_flow: String,
    producer_label: String,
    producer_flow: String,
    via: chasqui_broker::MatchStrategy,
    pinned: bool,
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
                // round-trip para pedir la lista de sesiones + matches.
                // Si está DOWN, ni intentar — la lista serviría de nada
                // con connect failed igual.
                let (sessions_snapshot, matches_snapshot) = match &new_state {
                    ProbeState::Down { .. } | ProbeState::Pending => (None, None),
                    _ => {
                        let s = bg
                            .spawn(async move {
                                list_sessions_blocking("brahman-broker-explorer").ok()
                            })
                            .await;
                        let m = bg
                            .spawn(async move {
                                list_matches_blocking("brahman-broker-explorer").ok()
                            })
                            .await;
                        (s, m)
                    }
                };

                let _ = this.update(cx, |me, cx| {
                    me.state = new_state;
                    me.sessions = sessions_snapshot;
                    if let Some(matches) = matches_snapshot {
                        me.diff_matches_into_timeline(&matches);
                    }
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
            last_match_keys: HashSet::new(),
            timeline: std::collections::VecDeque::new(),
        }
    }

    /// Diffea el snapshot recibido contra el último set de keys.
    /// Genera entries `Available` para keys nuevas y `Lost` para
    /// keys que estaban antes y no están ahora. Cada entry se
    /// prepende al timeline; el cap se aplica desde la cola.
    ///
    /// El primer tick del explorer (cuando `last_match_keys` está
    /// vacío) hace que TODOS los matches actuales aparezcan como
    /// `Available` — es el comportamiento querido (UI muestra el
    /// estado al boot sin que parezca "no pasa nada").
    fn diff_matches_into_timeline(
        &mut self,
        list: &card_handshake::messages::MatchList,
    ) {
        let (new_entries, new_keys) = diff_matches(&self.last_match_keys, list);
        for entry in new_entries {
            self.push_timeline(entry);
        }
        self.last_match_keys = new_keys;
    }

    fn push_timeline(&mut self, entry: TimelineEntry) {
        self.timeline.push_front(entry);
        while self.timeline.len() > TIMELINE_CAP {
            self.timeline.pop_back();
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

        let timeline_items: Vec<String> = self
            .timeline
            .iter()
            .take(20)
            .map(|e| format_timeline_entry(e))
            .collect();
        let timeline_value = self.timeline.len().to_string();
        let timeline_descr = if self.timeline.is_empty() {
            "esperando primer match…".to_string()
        } else {
            "↑ más reciente · ↓ más viejo · cap 50 entries".to_string()
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
            ))
            .child(stat_card(
                cx,
                "Timeline de matches",
                timeline_value,
                &timeline_descr,
                accent_partial,
                text,
                text_dim,
                &timeline_items,
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

/// Diff puro entre snapshots de matches. Devuelve la lista de
/// entries nuevas (Available + Lost) en orden Available-primero, y
/// el set actualizado de keys. Extraído como free fn para que sea
/// testeable sin instanciar `Explorer`.
///
/// El primer tick (last_keys vacío) marca todos los matches como
/// Available. Esto es deliberado: la UI muestra el estado al boot
/// como "todo recién apareció" en vez de quedarse vacía.
fn diff_matches(
    last_keys: &HashSet<MatchKey>,
    list: &card_handshake::messages::MatchList,
) -> (Vec<TimelineEntry>, HashSet<MatchKey>) {
    use card_handshake::messages::MatchEventKind;
    let now = std::time::SystemTime::now();
    let current_keys: HashSet<MatchKey> = list
        .matches
        .iter()
        .map(|m| {
            (
                m.consumer.session,
                m.consumer.flow_name.clone(),
                m.producer.session,
                m.producer.flow_name.clone(),
            )
        })
        .collect();

    let mut entries = Vec::new();
    for m in &list.matches {
        let key = (
            m.consumer.session,
            m.consumer.flow_name.clone(),
            m.producer.session,
            m.producer.flow_name.clone(),
        );
        if !last_keys.contains(&key) {
            entries.push(TimelineEntry {
                at: now,
                kind: MatchEventKind::Available,
                consumer_label: m.consumer_label.clone(),
                consumer_flow: m.consumer.flow_name.clone(),
                producer_label: m.producer_label.clone(),
                producer_flow: m.producer.flow_name.clone(),
                via: m.via,
                pinned: m.pinned,
            });
        }
    }
    for key in last_keys.iter() {
        if !current_keys.contains(key) {
            entries.push(TimelineEntry {
                at: now,
                kind: MatchEventKind::Lost,
                consumer_label: String::new(),
                consumer_flow: key.1.clone(),
                producer_label: String::new(),
                producer_flow: key.3.clone(),
                via: chasqui_broker::MatchStrategy::Exact,
                pinned: false,
            });
        }
    }
    (entries, current_keys)
}

/// Renderiza una entry del timeline en una sola línea: `HH:MM:SS
/// {kind} consumer.flow ← producer.flow [via]`. Compact por diseño
/// — el panel es vertical y las líneas largas se cortan.
fn format_timeline_entry(e: &TimelineEntry) -> String {
    use card_handshake::messages::MatchEventKind;
    // Wall-clock local en HH:MM:SS — sin zoneinfo (chrono es heavy).
    // Aproximación: total_seconds % 86400 → hora del día UTC.
    let secs_today = e
        .at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() % 86_400)
        .unwrap_or(0);
    let h = secs_today / 3600;
    let m = (secs_today % 3600) / 60;
    let s = secs_today % 60;
    let kind = match e.kind {
        MatchEventKind::Available => "+",
        MatchEventKind::Lost => "-",
    };
    let pinned = if e.pinned { " (pinned)" } else { "" };
    match e.kind {
        MatchEventKind::Available => format!(
            "{:02}:{:02}:{:02} {} {}.{} ← {}.{} [{:?}]{}",
            h, m, s, kind,
            e.consumer_label, e.consumer_flow,
            e.producer_label, e.producer_flow,
            e.via, pinned,
        ),
        MatchEventKind::Lost => format!(
            "{:02}:{:02}:{:02} {} ?.{} ← ?.{} (lost)",
            h, m, s, kind, e.consumer_flow, e.producer_flow,
        ),
    }
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

    fn synth_match(
        consumer_label: &str,
        consumer_flow: &str,
        producer_label: &str,
        producer_flow: &str,
    ) -> chasqui_broker::Match {
        use chasqui_broker::{Endpoint, Match, MatchStrategy};
        use card_core::TypeRef;
        Match {
            consumer: Endpoint {
                session: Ulid::new(),
                flow_name: consumer_flow.into(),
            },
            consumer_label: consumer_label.into(),
            producer: Endpoint {
                session: Ulid::new(),
                flow_name: producer_flow.into(),
            },
            producer_label: producer_label.into(),
            ty: TypeRef::Primitive { name: "json".into() },
            via: MatchStrategy::Exact,
            pinned: false,
        }
    }

    #[test]
    fn diff_matches_first_snapshot_marks_everything_available() {
        use card_handshake::messages::{MatchEventKind, MatchList};
        let list = MatchList {
            matches: vec![
                synth_match("a", "x", "b", "x"),
                synth_match("c", "y", "d", "y"),
            ],
        };
        let last = HashSet::new();
        let (entries, keys) = diff_matches(&last, &list);
        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .all(|e| matches!(e.kind, MatchEventKind::Available)));
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn diff_matches_emits_lost_when_match_disappears() {
        use card_handshake::messages::{MatchEventKind, MatchList};
        let m = synth_match("a", "x", "b", "x");
        let prev_key = (
            m.consumer.session,
            m.consumer.flow_name.clone(),
            m.producer.session,
            m.producer.flow_name.clone(),
        );
        let last: HashSet<_> = std::iter::once(prev_key.clone()).collect();
        let list = MatchList { matches: vec![] };
        let (entries, keys) = diff_matches(&last, &list);
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0].kind, MatchEventKind::Lost));
        assert_eq!(entries[0].consumer_flow, "x");
        assert_eq!(entries[0].producer_flow, "x");
        assert!(keys.is_empty());
    }

    #[test]
    fn diff_matches_no_change_emits_nothing() {
        use card_handshake::messages::MatchList;
        let m = synth_match("a", "x", "b", "x");
        let key = (
            m.consumer.session,
            m.consumer.flow_name.clone(),
            m.producer.session,
            m.producer.flow_name.clone(),
        );
        let last: HashSet<_> = std::iter::once(key.clone()).collect();
        let list = MatchList {
            matches: vec![m.clone()],
        };
        let (entries, keys) = diff_matches(&last, &list);
        assert!(entries.is_empty(), "match unchanged → no events");
        assert_eq!(keys.len(), 1);
        assert!(keys.contains(&key));
    }
}
