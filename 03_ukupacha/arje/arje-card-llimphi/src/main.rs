//! `arje-card-llimphi` — card de escritorio con el estado vivo del init
//! arje: las **capacidades de aislamiento** que el sistema puede otorgar
//! a un proceso encarnado (namespaces, cgroups, privilegios).
//!
//! El README de arje promete una "card escritorio (estado de arje)";
//! `arje-card` nunca lo fue (quedó como alias de tipos de `card-core`).
//! Esta es esa card, sobre Llimphi.
//!
//! La fuente de verdad es [`arje_incarnate::caps::CapabilitySet::detect`],
//! la misma rutina que `Incarnator::new` corre antes de encarnar una Card.
//! No requiere daemon ni privilegios: lee `/proc` y reporta qué se puede
//! aislar AQUÍ Y AHORA (los sysctl/LSM cambian entre boots, por eso se
//! re-detecta por polling, no se cachea).
//!
//! Stack visual idéntico a `minga-explorer-llimphi`: llimphi-theme +
//! app-header + banner + stat-card, theme reactivo a `wawa-config`.
//!
//! Uso:
//! ```sh
//! cargo run -p arje-card-llimphi
//! ```

#![forbid(unsafe_code)]

use std::time::Duration;

use arje_incarnate::caps::{CapabilitySet, CgroupStatus, NsKind, UserNsStatus};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_stat_card::{stat_card_view, StatCardPalette};

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// Los 7 namespaces que `CapabilitySet::can_create_ns` sabe evaluar, en
/// el orden en que se muestran. (NsKind no expone un `all()`.)
const NAMESPACES: [NsKind; 7] = [
    NsKind::User,
    NsKind::Mount,
    NsKind::Pid,
    NsKind::Net,
    NsKind::Uts,
    NsKind::Ipc,
    NsKind::Cgroup,
];

/// Snapshot derivado de un `CapabilitySet`, ya en forma presentable
/// (strings + bools) para que la `view` no toque los enums del dominio.
#[derive(Clone)]
struct CapsSnapshot {
    kernel: (u32, u32, u32),
    root: bool,
    user_ns: &'static str,
    cgroup_v2: &'static str,
    cgroup_delegated: bool,
    max_user_ns: Option<u64>,
    our_cgroup: Option<String>,
    /// (nombre del namespace, ¿creable aquí?).
    ns_can: Vec<(&'static str, bool)>,
    /// Cuántos de los 7 namespaces son creables.
    ns_creatable: usize,
}

impl CapsSnapshot {
    fn detect() -> Self {
        let caps = CapabilitySet::detect();
        let ns_can: Vec<(&'static str, bool)> = NAMESPACES
            .iter()
            .map(|&k| (k.name(), caps.can_create_ns(k)))
            .collect();
        let ns_creatable = ns_can.iter().filter(|(_, ok)| *ok).count();
        Self {
            kernel: caps.kernel_version,
            root: caps.has_cap_sys_admin,
            user_ns: human_user_ns(&caps.user_ns),
            cgroup_v2: human_cgroup(&caps.cgroup_v2),
            cgroup_delegated: caps.cgroup_delegated,
            max_user_ns: caps.max_user_namespaces,
            our_cgroup: caps.our_cgroup.map(|p| p.display().to_string()),
            ns_can,
            ns_creatable,
        }
    }
}

fn human_user_ns(s: &UserNsStatus) -> &'static str {
    match s {
        UserNsStatus::Allowed => "permitidos",
        UserNsStatus::DisabledBySysctl => "deshabilitados (sysctl)",
        UserNsStatus::RestrictedByLsm => "restringidos (LSM)",
        UserNsStatus::Unknown => "desconocido",
    }
}

fn human_cgroup(s: &CgroupStatus) -> &'static str {
    match s {
        CgroupStatus::Unified => "v2 unificado",
        CgroupStatus::Hybrid => "híbrido (v1+v2)",
        CgroupStatus::Legacy => "v1 legacy",
        CgroupStatus::NotMounted => "sin montar",
    }
}

struct Model {
    theme: Theme,
    snapshot: CapsSnapshot,
    last_detect_ms: u64,
    /// Mantiene vivo el watcher de wawa-config (su thread muere al dropear).
    _wawa_watcher: Option<wawa_config::ConfigWatcher>,
}

#[derive(Clone)]
enum Msg {
    /// Tick del scheduler: re-detecta capacidades y refresca el modelo.
    Tick,
    /// El bus de wawa-config cambió: re-aplicar theme/accent.
    WawaChanged(wawa_config::WawaConfig),
}

struct Card;

impl App for Card {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "arje — estado del init"
    }

    fn initial_size() -> (u32, u32) {
        (760, 540)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        // Re-detección periódica: los sysctl/LSM/cgroup delegation cambian
        // entre boots y a veces en caliente, así que no se cachea.
        handle.spawn_periodic(REFRESH_INTERVAL, || Msg::Tick);

        let initial_cfg = wawa_config::WawaConfig::load();
        let theme = theme_from_wawa(&initial_cfg);

        let handle_clone = handle.clone();
        let watcher = wawa_config::ConfigWatcher::spawn(move |cfg| {
            handle_clone.dispatch(Msg::WawaChanged(cfg));
        })
        .ok();

        Model {
            theme,
            snapshot: CapsSnapshot::detect(),
            last_detect_ms: 0,
            _wawa_watcher: watcher,
        }
    }

    fn update(model: Model, msg: Msg, _handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                // detect() lee unos cuantos archivos de /proc — microsegundos,
                // no bloquea el hilo de UI, no necesita spawn.
                let started = std::time::Instant::now();
                m.snapshot = CapsSnapshot::detect();
                m.last_detect_ms = started.elapsed().as_micros() as u64 / 1000;
            }
            Msg::WawaChanged(cfg) => {
                m.theme = theme_from_wawa(&cfg);
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        let header_palette = AppHeaderPalette::from_theme(theme);
        let stat_palette = StatCardPalette::from_theme(theme);
        let snap = &model.snapshot;

        // Acentos semánticos: aislamiento azul, privilegios ámbar, cgroups verde.
        let accent_iso = Color::from_rgba8(0x88, 0xc0, 0xd0, 0xff);
        let accent_priv = Color::from_rgba8(0xeb, 0xcb, 0x8b, 0xff);
        let accent_cgroup = Color::from_rgba8(0xa3, 0xbe, 0x8c, 0xff);

        let (ka, kb, kc) = snap.kernel;
        let header_text = format!(
            "Linux {ka}.{kb}.{kc}  ·  detección {} ms",
            model.last_detect_ms
        );
        let header = app_header::<Msg>(header_text, vec![], &header_palette);

        let mut body_children: Vec<View<Msg>> = Vec::new();

        // Banner de advertencia si no se puede aislar nada: el init no podrá
        // encarnar Cards con los namespaces que pidan.
        if snap.ns_creatable == 0 {
            body_children.push(banner_view::<Msg>(
                BannerKind::Warning,
                "Ningún namespace es creable aquí: las Cards que requieran \
                 aislamiento no podrán encarnarse sin CAP_SYS_ADMIN o user-ns."
                    .to_string(),
            ));
        }

        // Card 1 — aislamiento: namespaces creables.
        let ns_items: Vec<String> = snap
            .ns_can
            .iter()
            .map(|(name, ok)| format!("{}  {name}", if *ok { "✓" } else { "✗" }))
            .collect();
        body_children.push(stat_card_view::<Msg>(
            "Aislamiento",
            format!("{}/7", snap.ns_creatable),
            "namespaces creables para un proceso encarnado",
            accent_iso,
            &ns_items,
            &stat_palette,
        ));

        // Card 2 — privilegios.
        let priv_items = vec![
            format!(
                "CAP_SYS_ADMIN  {}",
                if snap.root { "sí (root)" } else { "no" }
            ),
            format!("user namespaces  {}", snap.user_ns),
            format!(
                "max_user_namespaces  {}",
                snap.max_user_ns
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "—".into())
            ),
        ];
        body_children.push(stat_card_view::<Msg>(
            "Privilegios",
            if snap.root { "root" } else { "usuario" },
            "de qué dispone el supervisor para aislar",
            accent_priv,
            &priv_items,
            &stat_palette,
        ));

        // Card 3 — cgroups.
        let cgroup_items = vec![
            format!(
                "delegación  {}",
                if snap.cgroup_delegated { "sí" } else { "no" }
            ),
            format!(
                "nuestro cgroup  {}",
                snap.our_cgroup.as_deref().unwrap_or("—")
            ),
        ];
        body_children.push(stat_card_view::<Msg>(
            "cgroups",
            snap.cgroup_v2,
            "control de recursos disponible",
            accent_cgroup,
            &cgroup_items,
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
            align_items: Some(AlignItems::Stretch),
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, body])
    }
}

/// Construye un `Theme` desde la config wawa (mismo helper que
/// minga-explorer-llimphi): variant canónico → `Theme::by_name`, accent
/// si está definido, fallback dark sin romper.
fn theme_from_wawa(cfg: &wawa_config::WawaConfig) -> Theme {
    let mut t = wawa_config::canonical_theme_name(&cfg.theme_variant)
        .and_then(Theme::by_name)
        .unwrap_or_else(Theme::dark);
    if let Some([r, g, b]) = wawa_config::accent_rgb(&cfg.accent) {
        let c = Color::from_rgba8(r, g, b, 0xff);
        t.accent = c;
        t.border_focus = c;
    }
    t
}

fn main() {
    llimphi_ui::run::<Card>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_snapshot_no_panic() {
        let snap = CapsSnapshot::detect();
        // Siempre evaluamos los 7 namespaces.
        assert_eq!(snap.ns_can.len(), 7);
        assert!(snap.ns_creatable <= 7);
    }

    #[test]
    fn human_labels_cubren_variantes() {
        assert_eq!(human_user_ns(&UserNsStatus::Allowed), "permitidos");
        assert_eq!(human_cgroup(&CgroupStatus::Unified), "v2 unificado");
    }
}
