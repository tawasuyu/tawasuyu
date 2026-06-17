//! **Showreel** de `chasqui` — la capa de red / broker P2P soberano sobre
//! Cards (brahman). NO es eye-candy abstracto: cada frame reconstruye el
//! **chrome real** del probe del broker (`chasqui-broker-explorer-llimphi`):
//! menubar + app-header con la línea de probe + banner de salud + tres
//! `stat_card` (Estado · Sesiones activas · Timeline de matches) + las filas
//! del timeline formateadas con `format_timeline_entry` REAL — todo con los
//! MISMOS widgets que pinta la app en producción (`menubar_view`,
//! `app_header`, `banner_view`, `stat_card_view`).
//!
//! El **estado** se deriva del tiempo normalizado `t∈[0,1]`: el chrome hace
//! slide-in; la salud del broker recorre DOWN → UP/NO PROVIDER → UP/PROVIDER;
//! las sesiones (peers) entran con stagger en la card de sesiones; y el
//! timeline de matches se llena fila a fila — consumidor ← productor
//! apareciendo en vivo, que es la imagen de "peers que se enganchan por flow".
//! Cierra con el wordmark **chasqui**.
//!
//! Render headless y determinista (sin reloj, sin runtime, sin winit): frame
//! `i` de `N` → `t = i/(N-1)` → View → layout (taffy + parley) → vello::Scene
//! → wgpu → PNG. Idéntico al eventloop.
//!
//! ```text
//! cargo run -p chasqui-broker-explorer-llimphi --example chasqui_showreel \
//!     --release -- [out_dir] [n_frames] [W] [H]
//! ```
//! Defaults: `out_dir=showreel_frames_chasqui`, `n_frames=300`, `W=1600`, `H=900`.
#![allow(dead_code)]

use std::fs::{create_dir_all, File};
use std::io::BufWriter;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use app_bus::{AppMenu, Menu, MenuItem};
use card_handshake::health::TimelineEntry;
use card_handshake::messages::MatchEventKind;
use chasqui_broker::MatchStrategy;
use llimphi_motion::motion;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::{
    self,
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    style::Position,
    AlignItems, Rect,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::peniko::{self, Color};
use llimphi_ui::llimphi_raster::vello::kurbo::{Affine, BezPath, Circle, Point, Stroke};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{draw_layout_brush_xf, measurement, Alignment, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, Mounted, PaintRect, View};

use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_stat_card::{stat_card_view, StatCardPalette};

const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Msg fantasma: el showreel no despacha eventos, pero los widgets reales
/// exigen un Msg `Clone + 'static`.
#[derive(Clone)]
enum Msg {
    Nada,
}

// ───────────────────────── utilidades ─────────────────────────

fn with_alpha(c: Color, a: f32) -> Color {
    let [r, g, b, _] = c.components;
    Color::new([r, g, b, a.clamp(0.0, 1.0)])
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Reescala `t` desde el subintervalo `[lo,hi]` a `[0,1]`, clampado.
fn seg(t: f32, lo: f32, hi: f32) -> f32 {
    ((t - lo) / (hi - lo)).clamp(0.0, 1.0)
}

// ───────────────────────── skin ─────────────────────────

#[derive(Clone)]
struct Skin {
    theme: Theme,
    accent: Color,
    bg: Color,
    fg: Color,
    fg_muted: Color,
    // Acentos por estado del probe (espejo de la app real).
    accent_up: Color,
    accent_partial: Color,
    accent_down: Color,
    accent_pending: Color,
}

// ───────────────────────── datos sintéticos (forma real) ─────────────────────────

/// Una "sesión" (peer) registrada en el broker, con sus flows in/out — la
/// misma forma que `SessionEntry` produce en la card de sesiones real.
struct Peer {
    label: &'static str,
    inputs: &'static [&'static str],
    outputs: &'static [&'static str],
    conscious: bool,
}

const PEERS: &[Peer] = &[
    Peer {
        label: "chasqui-nous",
        inputs: &["doc-ingest:json"],
        outputs: &["monad-list:json", "event-log:tail"],
        conscious: true,
    },
    Peer {
        label: "pluma-app",
        inputs: &["monad-list:json"],
        outputs: &["doc-ingest:json"],
        conscious: false,
    },
    Peer {
        label: "nahual-shell",
        inputs: &["monad-list:json", "thumb:png"],
        outputs: &[],
        conscious: false,
    },
    Peer {
        label: "shuma-term",
        inputs: &["event-log:tail"],
        outputs: &["cmd-run:json"],
        conscious: false,
    },
    Peer {
        label: "khipu",
        inputs: &[],
        outputs: &["thumb:png"],
        conscious: true,
    },
];

/// Formatea una entrada de sesión igual que la app real (label · in/out · wit).
fn fmt_peer(p: &Peer) -> String {
    format!(
        "{}  ·  in:[{}]  out:[{}]{}",
        p.label,
        p.inputs.join(","),
        p.outputs.join(","),
        if p.conscious { "  (wit)" } else { "" }
    )
}

/// Matches que el broker va "descubriendo" — consumidor ← productor por flow.
/// `(consumer_label, flow, producer_label, flow, via, pinned)`.
struct MatchSeed {
    consumer_label: &'static str,
    flow: &'static str,
    producer_label: &'static str,
    via: MatchStrategy,
    pinned: bool,
}

const MATCHES: &[MatchSeed] = &[
    MatchSeed {
        consumer_label: "pluma-app",
        flow: "monad-list:json",
        producer_label: "chasqui-nous",
        via: MatchStrategy::Exact,
        pinned: true,
    },
    MatchSeed {
        consumer_label: "nahual-shell",
        flow: "monad-list:json",
        producer_label: "chasqui-nous",
        via: MatchStrategy::Exact,
        pinned: false,
    },
    MatchSeed {
        consumer_label: "chasqui-nous",
        flow: "doc-ingest:json",
        producer_label: "pluma-app",
        via: MatchStrategy::Exact,
        pinned: false,
    },
    MatchSeed {
        consumer_label: "nahual-shell",
        flow: "thumb:png",
        producer_label: "khipu",
        via: MatchStrategy::Structural,
        pinned: false,
    },
    MatchSeed {
        consumer_label: "shuma-term",
        flow: "event-log:tail",
        producer_label: "chasqui-nous",
        via: MatchStrategy::Exact,
        pinned: false,
    },
];

/// Convierte un seed en `TimelineEntry` real (kind=Available) con un `at`
/// determinista escalonado, para reusar el formateo idéntico al de la app.
fn timeline_entry(seed: &MatchSeed, idx: usize) -> TimelineEntry {
    // Base fija (no usa el reloj real → determinista): 14:32:10 + idx*7s.
    let base = UNIX_EPOCH + Duration::from_secs(52_330 + idx as u64 * 7);
    TimelineEntry {
        at: base,
        kind: MatchEventKind::Available,
        consumer_label: seed.consumer_label.to_string(),
        consumer_flow: seed.flow.to_string(),
        producer_label: seed.producer_label.to_string(),
        producer_flow: seed.flow.to_string(),
        via: seed.via,
        pinned: seed.pinned,
    }
}

/// Espejo EXACTO de `format_timeline_entry` del binario `src/main.rs` —
/// no podemos importarlo (es un binario sin lib), así que lo replicamos
/// carácter a carácter para que las filas se vean idénticas a la app.
fn format_timeline_entry(e: &TimelineEntry) -> String {
    let secs_today = e
        .at
        .duration_since(SystemTime::UNIX_EPOCH)
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

// ───────────────────────── chrome real ─────────────────────────

/// Menú real del probe (espejo de `app_menu` del binario).
fn app_menu() -> AppMenu {
    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Refrescar probe", "file.refresh").shortcut("Ctrl+R"))
                .item(MenuItem::new("Limpiar timeline", "file.clear"))
                .item(MenuItem::new("Salir", "file.quit").shortcut("Ctrl+Q").separated()),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Reconectar", "view.reconnect"))
                .item(MenuItem::new("Cambiar tema", "view.theme").separated()),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

fn menubar(cw: f64, ch: f64, s: &Skin) -> View<Msg> {
    let menu = app_menu();
    menubar_view(&MenuBarSpec {
        menu: &menu,
        open: None,
        theme: &s.theme,
        viewport: (cw as f32, ch as f32),
        height: MENU_H,
        on_open: Arc::new(|_| Msg::Nada),
        on_command: Arc::new(|_: &str| Msg::Nada),
    })
}

/// Fase de salud del broker derivada de `t` — recorre los tres estados
/// reales que el probe reporta.
#[derive(Clone, Copy, PartialEq)]
enum Health {
    Pending,
    Down,
    UpNoProvider,
    UpProvider,
}

fn health_at(t: f32) -> Health {
    if t < 0.20 {
        Health::Pending
    } else if t < 0.34 {
        Health::Down
    } else if t < 0.46 {
        Health::UpNoProvider
    } else {
        Health::UpProvider
    }
}

fn health_banner(h: Health) -> Option<View<Msg>> {
    match h {
        Health::Pending => None,
        Health::Down => Some(banner_view::<Msg>(
            BannerKind::Error,
            "Broker DOWN — connect failed (init socket no escucha)".to_string(),
        )),
        Health::UpNoProvider => Some(banner_view::<Msg>(
            BannerKind::Warning,
            "Broker UP, sin provider para el flow".to_string(),
        )),
        Health::UpProvider => Some(banner_view::<Msg>(
            BannerKind::Success,
            "Broker UP, provider matcheado".to_string(),
        )),
    }
}

fn state_card_params(h: Health, s: &Skin) -> (Color, String, String) {
    match h {
        Health::Pending => (
            s.accent_pending,
            "PENDING".into(),
            "esperando primer probe…".into(),
        ),
        Health::Down => (
            s.accent_down,
            "DOWN".into(),
            "connect failed: init socket no responde".into(),
        ),
        Health::UpNoProvider => (
            s.accent_partial,
            "UP / NO PROVIDER".into(),
            "broker reachable; sin productor para flow `monad-list:json`".into(),
        ),
        Health::UpProvider => (
            s.accent_up,
            "UP / PROVIDER".into(),
            "flow `monad-list:json` matcheado en chasqui-nous.sock".into(),
        ),
    }
}

/// Fila del timeline (espejo de la fila real en `view()`): texto monoespaciado
/// con la entry formateada, la última resaltada como "recién llegada".
fn timeline_row(text: String, selected: bool, s: &Skin) -> View<Msg> {
    let mut row = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(text, 12.0, s.theme.fg_text, Alignment::Start);
    if selected {
        row = row.fill(s.theme.bg_selected);
    }
    row
}

// ───────────────────────── overlays vector (cold-open + wordmark) ─────────────────────────

fn signature_path(cw: f64, ch: f64) -> BezPath {
    let cx = cw / 2.0;
    let cy = ch / 2.0;
    let mut p = BezPath::new();
    p.move_to((cx - 360.0, cy + 40.0));
    p.curve_to(
        (cx - 150.0, cy - 220.0),
        (cx + 150.0, cy + 220.0),
        (cx + 360.0, cy - 40.0),
    );
    p
}

fn trim_path(full: &BezPath, prog: f64) -> (BezPath, Point) {
    use vello::kurbo::ParamCurve;
    let prog = prog.clamp(0.0, 1.0);
    let mut cubic = None;
    let mut start = Point::ZERO;
    for el in full.elements() {
        match el {
            vello::kurbo::PathEl::MoveTo(p) => start = *p,
            vello::kurbo::PathEl::CurveTo(c1, c2, p) => {
                cubic = Some(vello::kurbo::CubicBez::new(start, *c1, *c2, *p));
            }
            _ => {}
        }
    }
    let mut out = BezPath::new();
    let mut head = start;
    if let Some(cb) = cubic {
        out.move_to(cb.p0);
        let steps = 96;
        for i in 1..=steps {
            let u = (i as f64 / steps as f64) * prog;
            let pt = cb.eval(u);
            out.line_to(pt);
            head = pt;
        }
    }
    (out, head)
}

fn draw_overlays(scene: &mut vello::Scene, ts: &mut Typesetter, t: f32, cw: f64, ch: f64, s: &Skin) {
    // ── COLD OPEN (0–11%) ──────────────────────────────────────────
    let b1 = seg(t, 0.0, 0.10);
    let line_vis = 1.0 - seg(t, 0.10, 0.17);
    if line_vis > 0.001 {
        let path = signature_path(cw, ch);
        let draw_on = motion::ease_out_cubic(seg(t, 0.01, 0.11)) as f64;
        let (trimmed, head) = trim_path(&path, draw_on);
        let line_col = with_alpha(s.accent, 0.9 * line_vis);
        scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, line_col, None, &trimmed);
        let pop = motion::ease_out_back(b1);
        let r = (4.0 + 7.0 * pop as f64).max(0.0);
        let dot_a = (b1 * line_vis).clamp(0.0, 1.0);
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.18 * dot_a),
            None,
            &Circle::new(head, r * 3.2),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, dot_a),
            None,
            &Circle::new(head, r),
        );
    }

    // ── WORDMARK (84–100%) ─────────────────────────────────────────
    let word_in = seg(t, 0.86, 0.96);
    let word_a = motion::ease_out_cubic(word_in);
    if word_a > 0.001 {
        let size = 140.0_f32;
        let layout = ts.layout(
            "chasqui", size, None, Alignment::Start, 1.0, false, None, 800.0, false, false, 0.0, 0.0,
        );
        let m = measurement(&layout);
        let rise = lerp(24.0, 0.0, word_a as f64);
        let ox = (cw - m.width as f64) / 2.0;
        let oy = (ch - m.height as f64) / 2.0 - 18.0 + rise;
        let brush = peniko::Brush::Solid(with_alpha(s.fg, word_a));
        draw_layout_brush_xf(scene, &layout, &brush, Affine::translate((ox, oy)));

        let sub_a = motion::ease_out_cubic(seg(t, 0.90, 1.0));
        if sub_a > 0.001 {
            let ssz = 26.0_f32;
            let sub = ts.layout(
                "sovereign P2P networking, in Rust",
                ssz,
                None,
                Alignment::Start,
                1.0,
                false,
                None,
                400.0,
                false,
                false, 0.0, 0.0,
            );
            let sm = measurement(&sub);
            let dot_r = 6.0;
            let block_w = sm.width as f64 + dot_r * 2.0 + 14.0;
            let sx = (cw - block_w) / 2.0;
            let sy = oy + m.height as f64 + 18.0;
            scene.fill(
                peniko::Fill::NonZero,
                Affine::IDENTITY,
                with_alpha(s.accent, sub_a),
                None,
                &Circle::new(Point::new(sx + dot_r, sy + ssz as f64 * 0.42), dot_r as f64),
            );
            let sbrush = peniko::Brush::Solid(with_alpha(s.fg_muted, sub_a));
            draw_layout_brush_xf(
                scene,
                &sub,
                &sbrush,
                Affine::translate((sx + dot_r * 2.0 + 14.0, sy)),
            );
        }
    }

    // ── punto teal de firma (esquina inf-der) ───────
    let corner_a = seg(t, 0.04, 0.12) * (1.0 - seg(t, 0.82, 0.88));
    if corner_a > 0.001 {
        let cx = cw - 54.0;
        let cy = ch - 54.0;
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.16 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 18.0),
        );
        scene.fill(
            peniko::Fill::NonZero,
            Affine::IDENTITY,
            with_alpha(s.accent, 0.9 * corner_a),
            None,
            &Circle::new(Point::new(cx, cy), 6.0),
        );
    }
}

// ───────────────────────── la escena por frame ─────────────────────────

fn build_view(t: f32, cw: f64, ch: f64, s: &Skin) -> View<Msg> {
    // ── timeline de animación ──
    // chrome slide-in (11–22%): menubar + header.
    let chrome = motion::ease_out_cubic(seg(t, 0.11, 0.22));
    // salud del broker (derivada por umbrales).
    let health = health_at(t);
    // sesiones (peers) entran con stagger una vez que el broker está UP (34–58%).
    let peers_reveal = motion::ease_out_cubic(seg(t, 0.34, 0.58));
    // matches del timeline aparecen fila a fila (46–80%) — sólo con provider.
    let matches_reveal = motion::ease_out_cubic(seg(t, 0.46, 0.80));
    // fade del chrome antes del wordmark (82–88%).
    let chrome_fade = 1.0 - seg(t, 0.82, 0.88);

    let mut children: Vec<View<Msg>> = Vec::new();

    if chrome_fade > 0.001 {
        let menubar = menubar(cw, ch, s);

        // header con la línea de probe real.
        let header_palette = AppHeaderPalette::from_theme(&s.theme);
        let reload_ms = 4 + ((t * 90.0) as u32 % 9);
        let header_text = format!(
            "Probe: {}/init.sock  ·  flow: broker-health/ping  ·  reload {} ms",
            "$XDG_RUNTIME_DIR", reload_ms
        );
        let header = app_header::<Msg>(header_text, vec![], &header_palette);

        // ── cuerpo ──
        let stat_palette = StatCardPalette::from_theme(&s.theme);
        let mut body_children: Vec<View<Msg>> = Vec::new();

        // banner permanente con la salud actual.
        if let Some(b) = health_banner(health) {
            body_children.push(b);
        }

        // card de Estado.
        let (state_accent, state_value, state_descr) = state_card_params(health, s);
        body_children.push(stat_card_view::<Msg>(
            "Estado",
            state_value,
            &state_descr,
            state_accent,
            &[],
            &stat_palette,
        ));

        // card de Sesiones activas (peers) — los items entran con stagger.
        let n_peers_up = matches!(
            health,
            Health::UpNoProvider | Health::UpProvider
        )
        .then(|| ((PEERS.len() as f32) * peers_reveal).ceil() as usize)
        .unwrap_or(0)
        .min(PEERS.len());
        // Listamos hasta 3 peers (los "recientes") para dejar aire al
        // timeline de matches debajo — el contador refleja el total real.
        let shown = n_peers_up.min(3);
        let peer_items: Vec<String> = PEERS[..shown].iter().map(fmt_peer).collect();
        let sessions_value = if n_peers_up == 0 {
            "—".to_string()
        } else {
            n_peers_up.to_string()
        };
        let sessions_descr = if n_peers_up == 0 {
            "lista no disponible (broker DOWN o pendiente)".to_string()
        } else {
            "labels visibles + flows in/out · (wit) = consciente".to_string()
        };
        body_children.push(stat_card_view::<Msg>(
            "Sesiones activas",
            sessions_value,
            &sessions_descr,
            s.accent_up,
            &peer_items,
            &stat_palette,
        ));

        // card de Timeline de matches (cabecera contador) — sólo con provider.
        let n_matches = if health == Health::UpProvider {
            (((MATCHES.len()) as f32) * matches_reveal).ceil() as usize
        } else {
            0
        }
        .min(MATCHES.len());
        let timeline_value = n_matches.to_string();
        let timeline_descr = if n_matches == 0 {
            "esperando primer match…".to_string()
        } else {
            "click selecciona · right-click = menú · cap 50 entries".to_string()
        };
        body_children.push(stat_card_view::<Msg>(
            "Timeline de matches",
            timeline_value,
            &timeline_descr,
            s.accent_partial,
            &[],
            &stat_palette,
        ));

        // filas del timeline: consumidor ← productor, la última resaltada.
        for i in 0..n_matches {
            let entry = timeline_entry(&MATCHES[i], i);
            let txt = format_timeline_entry(&entry);
            let is_last = i + 1 == n_matches && matches_reveal < 1.0;
            body_children.push(timeline_row(txt, is_last, s));
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
        .fill(s.theme.bg_app)
        .children(body_children);

        // slide-in vertical del chrome completo.
        let dy = lerp(-22.0, 0.0, chrome as f64);
        let chrome_view = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(s.theme.bg_app)
        .alpha((chrome * chrome_fade).clamp(0.0, 1.0))
        .transform(Affine::translate((0.0, dy)))
        .children(vec![menubar, header, body]);
        children.push(chrome_view);
    }

    // overlay full-screen del vector (cold-open + wordmark).
    let overlay = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0),
            top: length(0.0),
            right: length(0.0),
            bottom: length(0.0),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with({
        let s = s.clone();
        move |scene, ts, _rect: PaintRect| {
            draw_overlays(scene, ts, t, cw, ch, &s);
        }
    });
    children.push(overlay);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        position: Position::Relative,
        ..Default::default()
    })
    .fill(s.bg)
    .children(children)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = args.next().unwrap_or_else(|| "showreel_frames_chasqui".to_string());
    let n: usize = args.next().and_then(|v| v.parse().ok()).unwrap_or(300);
    let w: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1600);
    let h: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(900);
    create_dir_all(&out_dir).expect("mkdir out_dir");

    let theme = Theme::dark();
    let accent = Color::from_rgba8(0x2B, 0xD9, 0xA6, 0xFF); // teal firma
    let skin = Skin {
        accent,
        bg: theme.bg_app,
        fg: theme.fg_text,
        fg_muted: theme.fg_muted,
        // mismos acentos por estado que la app real.
        accent_up: Color::from_rgba8(0xa3, 0xbe, 0x8c, 0xff),
        accent_partial: Color::from_rgba8(0xeb, 0xcb, 0x8b, 0xff),
        accent_down: Color::from_rgba8(0xbf, 0x61, 0x6a, 0xff),
        accent_pending: Color::from_rgba8(0x6a, 0x72, 0x80, 0xff),
        theme,
    };

    let [br, bg, bb, _] = skin.bg.components;
    let base = Color::from_rgba8((br * 255.0) as u8, (bg * 255.0) as u8, (bb * 255.0) as u8, 255);

    // GPU una sola vez; reusar device/renderer/target para los N frames.
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("showreel-chasqui"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut ts = Typesetter::new();
    let cw = w as f64;
    let ch = h as f64;

    for i in 0..n {
        let t = if n <= 1 { 0.0 } else { i as f32 / (n as f32 - 1.0) };
        let root = build_view(t, cw, ch, &skin);

        let mut layout = LayoutTree::new();
        let mounted: Mounted<Msg> = mount(&mut layout, root);
        let computed = {
            let tmap = &mounted.text_measures;
            layout
                .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                    match tmap.get(&nid) {
                        Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                        None => taffy::Size::ZERO,
                    }
                })
                .expect("layout")
        };
        let mut scene = vello::Scene::new();
        paint(&mut scene, &mounted, &computed, &mut ts, None, None);

        renderer
            .render_to_view(&hal, &scene, &view, w, h, base)
            .expect("render_to_view");
        let path = format!("{out_dir}/frame_{i:04}.png");
        write_png(&hal, &target, &path, w, h);
        if i % 30 == 0 || i == n - 1 {
            eprintln!("showreel-chasqui: frame {}/{} (t={:.3})", i + 1, n, t);
        }
    }
    eprintln!("showreel-chasqui: {n} frames en {out_dir}/ ({w}x{h})");
}

fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str, w: u32, h: u32) {
    let unpadded = (w * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * h as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for r in 0..h as usize {
        let sidx = r * padded;
        pixels.extend_from_slice(&data[sidx..sidx + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&pixels).unwrap();
}
