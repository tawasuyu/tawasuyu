//! `chasqui-broker-explorer-llimphi` — probe Llimphi del broker
//! brahman.
//!
//! Cada [`POLL_INTERVAL`] arma un Card observer agnóstico y lo manda
//! al broker via `card_sidecar::await_provider_blocking` (que
//! internamente abre tokio runtime + Unix socket + handshake).
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

#![forbid(unsafe_code)]

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use card_handshake::messages::SessionList;
use card_handshake::transport;
use card_sidecar::{
    await_provider_blocking, build_consumer_card, list_matches_blocking, list_sessions_blocking,
    ConsumerError,
};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_stat_card::{stat_card_view, StatCardPalette};
use ulid::Ulid;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// Cap del buffer del timeline. Mantenemos las últimas N entries —
/// más viejo se descarta. 50 cubre ~4 minutos de actividad densa.
const TIMELINE_CAP: usize = 50;

#[derive(Clone, Debug)]
enum ProbeState {
    Pending,
    Down { reason: String },
    UpNoProvider { flow: String },
    UpWithProvider { flow: String, producer_socket: PathBuf },
}

#[derive(Clone, Debug)]
struct TimelineEntry {
    at: std::time::SystemTime,
    kind: card_handshake::messages::MatchEventKind,
    consumer_label: String,
    consumer_flow: String,
    producer_label: String,
    producer_flow: String,
    via: chasqui_broker::MatchStrategy,
    pinned: bool,
}

type MatchKey = (Ulid, String, Ulid, String);

struct Model {
    theme: Theme,
    socket_path: PathBuf,
    flow: String,
    type_name: String,
    state: ProbeState,
    last_probe_ms: u64,
    sessions: Option<SessionList>,
    last_match_keys: HashSet<MatchKey>,
    timeline: VecDeque<TimelineEntry>,
}

#[derive(Clone)]
enum Msg {
    Tick,
    ProbeResult {
        state: ProbeState,
        sessions: Option<SessionList>,
        matches: Option<card_handshake::messages::MatchList>,
        elapsed_ms: u64,
    },
}

struct Explorer;

impl App for Explorer {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Brahman Broker — Probe"
    }

    fn initial_size() -> (u32, u32) {
        (720, 480)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let flow = std::env::var("BRAHMAN_BROKER_PROBE_FLOW")
            .unwrap_or_else(|_| "broker-health".to_string());
        let type_name = std::env::var("BRAHMAN_BROKER_PROBE_TYPE")
            .unwrap_or_else(|_| "ping".to_string());

        handle.dispatch(Msg::Tick);
        handle.spawn_periodic(POLL_INTERVAL, || Msg::Tick);

        Model {
            theme: Theme::dark(),
            socket_path: transport::default_socket_path(),
            flow,
            type_name,
            state: ProbeState::Pending,
            last_probe_ms: 0,
            sessions: None,
            last_match_keys: HashSet::new(),
            timeline: VecDeque::new(),
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                let flow = m.flow.clone();
                let type_name = m.type_name.clone();
                handle.spawn(move || run_probe(flow, type_name));
            }
            Msg::ProbeResult { state, sessions, matches, elapsed_ms } => {
                m.state = state;
                m.sessions = sessions;
                if let Some(list) = matches {
                    m.diff_matches_into_timeline(&list);
                }
                m.last_probe_ms = elapsed_ms;
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        let header_palette = AppHeaderPalette::from_theme(theme);
        let stat_palette = StatCardPalette::from_theme(theme);

        // Acentos por estado.
        let accent_up = Color::from_rgba8(0xa3, 0xbe, 0x8c, 0xff);
        let accent_partial = Color::from_rgba8(0xeb, 0xcb, 0x8b, 0xff);
        let accent_down = Color::from_rgba8(0xbf, 0x61, 0x6a, 0xff);
        let accent_pending = Color::from_rgba8(0x6a, 0x72, 0x80, 0xff);

        let header_text = format!(
            "Probe: {}  ·  flow: {}/{}  ·  reload {} ms",
            model.socket_path.display(),
            model.flow,
            model.type_name,
            model.last_probe_ms,
        );

        let header = app_header::<Msg>(header_text, vec![], &header_palette);

        let mut body_children: Vec<View<Msg>> = Vec::new();

        // Banner permanente con el estado actual.
        match &model.state {
            ProbeState::Pending => {}
            ProbeState::Down { reason } => body_children.push(banner_view::<Msg>(
                BannerKind::Error,
                format!("Broker DOWN — {reason}"),
            )),
            ProbeState::UpNoProvider { .. } => body_children.push(banner_view::<Msg>(
                BannerKind::Warning,
                "Broker UP, sin provider para el flow".to_string(),
            )),
            ProbeState::UpWithProvider { .. } => body_children.push(banner_view::<Msg>(
                BannerKind::Success,
                "Broker UP, provider matcheado".to_string(),
            )),
        }

        // State card.
        let (state_accent, state_value, state_descr) = state_card_params(
            &model.state,
            accent_up,
            accent_partial,
            accent_down,
            accent_pending,
        );
        body_children.push(stat_card_view::<Msg>(
            "Estado",
            state_value,
            &state_descr,
            state_accent,
            &[],
            &stat_palette,
        ));

        // Sessions card.
        let sessions_items: Vec<String> = model
            .sessions
            .as_ref()
            .map(|list| {
                let mut entries: Vec<_> = list.entries.iter().collect();
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
        let sessions_count_value = model
            .sessions
            .as_ref()
            .map(|l| l.entries.len().to_string())
            .unwrap_or_else(|| "—".into());
        let sessions_descr = match &model.sessions {
            None => "lista no disponible (broker DOWN o pendiente)".to_string(),
            Some(l) if l.entries.is_empty() => {
                "sin sesiones registradas en el broker".into()
            }
            Some(_) => "labels visibles + flows in/out · (wit) = consciente".into(),
        };
        body_children.push(stat_card_view::<Msg>(
            "Sesiones activas",
            sessions_count_value,
            &sessions_descr,
            accent_up,
            &sessions_items,
            &stat_palette,
        ));

        // Timeline card.
        let timeline_items: Vec<String> = model
            .timeline
            .iter()
            .take(20)
            .map(format_timeline_entry)
            .collect();
        let timeline_value = model.timeline.len().to_string();
        let timeline_descr = if model.timeline.is_empty() {
            "esperando primer match…".to_string()
        } else {
            "↑ más reciente · ↓ más viejo · cap 50 entries".to_string()
        };
        body_children.push(stat_card_view::<Msg>(
            "Timeline de matches",
            timeline_value,
            &timeline_descr,
            accent_partial,
            &timeline_items,
            &stat_palette,
        ));

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

impl Model {
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

fn state_card_params(
    state: &ProbeState,
    accent_up: Color,
    accent_partial: Color,
    accent_down: Color,
    accent_pending: Color,
) -> (Color, String, String) {
    match state {
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
        ProbeState::UpWithProvider { flow, producer_socket } => (
            accent_up,
            "UP / PROVIDER".into(),
            format!(
                "flow `{flow}` matcheado en producer socket: {}",
                producer_socket.display()
            ),
        ),
    }
}

fn run_probe(flow: String, type_name: String) -> Msg {
    let started = Instant::now();
    let card = build_consumer_card("brahman-broker-explorer-llimphi", flow.clone(), type_name);
    let result = await_provider_blocking(card, PROBE_TIMEOUT);

    let new_state = match result {
        Ok(socket) => ProbeState::UpWithProvider {
            flow: flow.clone(),
            producer_socket: socket,
        },
        Err(ConsumerError::NoProvider { .. }) => ProbeState::UpNoProvider { flow: flow.clone() },
        Err(e) => ProbeState::Down {
            reason: e.to_string(),
        },
    };

    let (sessions, matches) = match &new_state {
        ProbeState::Down { .. } | ProbeState::Pending => (None, None),
        _ => (
            list_sessions_blocking("brahman-broker-explorer-llimphi").ok(),
            list_matches_blocking("brahman-broker-explorer-llimphi").ok(),
        ),
    };

    Msg::ProbeResult {
        state: new_state,
        sessions,
        matches,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

/// Diff puro entre snapshots de matches. Devuelve la lista de
/// entries nuevas (Available + Lost) en orden Available-primero, y
/// el set actualizado de keys.
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

fn format_timeline_entry(e: &TimelineEntry) -> String {
    use card_handshake::messages::MatchEventKind;
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
            h,
            m,
            s,
            kind,
            e.consumer_label,
            e.consumer_flow,
            e.producer_label,
            e.producer_flow,
            e.via,
            pinned,
        ),
        MatchEventKind::Lost => format!(
            "{:02}:{:02}:{:02} {} ?.{} ← ?.{} (lost)",
            h, m, s, kind, e.consumer_flow, e.producer_flow,
        ),
    }
}

fn main() {
    llimphi_ui::run::<Explorer>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_is_default_state_at_boot() {
        let s = ProbeState::Pending;
        assert!(matches!(s, ProbeState::Pending));
    }

    #[test]
    fn poll_and_probe_constants_are_sane() {
        assert!(PROBE_TIMEOUT < POLL_INTERVAL);
        assert!(POLL_INTERVAL >= Duration::from_secs(2));
    }

    fn synth_match(
        consumer_label: &str,
        consumer_flow: &str,
        producer_label: &str,
        producer_flow: &str,
    ) -> chasqui_broker::Match {
        use card_core::TypeRef;
        use chasqui_broker::{Endpoint, Match, MatchStrategy};
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
        let last: HashSet<_> = std::iter::once(prev_key).collect();
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
