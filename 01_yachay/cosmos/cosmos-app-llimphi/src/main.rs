//! `cosmos-app-llimphi` — visor del lienzo astrológico sobre Llimphi.
//!
//! Llama a [`cosmos_engine::compose`] con un `Chart` sample (sin store
//! todavía) y pinta el `RenderModel` resultante con `cosmos-canvas-llimphi`.
//! Eternal-bridge prendido por default → cuerpos calculados con VSOP2013,
//! casas Placidus, aspectos mayores. Toolbar arriba para alternar overlays
//! (Transit/Progression/SolarArc/Uranian/Lots/FixedStars/Midpoints).
//!
//! Pendiente: store de cartas (tree sidebar) + form de birth data.

use cosmos_canvas_llimphi::canvas_view;
use cosmos_engine::{compose, NatalOptions, PipelineRequest};
use cosmos_model::{
    Chart, ChartId, ChartKind, ContactId, StoredBirthData, StoredChartConfig, TimeCertainty,
};
use cosmos_render::{compose_wheel, CompositionOpts, RenderModel};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_button::{button_styled, ButtonPalette};
use wawa_config_llimphi::theme_from_wawa;

const WHEEL_SIZE: f32 = 720.0;

#[derive(Clone)]
enum Msg {
    WawaConfigChanged(Box<wawa_config::WawaConfig>),
    ToggleOverlay(OverlayKind),
    SetHarmonic(u32),
}

const HARMONICS: &[u32] = &[1, 4, 5, 7, 9];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum OverlayKind {
    Transit,
    Progression,
    SolarArc,
    Uranian,
    Lots,
    FixedStars,
    Midpoints,
}

impl OverlayKind {
    fn all() -> &'static [OverlayKind] {
        &[
            OverlayKind::Transit,
            OverlayKind::Progression,
            OverlayKind::SolarArc,
            OverlayKind::Uranian,
            OverlayKind::Lots,
            OverlayKind::FixedStars,
            OverlayKind::Midpoints,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            OverlayKind::Transit => "cosmos-overlay-transit",
            OverlayKind::Progression => "cosmos-overlay-progression",
            OverlayKind::SolarArc => "cosmos-overlay-solar-arc",
            OverlayKind::Uranian => "cosmos-overlay-uranian",
            OverlayKind::Lots => "cosmos-overlay-lots",
            OverlayKind::FixedStars => "cosmos-overlay-fixed-stars",
            OverlayKind::Midpoints => "cosmos-overlay-midpoints",
        }
    }

    fn to_request(self, target_age: f64) -> PipelineRequest {
        match self {
            OverlayKind::Transit => PipelineRequest::Transit,
            OverlayKind::Progression => PipelineRequest::SecondaryProgression {
                target_age_years: target_age,
            },
            OverlayKind::SolarArc => PipelineRequest::SolarArc {
                target_age_years: target_age,
            },
            OverlayKind::Uranian => PipelineRequest::Uranian,
            OverlayKind::Lots => PipelineRequest::Lots,
            OverlayKind::FixedStars => PipelineRequest::FixedStars,
            OverlayKind::Midpoints => PipelineRequest::Midpoints,
        }
    }
}

struct Model {
    chart: Chart,
    overlays: Vec<OverlayKind>,
    harmonic: u32,
    render: RenderModel,
    theme: Theme,
    error: Option<String>,
    _wawa_watcher: Option<wawa_config::ConfigWatcher>,
}

struct Cosmos;

impl App for Cosmos {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "cosmos · canvas (llimphi)"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 860)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let cfg = wawa_config::WawaConfig::load();
        let theme = theme_from_wawa(&cfg, &Theme::dark());
        let _ = rimay_localize::set_locale(&cfg.lang);
        let handle_clone = handle.clone();
        let watcher = wawa_config::ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::WawaConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("cosmos · wawa-config watcher: {e}"))
        .ok();

        let chart = sample_chart();
        let overlays: Vec<OverlayKind> = Vec::new();
        let harmonic = 1;
        let (render, error) = compute(&chart, &overlays, harmonic);
        Model {
            chart,
            overlays,
            harmonic,
            render,
            theme,
            error,
            _wawa_watcher: watcher,
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::WawaConfigChanged(cfg) => {
                m.theme = theme_from_wawa(&cfg, &m.theme);
                if cfg.lang != rimay_localize::current_locale() {
                    let _ = rimay_localize::set_locale(&cfg.lang);
                }
            }
            Msg::ToggleOverlay(kind) => {
                if let Some(idx) = m.overlays.iter().position(|k| *k == kind) {
                    m.overlays.remove(idx);
                } else {
                    m.overlays.push(kind);
                }
                let (render, error) = compute(&m.chart, &m.overlays, m.harmonic);
                m.render = render;
                m.error = error;
            }
            Msg::SetHarmonic(n) => {
                if m.harmonic != n {
                    m.harmonic = n;
                    let (render, error) = compute(&m.chart, &m.overlays, m.harmonic);
                    m.render = render;
                    m.error = error;
                }
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let opts = CompositionOpts {
            size: WHEEL_SIZE,
            rot_offset_deg: 0.0,
            include_bodies: true,
            palette: cosmos_render::Palette::dark(),
            draw_ascensional_cross: true,
            show_coord_labels: false,
            show_minor_aspects: false,
            dial_3d: true,
        };
        let commands = compose_wheel(&model.render, &opts);

        let canvas_bg = llimphi_ui::llimphi_raster::peniko::Color::from_rgba8(8, 10, 16, 255);
        let canvas = canvas_view::<Msg>(commands, WHEEL_SIZE, Some(canvas_bg));

        let header = header_bar(&model.render, &theme);
        let toolbar = overlay_toolbar(model, &theme);
        let harmonic_bar = harmonic_toolbar(model, &theme);
        let status = status_bar(model, &theme);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, toolbar, harmonic_bar, canvas, status])
    }
}

fn header_bar(m: &RenderModel, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(
        rimay_localize::t_args(
            "cosmos-header",
            &[
                ("title", m.title.as_str().into()),
                ("asc", format!("{:.1}", m.ascendant_deg).into()),
                ("mc", format!("{:.1}", m.midheaven_deg).into()),
            ],
        ),
        12.0,
        theme.fg_text,
        Alignment::Start,
    )
}

fn overlay_toolbar(model: &Model, theme: &Theme) -> View<Msg> {
    let pal_off = ButtonPalette::from_theme(theme);
    let pal_on = ButtonPalette {
        bg: theme.accent,
        bg_hover: theme.accent,
        fg: theme.bg_panel,
        radius: pal_off.radius,
    };

    let btns: Vec<View<Msg>> = OverlayKind::all()
        .iter()
        .map(|kind| {
            let active = model.overlays.contains(kind);
            let palette = if active { &pal_on } else { &pal_off };
            button_styled(
                rimay_localize::t(kind.label()),
                Style {
                    size: Size {
                        width: length(110.0_f32),
                        height: length(26.0_f32),
                    },
                    margin: Rect {
                        left: length(0.0_f32),
                        right: length(6.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    padding: Rect {
                        left: length(8.0_f32),
                        right: length(8.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Center),
                    ..Default::default()
                },
                Alignment::Center,
                palette,
                Msg::ToggleOverlay(*kind),
            )
        })
        .collect();

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(38.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(btns)
}

fn harmonic_toolbar(model: &Model, theme: &Theme) -> View<Msg> {
    let pal_off = ButtonPalette::from_theme(theme);
    let pal_on = ButtonPalette {
        bg: theme.accent,
        bg_hover: theme.accent,
        fg: theme.bg_panel,
        radius: pal_off.radius,
    };

    let mut row: Vec<View<Msg>> = Vec::with_capacity(HARMONICS.len() + 1);
    row.push(
        View::new(Style {
            size: Size {
                width: length(72.0_f32),
                height: length(22.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(
            rimay_localize::t("cosmos-harmonic-label"),
            12.0,
            theme.fg_muted,
            Alignment::Start,
        ),
    );
    for h in HARMONICS {
        let active = model.harmonic == *h;
        let palette = if active { &pal_on } else { &pal_off };
        row.push(button_styled(
            format!("H{h}"),
            Style {
                size: Size {
                    width: length(48.0_f32),
                    height: length(22.0_f32),
                },
                margin: Rect {
                    left: length(0.0_f32),
                    right: length(6.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                padding: Rect {
                    left: length(6.0_f32),
                    right: length(6.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            },
            Alignment::Center,
            palette,
            Msg::SetHarmonic(*h),
        ));
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(32.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(row)
}

fn status_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let txt = if let Some(err) = &model.error {
        rimay_localize::t_args("cosmos-status-error", &[("err", err.as_str().into())])
    } else {
        rimay_localize::t_args(
            "cosmos-status",
            &[
                ("ms", model.render.compute_ms.to_string().into()),
                ("layers", model.render.layers.len().to_string().into()),
                ("overlays", model.render.overlays.len().to_string().into()),
                ("aspects", model.render.aspect_summary.len().to_string().into()),
            ],
        )
    };
    let color = if model.error.is_some() {
        theme.fg_text
    } else {
        theme.fg_muted
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(txt, 11.0, color, Alignment::Start)
}

/// Carta sample mientras no haya store: nacimiento documentado, Lima.
/// La hora es razonable pero no es la mía — sirve para que la VSOP2013
/// escupa cuerpos reales y casas Placidus que tienen sentido visual.
fn sample_chart() -> Chart {
    Chart {
        id: ChartId::new(),
        contact_id: ContactId::new(),
        kind: ChartKind::Natal,
        label: rimay_localize::t("cosmos-demo-title"),
        birth_data: StoredBirthData {
            year: 1990,
            month: 6,
            day: 21,
            hour: 12,
            minute: 0,
            second: 0.0,
            tz_offset_minutes: -300,
            latitude_deg: -12.0464,
            longitude_deg: -77.0428,
            altitude_m: 154.0,
            time_certainty: TimeCertainty::Estimated,
            subject_name: None,
            birthplace_label: Some("Lima".into()),
        },
        config: StoredChartConfig::default(),
        related_chart_id: None,
        created_at_ms: 0,
    }
}

/// Llama al engine con los overlays activos. En error, vuelve a un
/// `compute_mock` para que la UI nunca quede en blanco; el mensaje se
/// surface vía status bar.
fn compute(
    chart: &Chart,
    overlays: &[OverlayKind],
    harmonic: u32,
) -> (RenderModel, Option<String>) {
    let target_age = 35.0;
    let requests: Vec<PipelineRequest> =
        overlays.iter().map(|k| k.to_request(target_age)).collect();
    let opts = NatalOptions {
        show_majors: true,
        show_minors: false,
        orb_multiplier: 1.0,
        show_dignities: true,
        harmonic,
    };
    match cosmos_engine::compose_with_options(chart, 0, &requests, &opts) {
        Ok(r) => (r, None),
        Err(e) => {
            let msg = format!("{e}");
            (compose(chart, 0, &[]).unwrap_or_else(|_| cosmos_engine::compute_mock(chart)), Some(msg))
        }
    }
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Cosmos>();
}
