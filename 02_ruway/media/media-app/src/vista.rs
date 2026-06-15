use std::sync::Arc;
use std::time::Duration;
use std::sync::atomic::Ordering;

use llimphi_icons::{icon_view, Icon};
use llimphi_motion::Tween;
use llimphi_module_command_palette::{self as palette, PaletteMsg, PaletteState};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{self, TextBlock};
use llimphi_ui::{DragPhase, View};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_timeline::{timeline_view_marked, TimelinePalette};
use llimphi_widget_transport::{
    transport_button_view, TransportAction, TransportButton, TransportPalette,
};
use llimphi_widget_waveform;
use app_bus::{AppMenu, Menu, MenuItem};
use media_core::control::MediaCommand;
use media_core::osd;
use media_core::toolbar::{BarItem, BarPosition};
use media_core::viewport::compute_layout;
use media_core::sync::FramePlan;
use media_core::{SubAlign, Levels, Waterfall};
use parking_lot::Mutex;

use crate::estado::{
    audio_probe_slot, chapters_slot, config_slot, eq, osd as get_osd, osd_now, pause,
    pipeline_slot, playlist_slot, recorder, settings, subtitles_slot, viewcontrol, volume,
    waveform_slot, SEEK_FORCE, SUB_DELAY_MS,
};
use crate::media_io::{bookmark_fractions, cover_image, fmt_mmss, is_network_url, media_title_string};
use crate::modelo::Model;
use crate::pipeline::pipeline_for;
use crate::playlist::{current_audio_position, playback_snapshot};
use crate::tipos::{Msg, VideoKind};
use crate::vista_config::settings_content as cfg_settings_content;

/// Overlay del command palette.
pub(crate) fn palette_overlay(model: &Model, state: &PaletteState) -> View<Msg> {
    let theme = llimphi_theme::Theme::dark();
    let pal = llimphi_module_command_palette::PalettePalette::from_theme(&theme);
    let inner = palette::view(state, &model.palette_commands, &pal, Msg::Palette);

    let (vw, vh) = model.viewport;
    let box_w = 560.0_f32.min(vw - 32.0);
    let x = ((vw - box_w) * 0.5).max(0.0);
    let y = (vh * 0.16).max(0.0);

    let panel = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(box_w),
            height: length(286.0_f32),
        },
        ..Default::default()
    })
    .on_click(Msg::Palette(PaletteMsg::Open))
    .children(vec![inner]);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 150))
    .on_click(Msg::Palette(PaletteMsg::Close))
    .children(vec![panel])
}

/// Hero de carátula (U5).
pub(crate) fn cover_hero() -> Option<View<Msg>> {
    let audio_only = matches!(config_slot().get().map(|c| c.kind), Some(VideoKind::Testcard));
    if !audio_only {
        return None;
    }
    let img = cover_image()?;
    Some(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            flex_grow: 1.0,
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            padding: TaffyRect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(12.0_f32),
                bottom: length(12.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(10, 12, 18, 255))
        .radius(10.0)
        .image(img),
    )
}

/// Overlay del OSD (U4).
pub(crate) fn osd_overlay(model: &Model) -> Option<View<Msg>> {
    let now = osd_now();
    let g = get_osd().lock();
    let text = g.active(now)?.to_string();
    let a = g.alpha_default(now).clamp(0.0, 1.0);
    drop(g);
    let fade = |base: u8| (base as f32 * a).round() as u8;
    let (vw, vh) = model.viewport;
    let box_w = 380.0_f32.min(vw - 40.0).max(120.0);
    let x = ((vw - box_w) * 0.5).max(0.0);
    let y = (vh * 0.09).max(MENU_H + 8.0);
    Some(
        View::new(Style {
            position: Position::Absolute,
            inset: TaffyRect {
                left: length(x),
                top: length(y),
                right: auto(),
                bottom: auto(),
            },
            size: Size {
                width: length(box_w),
                height: length(34.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(Color::from_rgba8(0, 0, 0, fade(175)))
        .radius(8.0)
        .text(text, 16.0, Color::from_rgba8(240, 240, 245, fade(255))),
    )
}

/// Arma el `MenuBarSpec` compartido.
pub(crate) fn menubar_spec<'a>(
    menu: &'a AppMenu,
    model: &Model,
    theme: &'a llimphi_theme::Theme,
) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: model.viewport,
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// El menú principal del reproductor.
pub(crate) fn app_menu() -> AppMenu {
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

/// Traduce un command id del menú al `Msg`/efecto real.
pub(crate) fn handle_menu_command(mut model: Model, cmd: &str, handle: &llimphi_ui::Handle<Msg>) -> Model {
    use MediaCommand::*;
    if let Some(code) = cmd.strip_prefix("lang.") {
        let _ = rimay_localize::set_locale(code);
        let mut cfg = wawa_config::WawaConfig::load();
        cfg.lang = code.to_string();
        let _ = cfg.save();
        return model;
    }
    let step = settings().seek_step_secs;
    let vstep = settings().volume_step;
    let dispatch = |c: MediaCommand| handle.dispatch(Msg::Command(c));
    match cmd {
        "file.snapshot" => dispatch(Snapshot),
        "file.record" => dispatch(ToggleRecord),
        "file.reload" => handle.dispatch(Msg::ReloadConfig),
        "file.quit" => {
            crate::config_io::save_history();
            crate::config_io::save_bookmarks();
            std::process::exit(0)
        }
        "play.toggle" => dispatch(TogglePause),
        "play.back" => dispatch(SeekBy { secs: -step }),
        "play.fwd" => dispatch(SeekBy { secs: step }),
        "play.prev" => dispatch(PrevTrack),
        "play.next" => dispatch(NextTrack),
        "play.vol_up" => dispatch(VolumeBy { delta: vstep }),
        "play.vol_dn" => dispatch(VolumeBy { delta: -vstep }),
        "view.settings" => handle.dispatch(Msg::ToggleSettings),
        "view.playlist" => handle.dispatch(Msg::TogglePlaylist),
        "view.visualizers" => model.visualizers_open = !model.visualizers_open,
        "view.palette" => handle.dispatch(Msg::Palette(PaletteMsg::Open)),
        "view.help" => handle.dispatch(Msg::ToggleHelp),
        _ => {}
    }
    model
}

/// Menú contextual del reproductor.
pub(crate) fn context_menu(model: &Model, x: f32, y: f32) -> View<Msg> {
    let t = rimay_localize::t;
    let paused = pause().is_paused();
    let recording = recorder().is_recording();
    let items = vec![
        ContextMenuItem::action(if paused { t("play") } else { t("pause") }),
        ContextMenuItem::action(t("media-menu-capture-frame")),
        ContextMenuItem::action(if recording { t("media-ctx-stop-record") } else { t("media-ctx-record-audio") }),
        ContextMenuItem::action(t("command-palette")),
        ContextMenuItem::action(t("media-menu-shortcuts-help")),
    ];
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(|i: usize| match i {
        0 => Msg::Command(MediaCommand::TogglePause),
        1 => Msg::Command(MediaCommand::Snapshot),
        2 => Msg::Command(MediaCommand::ToggleRecord),
        3 => Msg::Palette(PaletteMsg::Open),
        _ => Msg::ToggleHelp,
    });
    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: model.viewport,
        header: Some("media".to_string()),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&llimphi_theme::Theme::dark()),
    })
}

/// Franja debajo del canvas que muestra el cue de subtítulo activo.
pub(crate) fn subtitle_strip() -> View<Msg> {
    let guard = subtitles_slot().lock();
    let Some(track) = guard.as_ref() else {
        return View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        });
    };
    let position = playback_snapshot().position;
    let Some(cue) = track.at_with_delay(position, SUB_DELAY_MS.load(Ordering::Relaxed)) else {
        return View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(44.0_f32) },
            ..Default::default()
        });
    };
    let text = cue.text.clone();
    let align = track.align_for(cue);
    let color = match track.style_for(cue).map(|s| s.primary) {
        Some(c) if c.a > 0 => Color::from_rgba8(c.r, c.g, c.b, c.a),
        _ => Color::from_rgba8(240, 240, 240, 255),
    };
    let justify = match align {
        SubAlign::BottomLeft | SubAlign::MiddleLeft | SubAlign::TopLeft => JustifyContent::FlexStart,
        SubAlign::BottomRight | SubAlign::MiddleRight | SubAlign::TopRight => JustifyContent::FlexEnd,
        _ => JustifyContent::Center,
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(44.0_f32),
        },
        justify_content: Some(justify),
        align_items: Some(AlignItems::Center),
        padding: TaffyRect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(8, 10, 14, 240))
    .radius(6.0)
    .text(text, 18.0, color)
}

/// Barra de progreso clickeable bajo el video. Si el escaneo de onda
/// (background, tipo Audacity) ya terminó, pinta el **perfilado de la onda**
/// detrás del avance — la parte ya reproducida en acento y el resto atenuada,
/// con playhead y marcas. Si todavía no hay picos, cae a la barra de progreso
/// lisa (`llimphi-widget-timeline`).
pub(crate) fn timeline_strip() -> View<Msg> {
    let frac = {
        let s = playback_snapshot();
        let dur = s.duration.unwrap_or(Duration::ZERO).as_secs_f64();
        if dur <= 0.0 {
            0.0
        } else {
            (s.position.as_secs_f64() / dur).clamp(0.0, 1.0) as f32
        }
    };
    let marks = bookmark_fractions();
    let peaks: Option<Vec<(f32, f32)>> = waveform_slot()
        .lock()
        .as_ref()
        .filter(|w| !w.is_empty())
        .map(|w| w.peaks().to_vec());
    let inner = match peaks {
        Some(peaks) => waveform_timeline(peaks, frac, marks),
        None => {
            let palette = TimelinePalette::from_theme(&llimphi_theme::Theme::dark());
            timeline_view_marked(frac, &marks, &palette, |fraction| {
                Some(Msg::Command(MediaCommand::SeekTo { fraction }))
            })
        }
    };
    timeline_with_hover(inner)
}

/// Envuelve la barra de tiempo para reportar el **hover** (fracción bajo el
/// cursor → `Msg::TimelineHover`) y, si ya se extrajo, pintar el **preview de
/// scrub**: la miniatura del instante apuntado, flotando sobre la barra a la
/// altura del cursor (posición absoluta a `left = frac·ancho`). El scrub por
/// click del `inner` sigue funcionando (predicados de hit-test distintos).
fn timeline_with_hover(inner: View<Msg>) -> View<Msg> {
    let hover = *crate::estado::hover_frac_slot().lock();
    let preview: Option<View<Msg>> = hover.and_then(|hf| {
        let path = crate::estado::current_media_path()?;
        let img = crate::thumbs::hover_frame(&path.to_string_lossy(), hf)?;
        Some(
            View::new(Style {
                position: Position::Absolute,
                inset: TaffyRect {
                    left: percent(hf),
                    top: length(-74.0_f32),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size {
                    width: length(120.0_f32),
                    height: length(68.0_f32),
                },
                ..Default::default()
            })
            .fill(Color::from_rgba8(8, 10, 14, 255))
            .radius(6.0)
            .image(img),
        )
    });
    let mut kids = vec![inner];
    if let Some(p) = preview {
        kids.push(p);
    }
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .on_pointer_move_at(|lx, _ly, w, _h| {
        if w <= 0.0 {
            return None;
        }
        Some(Msg::TimelineHover(Some((lx / w).clamp(0.0, 1.0))))
    })
    .on_pointer_leave(Msg::TimelineHover(None))
    .children(kids)
}

/// Línea de tiempo con perfilado de onda (Audacity-like) y scrub.
fn waveform_timeline(peaks: Vec<(f32, f32)>, frac: f32, marks: Vec<f32>) -> View<Msg> {
    let played = Color::from_rgba8(120, 220, 170, 255);
    let unplayed = Color::from_rgba8(70, 86, 104, 255);
    let center_color = Color::from_rgba8(58, 68, 84, 255);
    let playhead_color = Color::from_rgba8(242, 184, 92, 255);
    let mark_color = Color::from_rgba8(255, 196, 84, 255);
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(16, 19, 26, 255))
    .radius(7.0)
    .on_click_at(|lx, _ly, w, _h| {
        let f = (lx / w.max(1.0)).clamp(0.0, 1.0);
        Some(Msg::Command(MediaCommand::SeekTo { fraction: f }))
    })
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 {
            return;
        }
        let pad_x: f32 = 6.0;
        let pad_y: f32 = 4.0;
        let ix = rect.x + pad_x;
        let iy = rect.y + pad_y;
        let iw = (rect.w - 2.0 * pad_x).max(1.0);
        let ih = (rect.h - 2.0 * pad_y).max(1.0);
        let mid = iy + ih * 0.5;
        let amp = ih * 0.5;

        let mut center = BezPath::new();
        center.move_to((ix as f64, mid as f64));
        center.line_to(((ix + iw) as f64, mid as f64));
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, center_color, None, &center);

        let n = peaks.len().max(1);
        let play_x = ix + frac.clamp(0.0, 1.0) * iw;
        // Dos pasadas (atenuada / acento) para no recolorear stroke por columna.
        let mut env_un = BezPath::new();
        let mut env_pl = BezPath::new();
        for (i, &(vmin, vmax)) in peaks.iter().enumerate() {
            let x = ix + (i as f32 / n as f32) * iw;
            let y_top = mid - vmax.clamp(-1.0, 1.0) * amp;
            let y_bot = mid - vmin.clamp(-1.0, 1.0) * amp;
            let env = if x <= play_x { &mut env_pl } else { &mut env_un };
            env.move_to((x as f64, y_top as f64));
            env.line_to((x as f64, y_bot as f64));
        }
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, unplayed, None, &env_un);
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, played, None, &env_pl);

        for f in &marks {
            if !(0.0..=1.0).contains(f) {
                continue;
            }
            let mx = ix + f * iw;
            let mut m = BezPath::new();
            m.move_to((mx as f64, iy as f64));
            m.line_to((mx as f64, (iy + ih) as f64));
            scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, mark_color, None, &m);
        }

        let mut ph = BezPath::new();
        ph.move_to((play_x as f64, iy as f64));
        ph.line_to((play_x as f64, (iy + ih) as f64));
        scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, playhead_color, None, &ph);
    })
}

/// Pinta las barras de controles configurables.
///
/// Respeta los flags de cada barra: `enabled=false` no se pinta; `autohide`
/// se esconde mientras el medio reproduce y reaparece al pausar o cuando el
/// usuario revela las barras (Tab → `model.reveal_bars`).
pub(crate) fn toolbar_view_at(model: &Model, position: BarPosition) -> Option<View<Msg>> {
    let reveal = model.reveal_bars || pause().is_paused();
    let bars: Vec<View<Msg>> = model
        .config
        .toolbar
        .bars
        .iter()
        .filter(|bar| bar.position == position)
        .filter(|bar| bar.enabled && (reveal || !bar.autohide))
        .map(|bar| {
            let items: Vec<View<Msg>> = bar.items.iter().map(|&it| bar_item_view(it)).collect();
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

/// Texto fijo dentro de una barra.
pub(crate) fn bar_label(text: String, width: f32, color: Color) -> View<Msg> {
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

/// Paleta de `llimphi-widget-transport`.
pub(crate) fn transport_palette() -> TransportPalette {
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

pub(crate) fn icon_button(icon: Icon, active: bool, msg: Msg) -> View<Msg> {
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
    .on_click(msg)
    .children(vec![icon_view::<Msg>(icon, col, 2.0)])
}

/// Mapea un [`BarItem`] a su vista concreta.
pub(crate) fn bar_item_view(item: BarItem) -> View<Msg> {
    use MediaCommand::*;
    let step = settings().seek_step_secs;
    let vstep = settings().volume_step;
    let snap = playback_snapshot();
    let to_cmd = |a: TransportAction| -> Msg {
        match a {
            TransportAction::TogglePlay => Msg::Command(TogglePause),
            TransportAction::Stop => Msg::Command(SeekTo { fraction: 0.0 }),
            TransportAction::Prev => Msg::Command(PrevTrack),
            TransportAction::Next => Msg::Command(NextTrack),
            TransportAction::SeekBy(secs) => Msg::Command(SeekBy { secs }),
            TransportAction::VolumeBy(delta) => Msg::Command(VolumeBy { delta }),
            TransportAction::ToggleMute => Msg::Command(ToggleMute),
            TransportAction::CycleRepeat => Msg::Command(CycleRepeat),
            TransportAction::ToggleShuffle => Msg::Command(ToggleShuffle),
            TransportAction::SpeedStep(dir) => Msg::Command(SpeedStep { dir }),
            TransportAction::SpeedReset => Msg::Command(SetSpeed { mult: 1.0 }),
            TransportAction::Snapshot => Msg::Command(Snapshot),
            TransportAction::ToggleRecord => Msg::Command(ToggleRecord),
            TransportAction::ToggleEqualizer => Msg::Command(EqToggle),
        }
    };
    let tpal = transport_palette();
    let tbtn = |b: TransportButton| transport_button_view(b, &tpal, to_cmd);
    match item {
        BarItem::PlayPause => tbtn(TransportButton::PlayPause { playing: !pause().is_paused() }),
        BarItem::Stop => tbtn(TransportButton::Stop),
        BarItem::Prev => tbtn(TransportButton::Prev),
        BarItem::Next => tbtn(TransportButton::Next),
        BarItem::SeekBack => tbtn(TransportButton::SeekBack { secs: step }),
        BarItem::SeekForward => tbtn(TransportButton::SeekForward { secs: step }),
        BarItem::VolumeDown => tbtn(TransportButton::VolumeDown { step: vstep }),
        BarItem::VolumeUp => tbtn(TransportButton::VolumeUp { step: vstep }),
        BarItem::Mute => tbtn(TransportButton::Mute { muted: volume().get() <= 1e-4 }),
        BarItem::Repeat => tbtn(TransportButton::Repeat { active: snap.repeat_label != "rep-" }),
        BarItem::Shuffle => tbtn(TransportButton::Shuffle { active: snap.shuffle_on }),
        BarItem::SpeedDown => tbtn(TransportButton::SpeedDown),
        BarItem::SpeedUp => tbtn(TransportButton::SpeedUp),
        BarItem::SpeedReset => tbtn(TransportButton::SpeedReset {
            is_default: (snap.speed - 1.0).abs() < 1e-3,
        }),
        BarItem::Snapshot => tbtn(TransportButton::Snapshot),
        BarItem::Record => tbtn(TransportButton::Record { recording: recorder().is_recording() }),
        BarItem::Equalizer => tbtn(TransportButton::Equalizer { enabled: eq().is_enabled() }),
        BarItem::Settings => icon_button(Icon::Settings, false, Msg::ToggleSettings),
        BarItem::Timeline => View::new(Style {
            size: Size {
                width: auto(),
                height: length(34.0_f32),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![timeline_strip()]),
        BarItem::Spacer => View::new(Style {
            size: Size {
                width: auto(),
                height: length(34.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        }),
        BarItem::Clock => {
            let s = playback_snapshot();
            let txt = match s.duration {
                Some(d) => format!("{} / {}", fmt_mmss(s.position), fmt_mmss(d)),
                None => fmt_mmss(s.position),
            };
            bar_label(txt, 120.0, Color::from_rgba8(180, 195, 215, 255))
        }
        BarItem::VolumeLabel => bar_label(
            format!("vol {:.0}%", (volume().get() * 100.0).round()),
            76.0,
            Color::from_rgba8(180, 195, 215, 255),
        ),
        BarItem::VolumeSlider => {
            let mut pal = SliderPalette::from_theme(&llimphi_theme::Theme::dark());
            pal.label_width = 0.0;
            pal.value_width = 0.0;
            pal.track_width = 120.0;
            pal.row_height = 34.0;
            pal.track_thickness = 8.0;
            View::new(Style {
                size: Size { width: length(128.0_f32), height: length(34.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .children(vec![slider_view::<Msg, _>(
                "",
                volume().get(),
                0.0,
                2.0,
                &pal,
                |phase, delta| match phase {
                    DragPhase::Move => Some(Msg::Command(VolumeBy { delta })),
                    DragPhase::End => None,
                },
            )])
        }
        BarItem::Title => {
            bar_label(media_title_string(), 300.0, Color::from_rgba8(200, 212, 230, 255))
        }
    }
}

/// Onda de pista completa (tipo Audacity).
pub(crate) fn fulltrack_waveform_view() -> View<Msg> {
    let peaks: Option<Vec<(f32, f32)>> = waveform_slot()
        .lock()
        .as_ref()
        .filter(|w| !w.is_empty())
        .map(|w| w.peaks().to_vec());
    let Some(peaks) = peaks else {
        return waveform_panel();
    };

    let stroke = Color::from_rgba8(120, 220, 170, 255);
    let center_color = Color::from_rgba8(64, 74, 90, 255);
    let playhead_color = Color::from_rgba8(242, 184, 92, 255);

    View::new(Style {
        size: Size { width: auto(), height: percent(1.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(14, 16, 22, 255))
    .radius(8.0)
    .on_click_at(|lx, _ly, w, _h| {
        let f = (lx / w.max(1.0)).clamp(0.0, 1.0);
        Some(Msg::Command(MediaCommand::SeekTo { fraction: f }))
    })
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

        let mut center = BezPath::new();
        center.move_to((ix as f64, mid as f64));
        center.line_to(((ix + iw) as f64, mid as f64));
        scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, center_color, None, &center);

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

        let s = playback_snapshot();
        if let Some(dur) = s.duration {
            let d = dur.as_secs_f32();
            if d > 0.0 {
                let f = (s.position.as_secs_f32() / d).clamp(0.0, 1.0);
                let px = ix + f * iw;
                let mut ph = BezPath::new();
                ph.move_to((px as f64, iy as f64));
                ph.line_to((px as f64, (iy + ih) as f64));
                scene.stroke(&Stroke::new(2.0), Affine::IDENTITY, playhead_color, None, &ph);
            }
        }
    })
}

/// Visor de waveform en vivo.
pub(crate) fn waveform_panel() -> View<Msg> {
    let probe = audio_probe_slot().get().cloned().flatten();
    let palette = llimphi_widget_waveform::WaveformPalette {
        bg: Color::from_rgba8(14, 16, 22, 255),
        center: Color::from_rgba8(80, 92, 110, 255),
        stroke: Color::from_rgba8(120, 220, 170, 255),
        fill: Color::from_rgba8(120, 220, 170, 70),
        radius: 8.0,
        pad_x: 12.0,
        pad_y: 8.0,
        stroke_w: 1.2,
    };
    llimphi_widget_waveform::waveform_view(
        move |out| match probe.as_ref() {
            Some(p) => {
                let (_sr, channels) = p.snapshot(out);
                channels
            }
            None => {
                out.clear();
                0
            }
        },
        &palette,
    )
}

/// Botón compacto del row del título.
pub(crate) fn chip_button(label: &str, bg: Color, fg: Color, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(64.0_f32),
            height: length(36.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(Color::from_rgba8(80, 100, 130, 255))
    .radius(8.0)
    .text(label.to_string(), 15.0, fg)
    .on_click(msg)
}

/// Strip de medidores peak + RMS.
pub(crate) fn meters_panel() -> View<Msg> {
    let probe = audio_probe_slot().get().cloned().flatten();
    let scratch: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let levels: Arc<Mutex<Levels>> = Arc::new(Mutex::new(Levels::new()));
    let track_bg = Color::from_rgba8(34, 40, 52, 255);
    let label_color = Color::from_rgba8(150, 165, 185, 255);
    let off_color = Color::from_rgba8(80, 92, 110, 255);

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

        let pk_label = TextBlock::simple(
            "PK",
            11.0,
            label_color,
            (inner_x as f64, (inner_y - 3.0) as f64),
        );
        llimphi_ui::llimphi_text::draw_block(scene, ts, &pk_label);
        let rms_label = TextBlock::simple(
            "RMS",
            11.0,
            label_color,
            (inner_x as f64, (inner_y + bar_h + gap_y - 3.0) as f64),
        );
        llimphi_ui::llimphi_text::draw_block(scene, ts, &rms_label);

        let pk_track = KurboRect::new(
            bars_x as f64,
            inner_y as f64,
            (bars_x + bars_w) as f64,
            (inner_y + bar_h) as f64,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, track_bg, None, &pk_track);
        let rms_y = inner_y + bar_h + gap_y;
        let rms_track = KurboRect::new(
            bars_x as f64,
            rms_y as f64,
            (bars_x + bars_w) as f64,
            (rms_y + bar_h) as f64,
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, track_bg, None, &rms_track);

        let Some(probe) = probe.as_ref() else {
            let pk_off = KurboRect::new(
                bars_x as f64,
                (inner_y + bar_h - 1.0) as f64,
                (bars_x + bars_w) as f64,
                (inner_y + bar_h) as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, off_color, None, &pk_off);
            let rms_off = KurboRect::new(
                bars_x as f64,
                (rms_y + bar_h - 1.0) as f64,
                (bars_x + bars_w) as f64,
                (rms_y + bar_h) as f64,
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, off_color, None, &rms_off);
            return;
        };

        let mut snap = scratch.lock();
        let (_sr, channels) = probe.snapshot(&mut snap);
        let mut levels = levels.lock();
        levels.analyze(&snap, channels);
        let pk = levels.peak();
        let rms = levels.rms();

        let pk_w = (pk.clamp(0.0, 1.0) * bars_w).max(0.0);
        let rms_w = (rms.clamp(0.0, 1.0) * bars_w).max(0.0);

        if pk_w > 0.0 {
            let pk_fill = KurboRect::new(
                bars_x as f64,
                inner_y as f64,
                (bars_x + pk_w) as f64,
                (inner_y + bar_h) as f64,
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                level_color(pk),
                None,
                &pk_fill,
            );
        }
        if rms_w > 0.0 {
            let rms_fill = KurboRect::new(
                bars_x as f64,
                rms_y as f64,
                (bars_x + rms_w) as f64,
                (rms_y + bar_h) as f64,
            );
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                level_color(rms),
                None,
                &rms_fill,
            );
        }
    })
}

/// Gradiente verde → ámbar → rojo según el nivel.
pub(crate) fn level_color(v: f32) -> Color {
    let v = v.clamp(0.0, 1.0);
    if v < 0.5 {
        Color::from_rgba8(110, 220, 140, 255)
    } else if v < 0.85 {
        Color::from_rgba8(230, 200, 90, 255)
    } else {
        Color::from_rgba8(240, 95, 95, 255)
    }
}

/// Panel waterfall (spectrogram histórico).
pub(crate) fn waterfall_panel() -> View<Msg> {
    let probe = audio_probe_slot().get().cloned().flatten();
    let scratch: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let grid_buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let waterfall: Arc<Mutex<Waterfall>> =
        Arc::new(Mutex::new(Waterfall::new(28, 60, 40.0, 16_000.0)));
    let base_color = Color::from_rgba8(46, 36, 28, 255);

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
        let pad: f32 = 6.0;
        let inner_x = rect.x + pad;
        let inner_y = rect.y + pad;
        let inner_w = (rect.w - 2.0 * pad).max(1.0);
        let inner_h = (rect.h - 2.0 * pad).max(1.0);

        let Some(probe) = probe.as_ref() else {
            let mut center = BezPath::new();
            let mid = inner_y + inner_h * 0.5;
            center.move_to((inner_x as f64, mid as f64));
            center.line_to(((inner_x + inner_w) as f64, mid as f64));
            scene.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                base_color,
                None,
                &center,
            );
            return;
        };

        let mut snap = scratch.lock();
        let (sr, channels) = probe.snapshot(&mut snap);
        if sr == 0 {
            return;
        }
        let mut wf = waterfall.lock();
        wf.analyze(&snap, channels, sr);

        let mut grid = grid_buf.lock();
        let (rows, bands) = wf.snapshot(&mut grid);
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
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    heat_color(m),
                    None,
                    &cell,
                );
            }
        }
    })
}

/// Gradiente "heat" para el waterfall.
pub(crate) fn heat_color(v: f32) -> Color {
    let v = v.clamp(0.0, 1.0);
    if v < 0.25 {
        let t = v / 0.25;
        let r = (60.0 + 110.0 * t) as u8;
        let g = (20.0 + 30.0 * t) as u8;
        let b = (20.0 + 10.0 * t) as u8;
        Color::from_rgba8(r, g, b, 255)
    } else if v < 0.6 {
        let t = (v - 0.25) / 0.35;
        let r = (170.0 + 70.0 * t) as u8;
        let g = (50.0 + 110.0 * t) as u8;
        let b = (30.0 + 40.0 * t) as u8;
        Color::from_rgba8(r, g, b, 255)
    } else {
        let t = (v - 0.6) / 0.4;
        let r = (240.0 + 15.0 * t) as u8;
        let g = (160.0 + 80.0 * t) as u8;
        let b = (70.0 + 160.0 * t) as u8;
        Color::from_rgba8(r, g, b, 255.min((180.0 + 75.0 * t) as u8))
    }
}

// Re-export settings_content and playlist_content from vista_config
pub(crate) use crate::vista_config::{settings_content, playlist_content};
