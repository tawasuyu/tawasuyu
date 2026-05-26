//! `cosmos-app-llimphi` — visor minimalista del lienzo astrológico
//! sobre Llimphi.
//!
//! MVP: arma a mano un `RenderModel` (carta natal estática con cuerpos
//! clásicos repartidos en grados representativos), llama a
//! `cosmos-render::compose_wheel` para producir la lista de
//! `DrawCommand`, y la pinta con `cosmos-canvas-llimphi`.
//!
//! Sin engine real (cosmos-engine arrastra eternal-sky con efemérides
//! VSOP2013, ~30 MB de tablas). Cuando se quiera el visor con cálculo
//! real se reemplaza `mock_model()` por un `engine.compute(...)`.

use cosmos_canvas_llimphi::canvas_view;
use cosmos_render::{
    compose_wheel, ChartId, ChartKind, CompositionOpts, Geometry, Glyph, Layer, LayerKind,
    RenderModel,
};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use wawa_config_llimphi::theme_from_wawa;

const WHEEL_SIZE: f32 = 720.0;

#[derive(Clone)]
enum Msg {
    /// El bus `wawa-config` publicó una versión nueva.
    WawaConfigChanged(Box<wawa_config::WawaConfig>),
}

struct Model {
    render: RenderModel,
    theme: Theme,
    /// Suscripción al bus. Mantiene vivo el watcher.
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
        (980, 800)
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
        Model {
            render: mock_model(),
            theme,
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

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, canvas])
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

/// Carta natal sintética: cuerpos clásicos repartidos a tercios del
/// zodiaco para que el dial muestre algo equilibrado. No representa
/// ninguna fecha real — el cálculo viene del engine cuando exista.
fn mock_model() -> RenderModel {
    let bodies = vec![
        ("sun", 15.0_f32, false),
        ("moon", 78.0, false),
        ("mercury", 32.0, true),
        ("venus", 55.0, false),
        ("mars", 142.0, false),
        ("jupiter", 195.0, false),
        ("saturn", 240.0, true),
        ("uranus", 285.0, false),
        ("neptune", 320.0, false),
        ("pluto", 350.0, true),
    ];

    let sign_dial = Layer {
        module_id: "natal".into(),
        kind: LayerKind::SignDial,
        ring: 0.95,
        z: 0,
        geometry: Geometry::Ring {
            cusps_deg: (0..12).map(|i| i as f32 * 30.0).collect(),
        },
        glyphs: (0..12)
            .map(|i| Glyph {
                deg: i as f32 * 30.0 + 15.0,
                symbol: sign_name(i).into(),
                ..Default::default()
            })
            .collect(),
    };

    let bodies_layer = Layer {
        module_id: "natal".into(),
        kind: LayerKind::Bodies,
        ring: 0.62,
        z: 10,
        geometry: Geometry::GlyphsOnly,
        glyphs: bodies
            .iter()
            .map(|(name, deg, retro)| Glyph {
                deg: *deg,
                symbol: (*name).into(),
                retrograde: *retro,
                ..Default::default()
            })
            .collect(),
    };

    RenderModel {
        chart_id: ChartId::new(),
        chart_kind: ChartKind::Natal,
        title: rimay_localize::t("cosmos-demo-title"),
        subtitle: Some(rimay_localize::t("cosmos-demo-subtitle")),
        compute_ms: 0,
        ascendant_deg: 8.0,
        midheaven_deg: 280.0,
        descendant_deg: 188.0,
        imum_coeli_deg: 100.0,
        geo_latitude_deg: 0.0,
        geo_longitude_deg: 0.0,
        layers: vec![sign_dial, bodies_layer],
        overlays: vec![],
        aspect_summary: vec![],
        uranian_groups: vec![],
        gr_triggers: vec![],
        harmonic: 1,
        harmonic_spectrum: vec![],
    }
}

fn sign_name(i: usize) -> &'static str {
    [
        "aries", "taurus", "gemini", "cancer", "leo", "virgo", "libra", "scorpio",
        "sagittarius", "capricorn", "aquarius", "pisces",
    ][i]
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Cosmos>();
}
