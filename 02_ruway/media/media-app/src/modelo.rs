use std::sync::Arc;
use std::time::{Duration, Instant};

use llimphi_motion::{animate, motion, Tween};
use llimphi_module_command_palette::{
    self as palette, Command as PaletteCommand, PaletteMsg, PaletteState,
};
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use media_core::config::MediaConfig;
use media_core::control::MediaCommand;
use media_core::layout::PanelId as TileId;

use crate::comandos::{
    apply_command, apply_palette, build_command_catalog, chord_from_event,
};
use crate::config_io::{
    apply_bar_edit, apply_config_edit, apply_media_config, load_layout, media_config_slot,
    save_history, save_layout, save_media_config, save_bookmarks,
};
use crate::estado::{
    chapters_slot, config_slot, current_media_path, osd_now, pipeline_slot,
    playlist_slot, settings, settings_slot, waveform_slot, CONFIG_WIN, PLAYLIST_WIN, TICK_MS,
};
use crate::media_io::media_title_string;
use crate::pipeline::media_host;
use crate::playlist::{jump_playlist_to, playback_snapshot, record_playback_progress};
use crate::estado::{reload_settings, spawn_controles_watcher};
use crate::tipos::{Msg, SettingsTab};
use crate::vista::{
    context_menu, cover_hero, menubar_spec, osd_overlay, palette_overlay,
    playlist_content, settings_content, subtitle_strip, toolbar_view_at,
    fulltrack_waveform_view, waterfall_panel, meters_panel, app_menu, handle_menu_command,
};

pub(crate) struct Model {
    pub(crate) frames: u64,
    pub(crate) started_at: Instant,
    pub(crate) tile_order: Vec<TileId>,
    pub(crate) help_open: bool,
    pub(crate) palette: Option<PaletteState>,
    pub(crate) palette_commands: Vec<PaletteCommand>,
    pub(crate) palette_cmds: Vec<MediaCommand>,
    pub(crate) viewport: (f32, f32),
    pub(crate) menu_open: Option<usize>,
    pub(crate) menu_active: usize,
    pub(crate) menu_anim: Tween<f32>,
    pub(crate) context_menu: Option<(f32, f32)>,
    pub(crate) config: MediaConfig,
    pub(crate) settings_open: bool,
    pub(crate) settings_tab: SettingsTab,
    pub(crate) bar_target: usize,
    pub(crate) settings_scroll: f32,
    pub(crate) visualizers_open: bool,
    pub(crate) playlist_open: bool,
    pub(crate) _host: Option<pata_host::HostClient>,
}

pub(crate) struct MediaApp;

impl App for MediaApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "media · player"
    }

    fn app_id() -> Option<&'static str> {
        Some("tawasuyu.media")
    }

    fn window_title(_model: &Self::Model) -> Option<String> {
        let t = media_title_string();
        let t = t.trim();
        Some(if t.is_empty() {
            "media · player".to_string()
        } else {
            format!("media — {t}")
        })
    }

    fn secondary_view(model: &Self::Model, key: u64) -> Option<View<Self::Msg>> {
        match key {
            CONFIG_WIN if model.settings_open => Some(settings_content(model)),
            PLAYLIST_WIN if model.playlist_open => Some(playlist_content()),
            _ => None,
        }
    }

    fn secondary_title(_model: &Self::Model, key: u64) -> Option<String> {
        match key {
            CONFIG_WIN => Some(rimay_localize::t("media-win-config-title")),
            PLAYLIST_WIN => Some(rimay_localize::t("media-win-playlist-title")),
            _ => None,
        }
    }

    fn on_secondary_close(_model: &Self::Model, key: u64) -> Option<Self::Msg> {
        match key {
            CONFIG_WIN => Some(Msg::SettingsClosed),
            PLAYLIST_WIN => Some(Msg::PlaylistClosed),
            _ => None,
        }
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        handle.spawn_periodic(Duration::from_millis(TICK_MS), || Msg::Tick);
        spawn_controles_watcher(handle);
        if let Some(path) = current_media_path() {
            handle.spawn(move || {
                match foreign_av::decode_peaks(&path, 1600) {
                    Ok(w) => *waveform_slot().lock() = Some(w),
                    Err(e) => eprintln!("media-app: escaneo de onda: {e}"),
                }
                Msg::WaveformReady
            });
        }
        let (palette_commands, palette_cmds) = build_command_catalog(&settings());
        let config = media_config_slot().get().cloned().unwrap_or_default();
        Model {
            frames: 0,
            started_at: Instant::now(),
            tile_order: load_layout(),
            help_open: false,
            palette: None,
            palette_commands,
            palette_cmds,
            viewport: (960.0, 540.0),
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            context_menu: None,
            config,
            settings_open: false,
            settings_tab: SettingsTab::Audio,
            bar_target: 0,
            settings_scroll: 0.0,
            visualizers_open: false,
            playlist_open: false,
            _host: media_host(handle),
        }
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => {
                record_playback_progress(model.frames);
                Model {
                    frames: model.frames.wrapping_add(1),
                    ..model
                }
            }
            Msg::SwapTile { from, to } => {
                let mut m = model;
                if from != to && from < m.tile_order.len() && to < m.tile_order.len() {
                    m.tile_order.swap(from, to);
                    save_layout(&m.tile_order);
                }
                m
            }
            Msg::Command(cmd) => {
                apply_command(cmd);
                model
            }
            Msg::HostActivate(id) => {
                let mut m = model;
                match id {
                    0 => handle.dispatch(Msg::ToggleSettings),
                    1 => handle.dispatch(Msg::TogglePlaylist),
                    2 => m.visualizers_open = !m.visualizers_open,
                    3 => handle.dispatch(Msg::ToggleHelp),
                    _ => {}
                }
                m
            }
            Msg::ToggleHelp => {
                let mut m = model;
                m.help_open = !m.help_open;
                m
            }
            Msg::ToggleSettings => {
                let mut m = model;
                m.settings_open = !m.settings_open;
                m.settings_scroll = 0.0;
                if m.settings_open {
                    handle.open_window(CONFIG_WIN, &rimay_localize::t("media-win-config-title"), 760, 600);
                } else {
                    handle.close_window(CONFIG_WIN);
                }
                m
            }
            Msg::SettingsClosed => {
                let mut m = model;
                m.settings_open = false;
                m
            }
            Msg::TogglePlaylist => {
                let mut m = model;
                m.playlist_open = !m.playlist_open;
                if m.playlist_open {
                    handle.open_window(PLAYLIST_WIN, &rimay_localize::t("media-win-playlist-title"), 420, 560);
                } else {
                    handle.close_window(PLAYLIST_WIN);
                }
                m
            }
            Msg::PlaylistClosed => {
                let mut m = model;
                m.playlist_open = false;
                m
            }
            Msg::JumpTrack(i) => {
                jump_playlist_to(i);
                model
            }
            Msg::WaveformReady => model,
            Msg::ConfigEdit(edit) => {
                let mut m = model;
                apply_config_edit(&mut m.config, edit);
                m.config = std::mem::take(&mut m.config).sanitized();
                apply_media_config(&m.config);
                save_media_config(&m.config);
                m
            }
            Msg::SettingsTab(tab) => {
                let mut m = model;
                if m.settings_tab != tab {
                    m.settings_scroll = 0.0;
                }
                m.settings_tab = tab;
                m
            }
            Msg::SettingsScroll(dy) => {
                let mut m = model;
                m.settings_scroll = (m.settings_scroll - dy * 28.0).clamp(0.0, 900.0);
                m
            }
            Msg::BarEdit(edit) => {
                let mut m = model;
                apply_bar_edit(&mut m, edit);
                m.config.toolbar = std::mem::take(&mut m.config.toolbar).sanitized();
                m.bar_target = m.bar_target.min(m.config.toolbar.bars.len().saturating_sub(1));
                save_media_config(&m.config);
                m
            }
            Msg::ReloadConfig => {
                reload_settings();
                let (palette_commands, palette_cmds) = build_command_catalog(&settings());
                Model {
                    palette_commands,
                    palette_cmds,
                    ..model
                }
            }
            Msg::Palette(pm) => apply_palette(model, pm, handle),
            Msg::MenuOpen(which) => {
                let mut m = model;
                m.menu_open = which;
                m.menu_active = usize::MAX;
                m.context_menu = None;
                if which.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
                m
            }
            Msg::MenuNav(dir) => {
                let mut m = model;
                if let Some(mi) = m.menu_open {
                    let menu = app_menu();
                    m.menu_active = llimphi_widget_menubar::menubar_nav(&menu, mi, m.menu_active, dir);
                }
                m
            }
            Msg::MenuActivate => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu();
                    if let Some(cmd) = llimphi_widget_menubar::menubar_command_at(&menu, mi, model.menu_active) {
                        let mut m = model;
                        m.menu_open = None;
                        m.context_menu = None;
                        return handle_menu_command(m, &cmd, handle);
                    }
                }
                model
            }
            Msg::MenuTick => model,
            Msg::CloseMenus => {
                let mut m = model;
                m.menu_open = None;
                m.menu_active = usize::MAX;
                m.context_menu = None;
                m
            }
            Msg::MenuCommand(cmd) => {
                let mut m = model;
                m.menu_open = None;
                m.context_menu = None;
                handle_menu_command(m, &cmd, handle)
            }
            Msg::ContextMenuOpen(x, y) => {
                let mut m = model;
                m.menu_open = None;
                m.context_menu = Some((x, y));
                m
            }
        }
    }

    fn on_wheel(
        model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _modifiers: Modifiers,
    ) -> Option<Self::Msg> {
        if model.settings_open {
            return Some(Msg::SettingsScroll(delta.y));
        }
        None
    }

    fn on_key(model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if let Some(state) = model.palette.as_ref() {
            return palette::on_key(state, event).map(Msg::Palette);
        }
        if event.state != KeyState::Pressed {
            return None;
        }
        if palette::open_shortcut(event) {
            return Some(Msg::Palette(PaletteMsg::Open));
        }
        if let Some(mi) = model.menu_open {
            let n = app_menu().menus.len().max(1);
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        if matches!(event.key, Key::Named(NamedKey::Escape))
            && (model.menu_open.is_some() || model.context_menu.is_some())
        {
            return Some(Msg::CloseMenus);
        }
        match &event.key {
            Key::Character(c) if c == "?" => return Some(Msg::ToggleHelp),
            Key::Named(NamedKey::Escape) if model.help_open => return Some(Msg::ToggleHelp),
            Key::Named(NamedKey::Escape) if model.settings_open => return Some(Msg::ToggleSettings),
            Key::Named(NamedKey::F2) => return Some(Msg::ToggleSettings),
            Key::Named(NamedKey::F5) => return Some(Msg::ReloadConfig),
            _ => {}
        }
        let chord = chord_from_event(event)?;
        settings().keymap.resolve(&chord).cloned().map(Msg::Command)
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        if let Some((x, y)) = model.context_menu {
            return Some(context_menu(model, x, y));
        }
        let menu = app_menu();
        if let Some(v) = llimphi_widget_menubar::menubar_overlay_animated(
            &menubar_spec(&menu, model, &llimphi_theme::Theme::dark()),
            model.menu_active,
            model.menu_anim.value(),
        ) {
            return Some(v);
        }
        if let Some(state) = model.palette.as_ref() {
            return Some(palette_overlay(model, state));
        }
        if !model.help_open {
            return None;
        }
        let theme = llimphi_theme::Theme::dark();
        let t = rimay_localize::t;
        let acciones: Vec<llimphi_widget_shortcuts_help::ShortcutEntry> = settings()
            .keymap
            .bindings
            .iter()
            .map(|b| llimphi_widget_shortcuts_help::ShortcutEntry::new(b.chord.display(), b.command.describe()))
            .collect();
        Some(llimphi_widget_shortcuts_help::shortcuts_help_view(llimphi_widget_shortcuts_help::ShortcutsHelpSpec {
            title: t("media-help-title"),
            groups: vec![
                llimphi_widget_shortcuts_help::ShortcutGroup::new(t("media-help-group-playback"), acciones),
                llimphi_widget_shortcuts_help::ShortcutGroup::new(
                    t("help"),
                    vec![
                        llimphi_widget_shortcuts_help::ShortcutEntry::new("?", t("media-help-toggle")),
                        llimphi_widget_shortcuts_help::ShortcutEntry::new("Esc", t("media-help-close")),
                        llimphi_widget_shortcuts_help::ShortcutEntry::new("F5", t("media-help-reload")),
                        llimphi_widget_shortcuts_help::ShortcutEntry::new("Ctrl+Shift+P", t("command-palette")),
                    ],
                ),
            ],
            viewport: model.viewport,
            on_dismiss: Msg::ToggleHelp,
            palette: llimphi_widget_shortcuts_help::ShortcutsHelpPalette::from_theme(&theme),
        }))
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        use llimphi_ui::llimphi_layout::taffy::{
            prelude::{auto, length, percent, FlexDirection, Size, Style},
            AlignItems, JustifyContent, Rect as TaffyRect,
        };
        use llimphi_ui::llimphi_raster::peniko::Color;
        use llimphi_ui::llimphi_raster::kurbo::Affine;
        use llimphi_widget_menubar::{menubar_view, DEFAULT_HEIGHT as MENU_H};
        use media_core::viewport::compute_layout;
        use media_core::sync::FramePlan;
        use crate::estado::{viewcontrol, SEEK_FORCE};
        use crate::pipeline::pipeline_for;
        use crate::playlist::current_audio_position;
        use crate::media_io::fmt_secs;
        use std::sync::atomic::Ordering;
        use std::time::Instant;

        let secs = model.started_at.elapsed().as_secs_f32().max(0.001);
        let fps = model.frames as f32 / secs;

        let theme = llimphi_theme::Theme::dark();
        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));

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
        .gpu_paint_with(move |device, queue, encoder, view, rect, viewport| {
            let pipe = pipeline_for(device, queue);
            let now = Instant::now();
            let wall_dt = {
                let mut last = pipe.last_tick.lock();
                let d = now - *last;
                *last = now;
                d
            };

            let dt = wall_dt;
            let audio_pos = current_audio_position();

            let force = SEEK_FORCE.load(Ordering::Relaxed);
            let do_tick = !crate::estado::pause().is_paused() || force;

            let mut buf = pipe.buf.lock();
            let mut src = pipe.source.lock();
            if do_tick {
                if let Some((w, h)) = src.tick(dt, &mut buf) {
                    let frame_pts = src.pts();
                    drop(src);
                    let present = force
                        || match (audio_pos, frame_pts) {
                            (Some(audio), Some(pts)) => {
                                !matches!(pipe.sync.lock().plan(audio, pts), FramePlan::Drop)
                            }
                            _ => true,
                        };
                    if present {
                        pipe.surface.upload(&buf, w, h);
                        *pipe.last_dim.lock() = (w, h);
                        if force {
                            SEEK_FORCE.store(false, Ordering::Relaxed);
                        }
                    }
                } else {
                    drop(src);
                }
            } else {
                drop(src);
            }
            drop(buf);
            let (tw, th) = pipe.surface.size();
            if tw > 0 && th > 0 {
                let ctl = viewcontrol().lock().clone();
                let lay = compute_layout(tw as f32, th as f32, rect.w, rect.h, &ctl);
                let dst = [rect.x + lay.dst.x, rect.y + lay.dst.y, lay.dst.w, lay.dst.h];
                let src_uv = [
                    lay.src.x / tw as f32,
                    lay.src.y / th as f32,
                    lay.src.w / tw as f32,
                    lay.src.h / th as f32,
                ];
                let clip = [rect.x, rect.y, rect.w, rect.h];
                pipe.surface
                    .blit_layout(queue, encoder, view, dst, src_uv, clip, viewport);
            } else {
                pipe.surface.blit(queue, encoder, view, rect, viewport);
            }
        });

        let subs_strip = subtitle_strip();
        let above_bars = toolbar_view_at(model, media_core::toolbar::BarPosition::Above);
        let below_bars = toolbar_view_at(model, media_core::toolbar::BarPosition::Below);

        let time_label = {
            let s = playback_snapshot();
            if s.present {
                let dur = s.duration.unwrap_or(std::time::Duration::ZERO);
                let track = if s.len > 1 {
                    format!(" · trk {}/{}", s.idx + 1, s.len)
                } else {
                    String::new()
                };
                format!(" · {} / {}{}", fmt_secs(s.position), fmt_secs(dur), track)
            } else {
                String::new()
            }
        };
        let footer = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            format!("ticks {} · ui ≈ {fps:.1} fps{time_label}", model.frames),
            13.0,
            Color::from_rgba8(150, 165, 185, 255),
        );

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
        .children({
            let mut kids: Vec<View<Msg>> = Vec::new();
            if let Some(v) = above_bars {
                kids.push(v);
            }
            kids.push(cover_hero().unwrap_or(canvas));
            kids.push(subs_strip);
            if let Some(v) = below_bars {
                kids.push(v);
            }
            if model.visualizers_open {
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
                .children(vec![fulltrack_waveform_view(), waterfall_panel(), meters_panel()]);
                kids.push(visualizers);
            }
            kids.push(footer);
            kids
        });

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(22, 26, 34, 255))
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children({
            let mut kids = vec![menubar, content];
            if let Some(osd) = osd_overlay(model) {
                kids.push(osd);
            }
            kids
        })
    }
}
