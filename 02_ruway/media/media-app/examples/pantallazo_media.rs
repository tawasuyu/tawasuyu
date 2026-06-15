//! Pantallazo headless de `media-app` — el reproductor nativo del dominio.
//!
//! Reproduce la composición **real** del `view()` de la app (src/main.rs):
//! menubar arriba · canvas de video hero · franja de subtítulo activo ·
//! barras de control configurables estilo VLC (timeline + transporte +
//! volumen, las del `Toolbar::default()` de media-core) · fila de
//! visualizadores (onda de pista completa con playhead, waterfall
//! espectral, medidores peak/RMS) · pie con ticks/fps/posición.
//! `media-app` es bin-only (todo vive privado en src/main.rs), así que la
//! composición se calca acá tal cual — mismo criterio que el pantallazo
//! de khipu.
//!
//! Estado sembrado verosímil y determinista (nada depende de la hora):
//! el fotograma actual es un frame **real** del [`TestCard`] de
//! media-core (la fuente fallback de la propia app) avanzado ~3 s; la
//! pista va por la mitad (2:34 / 5:12, trk 2/3); el subtítulo activo sale
//! de un SRT parseado por el `SubtitleTrack` real; la onda de pista usa
//! `media_core::waveform::Waveform` sobre una canción sintética con
//! dinámica verso/estribillo; el waterfall son 60 análisis Goertzel del
//! `Waterfall` real sobre audio sintetizado (bajo + melodía + brillos).
//!
//! Pinta a una textura wgpu sin ventana y vuelca PNG (mismo patrón que
//! `agora-app/examples/pantallazo_agora.rs`).
//!
//! `cargo run -p media-app --example pantallazo_media --release -- [out.png]`

use std::fs::File;
use std::io::BufWriter;
use std::sync::Arc;
use std::time::Duration;

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_icons::{icon_view, Icon};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_hal::{wgpu, Hal};
use llimphi_ui::llimphi_layout::taffy::{
    self,
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect as TaffyRect,
};
use llimphi_ui::llimphi_layout::LayoutTree;
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{
    Blob, Color, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat,
};
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::{draw_block, TextBlock, Typesetter};
use llimphi_ui::{measure_text_node, mount, paint, DragPhase, ImageFit, View};
use llimphi_widget_menubar::{menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_timeline::{timeline_view, TimelinePalette};
use llimphi_widget_transport::{transport_button_view, TransportButton, TransportPalette};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_ui::llimphi_layout::taffy::style::Position;
use media_core::control::ControlSettings;
use media_core::toolbar::{Bar, BarItem, BarPosition, Toolbar};
use media_core::waveform::Waveform;
use media_core::{FrameSource, Levels, SubtitleTrack, TestCard, Waterfall};

const W: u32 = 1600;
const H: u32 = 1000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// El pantallazo no despacha mensajes: la view se monta con `Msg = ()`.
type Msg = ();

// ============================================================
// Estado sembrado — lo que `playback_snapshot()` + slots globales
// le darían a la view tras media canción de una sesión real.
// ============================================================

struct Estado {
    /// Posición actual del track (2:34 — pasada la mitad ya se ve
    /// recorrido en timeline, playhead y reloj).
    position: Duration,
    /// Duración total (5:12).
    duration: Duration,
    /// Reproduciendo (play iluminado, no pausado).
    playing: bool,
    /// Ganancia lineal del volumen (80%).
    volume: f32,
    /// Repeat-all activo → botón tintado.
    repeat_on: bool,
    shuffle_on: bool,
    recording: bool,
    eq_on: bool,
    /// Título compuesto como `media_title_string()`: tag — artista · ▸ capítulo.
    title: String,
    /// Pista actual / total de la cola.
    trk: (usize, usize),
}

impl Estado {
    fn fraccion(&self) -> f32 {
        (self.position.as_secs_f64() / self.duration.as_secs_f64()).clamp(0.0, 1.0) as f32
    }
}

fn estado_demo() -> Estado {
    Estado {
        position: Duration::from_secs(154),
        duration: Duration::from_secs(312),
        playing: true,
        volume: 0.8,
        repeat_on: true,
        shuffle_on: false,
        recording: false,
        eq_on: false,
        // Mismo formato que `media_title_string()`: metadata + capítulo (V7).
        title: "Vals del calcetín — Killa  ·  ▸ estribillo".to_string(),
        trk: (2, 3),
    }
}

// ============================================================
// Calcos fieles de los helpers de view de src/main.rs
// ============================================================

/// Formatea `M:SS` (calco de `fmt_secs`).
fn fmt_secs(d: Duration) -> String {
    let t = d.as_secs();
    format!("{}:{:02}", t / 60, t % 60)
}

/// Formatea `M:SS` / `H:MM:SS` (calco de `fmt_mmss`).
fn fmt_mmss(d: Duration) -> String {
    let t = d.as_secs();
    let (h, m, s) = (t / 3600, (t % 3600) / 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// El menú principal del reproductor (calco de `app_menu`):
/// Archivo / Reproducción / Ver / Idioma / Ayuda.
fn app_menu() -> AppMenu {
    let t = rimay_localize::t;
    let cur = rimay_localize::current_locale();
    let lang_item = |label: &str, code: &str| {
        let mut it = MenuItem::new(label, format!("lang.{code}"));
        if cur == code {
            it = it.icon("\u{2714}");
        }
        it
    };

    AppMenu::new()
        .menu(
            Menu::new(t("file"))
                .item(MenuItem::new(t("media-menu-capture-frame"), "file.snapshot"))
                .item(MenuItem::new(t("media-menu-record"), "file.record").separated())
                .item(MenuItem::new(t("media-menu-reload-controls"), "file.reload").shortcut("F5"))
                .item(MenuItem::new(t("exit"), "file.quit").shortcut("Ctrl+Q").separated()),
        )
        .menu(
            Menu::new(t("media-menu-playback"))
                .item(MenuItem::new(t("media-menu-play-pause"), "play.toggle").shortcut("Space"))
                .item(MenuItem::new(t("media-menu-seek-back"), "play.back").shortcut("←"))
                .item(MenuItem::new(t("media-menu-seek-fwd"), "play.fwd").shortcut("→").separated())
                .item(MenuItem::new(t("media-menu-prev-track"), "play.prev"))
                .item(MenuItem::new(t("media-menu-next-track"), "play.next").separated())
                .item(MenuItem::new(t("media-menu-volume-up"), "play.vol_up"))
                .item(MenuItem::new(t("media-menu-volume-down"), "play.vol_dn")),
        )
        .menu(
            Menu::new(t("view"))
                .item(MenuItem::new(t("settings"), "view.settings").shortcut("F2").separated())
                .item(MenuItem::new(t("media-menu-playlist"), "view.playlist"))
                .item(MenuItem::new(t("media-menu-visualizers"), "view.visualizers"))
                .item(MenuItem::new(t("command-palette"), "view.palette").shortcut("Ctrl+Shift+P"))
                .item(MenuItem::new(t("media-menu-shortcuts-help"), "view.help").shortcut("?")),
        )
        .menu(
            Menu::new(t("language"))
                .item(lang_item("Español", "es-PE"))
                .item(lang_item("English", "en-US"))
                .item(lang_item("Runasimi", "qu-PE")),
        )
        .menu(Menu::new(t("help")).item(MenuItem::new(t("about"), "help.about")))
}

/// Paleta del transport con los colores exactos de la app (calco de
/// `transport_palette`).
fn transport_palette() -> TransportPalette {
    TransportPalette {
        bg: Color::from_rgba8(44, 52, 66, 255),
        bg_active: Color::from_rgba8(46, 84, 110, 255),
        bg_hover: Color::from_rgba8(70, 92, 120, 255),
        fg: Color::from_rgba8(214, 224, 240, 255),
        fg_active: Color::from_rgba8(150, 215, 245, 255),
        fg_record: Color::from_rgba8(232, 86, 86, 255),
        btn_w: 40.0,
        btn_h: 34.0,
        radius: 8.0,
        icon_stroke: 2.0,
        gap: 6.0,
    }
}

/// Botón con ícono del set canónico (calco de `icon_button`).
fn icon_button(icon: Icon, active: bool) -> View<Msg> {
    let bg = if active {
        Color::from_rgba8(46, 84, 110, 255)
    } else {
        Color::from_rgba8(44, 52, 66, 255)
    };
    let col = if matches!(icon, Icon::Record) {
        Color::from_rgba8(232, 86, 86, 255)
    } else if active {
        Color::from_rgba8(150, 215, 245, 255)
    } else {
        Color::from_rgba8(214, 224, 240, 255)
    };
    View::new(Style {
        size: Size {
            width: length(40.0_f32),
            height: length(34.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(Color::from_rgba8(70, 92, 120, 255))
    .radius(8.0)
    .children(vec![icon_view::<Msg>(icon, col, 2.0)])
}

/// Texto fijo dentro de una barra (calco de `bar_label`).
fn bar_label(text: String, width: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(width),
            height: length(36.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(text, 13.0, color)
}

/// Barra de progreso (calco de `timeline_strip` — mismo widget).
fn timeline_strip(frac: f32) -> View<Msg> {
    let palette = TimelinePalette::from_theme(&Theme::dark());
    timeline_view(frac, &palette, |_fraction| None)
}

/// Mapea un [`BarItem`] a su vista (calco de `bar_item_view`, con el
/// estado sembrado en lugar de los slots globales).
fn bar_item_view(item: BarItem, e: &Estado) -> View<Msg> {
    // Pasos reales del keymap default de media-core (mismos que `settings()`).
    let s = ControlSettings::default();
    let (step, vstep) = (s.seek_step_secs, s.volume_step);
    let tpal = transport_palette();
    let tbtn = |b: TransportButton| transport_button_view::<Msg, _>(b, &tpal, |_a| ());
    match item {
        BarItem::PlayPause => tbtn(TransportButton::PlayPause { playing: e.playing }),
        BarItem::Stop => tbtn(TransportButton::Stop),
        BarItem::Prev => tbtn(TransportButton::Prev),
        BarItem::Next => tbtn(TransportButton::Next),
        BarItem::SeekBack => tbtn(TransportButton::SeekBack { secs: step }),
        BarItem::SeekForward => tbtn(TransportButton::SeekForward { secs: step }),
        BarItem::VolumeDown => tbtn(TransportButton::VolumeDown { step: vstep }),
        BarItem::VolumeUp => tbtn(TransportButton::VolumeUp { step: vstep }),
        BarItem::Mute => tbtn(TransportButton::Mute { muted: e.volume <= 1e-4 }),
        BarItem::Repeat => tbtn(TransportButton::Repeat { active: e.repeat_on }),
        BarItem::Shuffle => tbtn(TransportButton::Shuffle { active: e.shuffle_on }),
        BarItem::SpeedDown => tbtn(TransportButton::SpeedDown),
        BarItem::SpeedUp => tbtn(TransportButton::SpeedUp),
        BarItem::SpeedReset => tbtn(TransportButton::SpeedReset { is_default: true }),
        BarItem::Snapshot => tbtn(TransportButton::Snapshot),
        BarItem::Record => tbtn(TransportButton::Record { recording: e.recording }),
        BarItem::Equalizer => tbtn(TransportButton::Equalizer { enabled: e.eq_on }),
        BarItem::Settings => icon_button(Icon::Settings, false),
        BarItem::Timeline => View::new(Style {
            size: Size {
                width: auto(),
                height: length(34.0_f32),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![timeline_strip(e.fraccion())]),
        BarItem::Spacer => View::new(Style {
            size: Size {
                width: auto(),
                height: length(34.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        }),
        BarItem::Clock => bar_label(
            format!("{} / {}", fmt_mmss(e.position), fmt_mmss(e.duration)),
            120.0,
            Color::from_rgba8(180, 195, 215, 255),
        ),
        BarItem::VolumeLabel => bar_label(
            format!("vol {:.0}%", (e.volume * 100.0).round()),
            76.0,
            Color::from_rgba8(180, 195, 215, 255),
        ),
        BarItem::VolumeSlider => {
            let mut pal = SliderPalette::from_theme(&Theme::dark());
            pal.label_width = 0.0;
            pal.value_width = 0.0;
            pal.track_width = 120.0;
            pal.row_height = 34.0;
            pal.track_thickness = 8.0;
            View::new(Style {
                size: Size {
                    width: length(128.0_f32),
                    height: length(34.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .children(vec![slider_view::<Msg, _>(
                "",
                e.volume,
                0.0,
                2.0,
                &pal,
                |_phase: DragPhase, _delta| None,
            )])
        }
        BarItem::Title => bar_label(e.title.clone(), 300.0, Color::from_rgba8(200, 212, 230, 255)),
    }
}

/// Barras ancladas a `position` (calco de `toolbar_view_at`).
fn toolbar_view_at(toolbar: &Toolbar, e: &Estado, position: BarPosition) -> Option<View<Msg>> {
    let bars: Vec<View<Msg>> = toolbar
        .bars
        .iter()
        .filter(|bar| bar.position == position)
        .map(|bar| {
            let items: Vec<View<Msg>> = bar.items.iter().map(|&it| bar_item_view(it, e)).collect();
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(48.0_f32),
                },
                gap: Size {
                    width: length(10.0_f32),
                    height: length(0.0_f32),
                },
                padding: TaffyRect {
                    left: length(10.0_f32),
                    right: length(10.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(Color::from_rgba8(28, 33, 43, 255))
            .radius(10.0)
            .children(items)
        })
        .collect();
    if bars.is_empty() {
        return None;
    }
    let n = bars.len() as f32;
    Some(
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: length(n * 56.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(8.0_f32),
            },
            ..Default::default()
        })
        .children(bars),
    )
}

/// Franja del cue de subtítulo activo (calco de `subtitle_strip`).
fn subtitle_strip(text: String) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(44.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(8, 10, 14, 240))
    .radius(6.0)
    .text(text, 18.0, Color::from_rgba8(240, 240, 240, 255))
}

// ============================================================
// Visualizadores (calcos de fulltrack_waveform_view / waterfall_panel /
// meters_panel con los datos sembrados en lugar de slots/probe vivos)
// ============================================================

/// Onda de pista completa tipo Audacity con playhead (calco de
/// `fulltrack_waveform_view`, con los picos ya escaneados).
fn fulltrack_waveform_view(peaks: Vec<(f32, f32)>, frac: f32) -> View<Msg> {
    let stroke = Color::from_rgba8(120, 220, 170, 255);
    let center_color = Color::from_rgba8(64, 74, 90, 255);
    let playhead_color = Color::from_rgba8(242, 184, 92, 255);

    View::new(Style {
        size: Size {
            width: auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(14, 16, 22, 255))
    .radius(8.0)
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 {
            return;
        }
        let pad_x: f32 = 12.0;
        let pad_y: f32 = 8.0;
        let ix = rect.x + pad_x;
        let iy = rect.y + pad_y;
        let iw = (rect.w - 2.0 * pad_x).max(1.0);
        let ih = (rect.h - 2.0 * pad_y).max(1.0);
        let mid = iy + ih * 0.5;
        let amp = ih * 0.5;

        // Línea central.
        let mut center = BezPath::new();
        center.move_to((ix as f64, mid as f64));
        center.line_to(((ix + iw) as f64, mid as f64));
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, center_color, None, &center);

        // Una columna vertical (min→max) por bucket de picos.
        let n = peaks.len().max(1);
        let mut env = BezPath::new();
        for (i, &(vmin, vmax)) in peaks.iter().enumerate() {
            let x = ix + (i as f32 / n as f32) * iw;
            let y_top = mid - vmax.clamp(-1.0, 1.0) * amp;
            let y_bot = mid - vmin.clamp(-1.0, 1.0) * amp;
            env.move_to((x as f64, y_top as f64));
            env.line_to((x as f64, y_bot as f64));
        }
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, stroke, None, &env);

        // Playhead en la posición actual.
        let px = ix + frac.clamp(0.0, 1.0) * iw;
        let mut ph = BezPath::new();
        ph.move_to((px as f64, iy as f64));
        ph.line_to((px as f64, (iy + ih) as f64));
        scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, playhead_color, None, &ph);
    })
}

/// Gradiente "heat" del waterfall (calco de `heat_color`).
fn heat_color(v: f32) -> Color {
    let v = v.clamp(0.0, 1.0);
    if v < 0.25 {
        let t = v / 0.25;
        Color::from_rgba8(
            (60.0 + 110.0 * t) as u8,
            (20.0 + 30.0 * t) as u8,
            (20.0 + 10.0 * t) as u8,
            255,
        )
    } else if v < 0.6 {
        let t = (v - 0.25) / 0.35;
        Color::from_rgba8(
            (170.0 + 70.0 * t) as u8,
            (50.0 + 110.0 * t) as u8,
            (30.0 + 40.0 * t) as u8,
            255,
        )
    } else {
        let t = (v - 0.6) / 0.4;
        Color::from_rgba8(
            (240.0 + 15.0 * t) as u8,
            (160.0 + 80.0 * t) as u8,
            (70.0 + 160.0 * t) as u8,
            255.min((180.0 + 75.0 * t) as u8),
        )
    }
}

/// Spectrogram histórico (calco de `waterfall_panel`, con el grid ya
/// analizado por el `Waterfall` real).
fn waterfall_view(grid: Vec<f32>, rows: usize, bands: usize) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(14, 16, 22, 255))
    .radius(8.0)
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 || rows == 0 || bands == 0 {
            return;
        }
        let pad: f32 = 6.0;
        let inner_x = rect.x + pad;
        let inner_y = rect.y + pad;
        let inner_w = (rect.w - 2.0 * pad).max(1.0);
        let inner_h = (rect.h - 2.0 * pad).max(1.0);
        let cell_w = inner_w / bands as f32;
        let cell_h = inner_h / rows as f32;
        for r in 0..rows {
            let y0 = inner_y + r as f32 * cell_h;
            for b in 0..bands {
                let m = grid[r * bands + b];
                if m < 0.02 {
                    continue;
                }
                let x0 = inner_x + b as f32 * cell_w;
                let cell = KurboRect::new(
                    x0 as f64,
                    y0 as f64,
                    (x0 + cell_w + 0.5) as f64,
                    (y0 + cell_h + 0.5) as f64,
                );
                scene.fill(Fill::NonZero, Affine::IDENTITY, heat_color(m), None, &cell);
            }
        }
    })
}

/// Gradiente verde → ámbar → rojo (calco de `level_color`).
fn level_color(v: f32) -> Color {
    let v = v.clamp(0.0, 1.0);
    if v < 0.5 {
        Color::from_rgba8(110, 220, 140, 255)
    } else if v < 0.85 {
        Color::from_rgba8(230, 200, 90, 255)
    } else {
        Color::from_rgba8(240, 95, 95, 255)
    }
}

/// Medidores peak + RMS (calco de `meters_panel`, niveles ya medidos
/// por el `Levels` real sobre el bloque sintetizado).
fn meters_view(pk: f32, rms: f32) -> View<Msg> {
    let track_bg = Color::from_rgba8(34, 40, 52, 255);
    let label_color = Color::from_rgba8(150, 165, 185, 255);

    View::new(Style {
        size: Size {
            width: length(160.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 {
            return;
        }
        let label_w: f32 = 36.0;
        let bar_h: f32 = 8.0;
        let gap_y: f32 = 6.0;
        let inner_x = rect.x;
        let inner_y = rect.y + (rect.h - (bar_h * 2.0 + gap_y)) * 0.5;
        let bars_x = inner_x + label_w;
        let bars_w = (rect.w - label_w).max(1.0);

        let pk_label =
            TextBlock::simple("PK", 11.0, label_color, (inner_x as f64, (inner_y - 3.0) as f64));
        draw_block(scene, ts, &pk_label);
        let rms_y = inner_y + bar_h + gap_y;
        let rms_label =
            TextBlock::simple("RMS", 11.0, label_color, (inner_x as f64, (rms_y - 3.0) as f64));
        draw_block(scene, ts, &rms_label);

        // Tracks (fondo) + fills coloreados por nivel.
        for (y, v) in [(inner_y, pk), (rms_y, rms)] {
            let track = KurboRect::new(
                bars_x as f64,
                y as f64,
                (bars_x + bars_w) as f64,
                (y + bar_h) as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, track_bg, None, &track);
            let w = (v.clamp(0.0, 1.0) * bars_w).max(0.0);
            if w > 0.0 {
                let fill = KurboRect::new(
                    bars_x as f64,
                    y as f64,
                    (bars_x + w) as f64,
                    (y + bar_h) as f64,
                );
                scene.fill(Fill::NonZero, Affine::IDENTITY, level_color(v), None, &fill);
            }
        }
    })
}

// ============================================================
// Siembras: frame de video, subtítulo, onda, waterfall, niveles
// ============================================================

/// Fotograma actual: un frame **real** del [`TestCard`] de media-core (la
/// fuente fallback de la propia app) avanzado ~3 s — gradiente animado +
/// círculo en lissajous, en 16:9 como un video de verdad.
fn frame_actual() -> ImageBrush {
    let (fw, fh) = (640u32, 360u32);
    let mut card = TestCard::new(fw, fh, 30.0);
    let dt = card.frame_interval();
    let mut buf = Vec::new();
    for _ in 0..96 {
        let _ = card.tick(dt, &mut buf);
    }
    ImageBrush::new(ImageData {
        data: Blob::new(Arc::new(buf)),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::Alpha,
        width: fw,
        height: fh,
    })
}

/// Cue activo a la posición sembrada, parseado por el `SubtitleTrack` real.
fn subtitulo_activo(pos: Duration) -> String {
    let srt = "1\n00:02:28,000 --> 00:02:40,000\n\
               — Guardalo en el calcetín: BLAKE3 no pierde nudos.\n";
    SubtitleTrack::parse_subtitles(srt)
        .ok()
        .and_then(|t| t.at(pos).map(|c| c.text.clone()))
        .unwrap_or_default()
}

/// Canción sintética con dinámica verso/estribillo: envolvente por tramos
/// suavizada exponencialmente × seno lento. Sólo para que `Waveform` (el
/// mismo tipo que llena `foreign_av::decode_peaks`) tenga picos creíbles.
fn onda_de_pista(dur_secs: f32) -> Waveform {
    const SR: f32 = 200.0;
    let tramos: [(f32, f32); 8] = [
        (0.0, 0.16),   // intro
        (20.0, 0.50),  // verso 1
        (80.0, 0.88),  // estribillo
        (112.0, 0.48), // verso 2
        (170.0, 0.90), // estribillo
        (202.0, 0.28), // puente
        (232.0, 0.95), // estribillo final
        (282.0, 0.05), // fade out
    ];
    let n = (dur_secs * SR) as usize;
    let mut env = 0.0f32;
    let samples: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let target = tramos
                .iter()
                .rev()
                .find(|(t0, _)| t >= *t0)
                .map(|(_, v)| *v)
                .unwrap_or(0.2);
            env += (target - env) * 0.01;
            let wob = 0.72 + 0.28 * (t * 0.83).sin();
            env * wob * (t * 23.0).sin()
        })
        .collect();
    Waveform::from_samples(&samples, 1600)
}

/// Un bloque de "música" sintética para el análisis espectral: bajo que
/// pulsa, melodía que sube por la octava y brillos intermitentes.
fn bloque_audio(row: usize, sr: u32, len: usize) -> Vec<f32> {
    let bass_amp = if row % 15 < 5 { 0.85 } else { 0.30 };
    let mel_freq = 220.0 * (2.0_f32).powf((row % 16) as f32 / 16.0 * 3.0);
    let treb_amp = if row % 8 < 2 { 0.30 } else { 0.04 };
    let mut rng: u64 = 0x9E37_79B9 ^ (row as u64).wrapping_mul(0x5DEE_CE66D);
    (0..len)
        .map(|i| {
            let t = i as f32 / sr as f32;
            // xorshift64 para un piso de ruido determinista (sin dep rand).
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let noise = ((rng >> 40) as f32 / 8_388_608.0) - 1.0;
            bass_amp * (t * 70.0 * std::f32::consts::TAU).sin()
                + 0.45 * (t * mel_freq * std::f32::consts::TAU).sin()
                + treb_amp * (t * 8_800.0 * std::f32::consts::TAU).sin()
                + 0.02 * noise
        })
        .collect()
}

/// 60 análisis Goertzel del `Waterfall` real (28 bandas, 40 Hz → 16 kHz —
/// los mismos parámetros del panel de la app) sobre los bloques sintéticos.
fn sembrar_waterfall() -> (Vec<f32>, usize, usize) {
    const SR: u32 = 48_000;
    let mut wf = Waterfall::new(28, 60, 40.0, 16_000.0);
    for r in 0..60 {
        let bloque = bloque_audio(r, SR, 2048);
        wf.analyze(&bloque, 1, SR);
    }
    let mut grid = Vec::new();
    let (rows, bands) = wf.snapshot(&mut grid);
    (grid, rows, bands)
}

// ============================================================
// La view completa (calco del `view()` de MediaApp)
// ============================================================

fn view_demo(e: &Estado, theme: &Theme) -> View<Msg> {
    let menu = app_menu();
    let menubar = menubar_view(&MenuBarSpec {
        menu: &menu,
        open: None,
        theme,
        viewport: (W as f32, H as f32),
        height: MENU_H,
        on_open: Arc::new(|_| ()),
        on_command: Arc::new(|_c: &str| ()),
    });

    // Hero: canvas de video. En la app es `gpu_paint_with` blitteando el
    // `ExternalSurface` del pipeline; acá el mismo rect muestra el frame
    // del TestCard como imagen (Contain = letterbox de reproductor).
    let canvas = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(10, 12, 18, 255))
    .radius(10.0)
    .image(frame_actual())
    .image_fit(ImageFit::Contain);

    let subs_strip = subtitle_strip(subtitulo_activo(e.position));

    // Barras configurables: las default de media-core (timeline + transporte,
    // abajo) más una barra "arriba" con el título — composición que el
    // usuario arma desde la pestaña "Barras" de la configuración.
    let mut toolbar = Toolbar::default();
    toolbar.bars.insert(
        0,
        Bar::at(vec![BarItem::Title, BarItem::Spacer], BarPosition::Above),
    );
    let above_bars = toolbar_view_at(&toolbar, e, BarPosition::Above);
    let below_bars = toolbar_view_at(&toolbar, e, BarPosition::Below);

    // Visualizadores desplegados (menú Ver → Visualizadores de audio):
    // onda de pista completa + waterfall + medidores.
    let onda = fulltrack_waveform_view(onda_de_pista(e.duration.as_secs_f32()).peaks().to_vec(), e.fraccion());
    let (grid, rows, bands) = sembrar_waterfall();
    let (pk, rms) = {
        let mut lv = Levels::new();
        lv.analyze(&bloque_audio(2, 48_000, 2048), 1);
        (lv.peak(), lv.rms())
    };
    let visualizers = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(200.0_f32),
        },
        gap: Size {
            width: length(10.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![onda, waterfall_view(grid, rows, bands), meters_view(pk, rms)]);

    let mut kids: Vec<View<Msg>> = Vec::new();
    if let Some(v) = above_bars {
        kids.push(v);
    }
    kids.push(canvas);
    kids.push(subs_strip);
    if let Some(v) = below_bars {
        kids.push(v);
    }
    kids.push(visualizers);

    let content = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(12.0_f32),
        },
        padding: TaffyRect {
            left: length(18.0_f32),
            right: length(18.0_f32),
            top: length(10.0_f32),
            bottom: length(14.0_f32),
        },
        ..Default::default()
    })
    .children(kids);

    // Sidebars de dientes (calco de `dock`): rail al borde interno izquierdo
    // + panel del diente activo (acá la Cola desplegada). Mismo widget real
    // `dock_rail_view` que usa la app.
    let body = {
        let inner = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .children(vec![perfiles_panel_demo(theme), content]);
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .children(vec![inner, dock_rail_demo(theme)])
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(22, 26, 34, 255))
    .children(vec![menubar, body])
}

/// Rail de dientes (overlay absoluto al borde interno izquierdo). Perfiles activa.
fn dock_rail_demo(theme: &Theme) -> View<Msg> {
    let icons = [Icon::Music, Icon::Home, Icon::Settings, Icon::Equalizer, Icon::Info];
    let items: Vec<DockRailItem> = (0..5)
        .map(|id| DockRailItem { id, active: id == 1 })
        .collect();
    let rail = dock_rail_view(
        &items,
        40.0,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| icon_view::<Msg>(icons[id as usize], color, size / 12.0),
        |_| (),
        |_| None,
    );
    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            top: length(8.0_f32),
            left: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(40.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![rail])
}

/// Panel de Perfiles desplegado (calco de `dock::perfiles_panel`): crear/
/// seleccionar/bloquear perfiles + sus playlists guardadas. Sembrado con un
/// perfil activo con candado y dos playlists cargadas de carpetas.
fn perfiles_panel_demo(theme: &Theme) -> View<Msg> {
    fn pbtn(label: String, bg: Color, fg: Color) -> View<Msg> {
        View::new(Style {
            size: Size { width: auto(), height: length(28.0_f32) },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            padding: TaffyRect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(bg)
        .radius(6.0)
        .text(label, 12.5, fg)
    }
    fn psquare(label: &str) -> View<Msg> {
        View::new(Style {
            size: Size { width: length(30.0_f32), height: length(28.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(Color::from_rgba8(74, 58, 64, 255))
        .radius(6.0)
        .text(label.to_string(), 13.0, Color::from_rgba8(225, 232, 245, 255))
    }
    fn prow(children: Vec<View<Msg>>) -> View<Msg> {
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            flex_shrink: 0.0,
            gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .children(children)
    }
    fn psection(title: &str) -> View<Msg> {
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(title.to_string(), 13.0, Color::from_rgba8(118, 182, 232, 255))
    }
    let t = rimay_localize::t;
    let green = Color::from_rgba8(48, 70, 58, 255);
    let green_fg = Color::from_rgba8(220, 235, 226, 255);

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: TaffyRect {
            left: length(40.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text(t("media-dock-perfiles"), 14.5, Color::from_rgba8(118, 182, 232, 255));

    let kids = vec![
        header,
        // Estado.
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(format!("▶ 130 {}", t("media-prof-tracks")), 12.0, Color::from_rgba8(232, 200, 130, 255)),
        prow(vec![pbtn(t("media-prof-new"), green, green_fg)]),
        psection(&t("media-dock-perfiles")),
        prow(vec![
            pbtn(format!("sergio · {}", t("media-prof-locked")), Color::from_rgba8(48, 86, 120, 255), Color::from_rgba8(224, 233, 245, 255)),
            psquare("✕"),
        ]),
        prow(vec![
            pbtn("invitado".to_string(), Color::from_rgba8(34, 40, 52, 255), Color::from_rgba8(224, 233, 245, 255)),
            psquare("✕"),
        ]),
        prow(vec![pbtn(t("media-prof-clear-pass"), Color::from_rgba8(70, 58, 64, 255), Color::from_rgba8(235, 220, 226, 255))]),
        psection(&format!("sergio · {}", t("media-prof-playlists"))),
        prow(vec![
            pbtn("▶  Cumbias del barrio  (42)".to_string(), Color::from_rgba8(34, 44, 40, 255), green_fg),
            psquare("✕"),
        ]),
        prow(vec![
            pbtn("▶  Lo-fi para el kernel  (130)".to_string(), Color::from_rgba8(34, 44, 40, 255), green_fg),
            psquare("✕"),
        ]),
        prow(vec![pbtn(t("media-prof-add-dir"), green, green_fg)]),
    ];

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(380.0_f32), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        padding: TaffyRect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(kids)
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/shots/media.png".to_string());
    if let Some(dir) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(dir).ok();
    }

    // Mismos rótulos que la app: localización en español (determinista,
    // no depende del wawa-config del usuario).
    rimay_localize::init();
    let _ = rimay_localize::set_locale("es-PE");

    let theme = Theme::dark();
    let estado = estado_demo();
    let root = view_demo(&estado, &theme);

    // view → layout → scene (misma secuencia que el eventloop real).
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, root);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (W as f32, H as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    let mut scene = vello::Scene::new();
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pantallazo-media"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
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
    let bg = Color::from_rgba8(22, 26, 34, 255); // el fill del root de la app
    renderer
        .render_to_view(&hal, &scene, &view, W, H, bg)
        .expect("render_to_view");

    write_png(&hal, &target, &out);
    eprintln!("pantallazo_media: escrito {out} ({W}x{H})");
}

/// Lee la textura a CPU y la vuelca como PNG RGBA8.
fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * H as usize) as u64,
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
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
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
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
