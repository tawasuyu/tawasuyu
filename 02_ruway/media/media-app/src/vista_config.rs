use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, FlexWrap, JustifyContent, Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_icons::{icon_view, Icon};
use llimphi_widget_tabs::{tabs_view, TabsPalette, TabsSpec};
use media_core::config::MediaConfig;
use media_core::control::MediaCommand;
use media_core::toolbar::BarItem;
use llimphi_ui::View;

use crate::estado::settings;
use crate::modelo::Model;
use crate::playlist::playback_snapshot;
use crate::tipos::{BarEdit, ConfigEdit, Msg, SettingsTab};
use crate::vista::{bar_label, chip_button};

/// Chip de toggle booleano: verde "sí" / gris "no".
pub(crate) fn cfg_toggle(on: bool, edit: ConfigEdit) -> View<Msg> {
    let (label, bg) = if on {
        (rimay_localize::t("yes"), Color::from_rgba8(56, 120, 84, 255))
    } else {
        (rimay_localize::t("no"), Color::from_rgba8(74, 60, 70, 255))
    };
    chip_button(&label, bg, Color::from_rgba8(235, 240, 248, 255), Msg::ConfigEdit(edit))
}

/// Chip de acción genérico de la ventana de config.
pub(crate) fn cfg_chip(label: &str, edit: ConfigEdit) -> View<Msg> {
    chip_button(
        label,
        Color::from_rgba8(55, 65, 80, 255),
        Color::from_rgba8(220, 230, 245, 255),
        Msg::ConfigEdit(edit),
    )
}

/// Una fila de ajuste: etiqueta · valor · controles.
pub(crate) fn settings_row(label: &str, value: &str, controls: Vec<View<Msg>>) -> View<Msg> {
    let lab = View::new(Style {
        size: Size {
            width: length(148.0_f32),
            height: length(38.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        ..Default::default()
    })
    .text(label.to_string(), 13.5, Color::from_rgba8(178, 193, 214, 255));
    let val = View::new(Style {
        size: Size {
            width: length(60.0_f32),
            height: length(38.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(value.to_string(), 13.5, Color::from_rgba8(232, 238, 248, 255));
    let mut kids = vec![lab, val];
    kids.extend(controls);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(42.0_f32),
        },
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(kids)
}

/// Cabecera de sección dentro de la ventana de config.
pub(crate) fn settings_header(title: &str) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(26.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(title.to_string(), 14.5, Color::from_rgba8(118, 182, 232, 255))
}

/// Caja de contenido con scroll.
pub(crate) fn scroll_box(children: Vec<View<Msg>>, visible_h: f32, scroll: f32) -> View<Msg> {
    let inner = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        margin: TaffyRect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(-scroll),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(children);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(visible_h),
        },
        ..Default::default()
    })
    .clip(true)
    .children(vec![inner])
}

/// Fila que envuelve sus hijos a varias líneas.
pub(crate) fn wrap_row(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(8.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

/// Chip ancho.
pub(crate) fn wide_chip(label: &str, bg: Color, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(116.0_f32),
            height: length(30.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(Color::from_rgba8(80, 100, 130, 255))
    .radius(7.0)
    .text(label.to_string(), 12.5, Color::from_rgba8(225, 232, 245, 255))
    .on_click(msg)
}

/// Chip pequeño cuadrado (reorden ‹ ›).
pub(crate) fn small_chip(label: &str, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(26.0_f32),
            height: length(30.0_f32),
        },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(40, 46, 58, 255))
    .hover_fill(Color::from_rgba8(80, 100, 130, 255))
    .radius(6.0)
    .text(label.to_string(), 14.0, Color::from_rgba8(220, 230, 245, 255))
    .on_click(msg)
}

/// Una columna (mitad de ancho).
pub(crate) fn half_column(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(0.5_f32),
            height: length(352.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

/// Pone dos columnas lado a lado.
pub(crate) fn two_columns(left: Vec<View<Msg>>, right: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(356.0_f32),
        },
        gap: Size {
            width: length(18.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![half_column(left), half_column(right)])
}

/// Contenido de la pestaña Audio.
pub(crate) fn tab_audio(c: &MediaConfig) -> Vec<View<Msg>> {
    let t = rimay_localize::t;
    vec![
        settings_header(&t("media-settings-tab-audio")),
        settings_row(
            &t("media-audio-volume"),
            &format!("{:.0}%", (c.audio.volume * 100.0).round()),
            vec![
                cfg_chip("−", ConfigEdit::VolumeDelta(-0.05)),
                cfg_chip("+", ConfigEdit::VolumeDelta(0.05)),
            ],
        ),
        settings_row(&t("media-audio-eq"), "", vec![cfg_toggle(c.audio.eq_enabled, ConfigEdit::ToggleEq)]),
        settings_row(
            &t("media-audio-normalization"),
            "",
            vec![cfg_toggle(c.audio.normalization_enabled, ConfigEdit::ToggleNormalization)],
        ),
        settings_row(
            &t("media-audio-lufs-target"),
            &format!("{:.0}", c.audio.normalization_target_lufs),
            vec![
                cfg_chip("−", ConfigEdit::NormTargetDelta(-1.0)),
                cfg_chip("+", ConfigEdit::NormTargetDelta(1.0)),
            ],
        ),
        settings_row(
            &t("media-audio-downmix"),
            "",
            vec![cfg_toggle(c.audio.downmix_to_stereo, ConfigEdit::ToggleDownmix)],
        ),
    ]
}

/// Contenido de la pestaña Video.
pub(crate) fn tab_video(c: &MediaConfig) -> Vec<View<Msg>> {
    let t = rimay_localize::t;
    let v = &c.video;
    let color = vec![
        settings_header(&t("media-video-color")),
        settings_row(&t("media-video-enable"), "", vec![cfg_toggle(v.color_enabled, ConfigEdit::ToggleColor)]),
        settings_row(
            &t("media-video-brightness"),
            &format!("{:+.2}", v.brightness),
            vec![cfg_chip("−", ConfigEdit::BrightnessDelta(-0.05)), cfg_chip("+", ConfigEdit::BrightnessDelta(0.05))],
        ),
        settings_row(
            &t("media-video-contrast"),
            &format!("{:.2}", v.contrast),
            vec![cfg_chip("−", ConfigEdit::ContrastDelta(-0.05)), cfg_chip("+", ConfigEdit::ContrastDelta(0.05))],
        ),
        settings_row(
            &t("media-video-gamma"),
            &format!("{:.2}", v.gamma),
            vec![cfg_chip("−", ConfigEdit::GammaDelta(-0.05)), cfg_chip("+", ConfigEdit::GammaDelta(0.05))],
        ),
        settings_row(
            &t("media-video-saturation"),
            &format!("{:.2}", v.saturation),
            vec![cfg_chip("−", ConfigEdit::SaturationDelta(-0.05)), cfg_chip("+", ConfigEdit::SaturationDelta(0.05))],
        ),
        settings_row(
            &t("media-video-hue"),
            &format!("{:.0}°", v.hue),
            vec![cfg_chip("−", ConfigEdit::HueDelta(-10.0)), cfg_chip("+", ConfigEdit::HueDelta(10.0))],
        ),
        settings_row("", "", vec![cfg_chip(&t("media-action-reset"), ConfigEdit::ColorReset)]),
    ];
    let orient = vec![
        settings_header(&t("media-video-orientation")),
        settings_row(
            &t("media-video-rotation"),
            &format!("{}°", v.rotation),
            vec![cfg_chip(&t("media-video-rotate-cw"), ConfigEdit::RotateCw)],
        ),
        settings_row(&t("media-video-flip-h"), "", vec![cfg_toggle(v.flip_h, ConfigEdit::FlipH)]),
        settings_row(&t("media-video-flip-v"), "", vec![cfg_toggle(v.flip_v, ConfigEdit::FlipV)]),
    ];
    vec![two_columns(color, orient)]
}

/// Contenido de la pestaña Reproducción.
pub(crate) fn tab_playback(c: &MediaConfig) -> Vec<View<Msg>> {
    let t = rimay_localize::t;
    vec![
        settings_header(&t("media-playback-playlist")),
        settings_row(
            &t("media-playback-resume"),
            "",
            vec![cfg_toggle(c.playlist.resume_on_open, ConfigEdit::ToggleResumeOnOpen)],
        ),
        settings_row(
            &t("media-playback-repeat"),
            c.playlist.repeat.slug(),
            vec![cfg_chip(&t("media-action-cycle"), ConfigEdit::CycleRepeatDefault)],
        ),
        settings_row(&t("media-playback-shuffle"), "", vec![cfg_toggle(c.playlist.shuffle, ConfigEdit::ToggleShuffleDefault)]),
        settings_header(&t("media-playback-subtitles")),
        settings_row(
            &t("media-playback-autoload-sidecar"),
            "",
            vec![cfg_toggle(c.subtitles.autoload_sidecar, ConfigEdit::ToggleAutoloadSidecar)],
        ),
        settings_row(
            &t("media-playback-sub-delay"),
            &format!("{}", c.subtitles.delay_ms),
            vec![
                cfg_chip("−", ConfigEdit::SubDelayDelta(-100)),
                cfg_chip("+", ConfigEdit::SubDelayDelta(100)),
            ],
        ),
        settings_row(
            &t("media-playback-font-size"),
            &format!("{:.1}×", c.subtitles.font_scale),
            vec![
                cfg_chip("−", ConfigEdit::SubFontDelta(-0.1)),
                cfg_chip("+", ConfigEdit::SubFontDelta(0.1)),
            ],
        ),
        settings_header(&t("media-playback-behavior")),
        settings_row(
            &t("media-playback-crossfade"),
            &format!("{:.1}", c.behavior.crossfade_secs),
            vec![
                cfg_chip("−", ConfigEdit::CrossfadeDelta(-0.5)),
                cfg_chip("+", ConfigEdit::CrossfadeDelta(0.5)),
            ],
        ),
    ]
}

/// Contenido de la pestaña Controles (keymap, sólo lectura por ahora).
pub(crate) fn tab_controls() -> Vec<View<Msg>> {
    let t = rimay_localize::t;
    let s = settings();
    let keys: Vec<View<Msg>> = s
        .keymap
        .bindings
        .iter()
        .map(|b| {
            View::new(Style {
                size: Size {
                    width: length(112.0_f32),
                    height: length(28.0_f32),
                },
                justify_content: Some(JustifyContent::Center),
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(Color::from_rgba8(40, 46, 58, 255))
            .radius(6.0)
            .text(
                format!("{} · {}", b.chord.display(), short_action(&b.command)),
                11.5,
                Color::from_rgba8(200, 212, 228, 255),
            )
        })
        .collect();
    vec![
        settings_header(&t("media-controls-header")),
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(40.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            t("media-controls-hint"),
            12.5,
            Color::from_rgba8(150, 165, 185, 255),
        ),
        wrap_row(keys),
    ]
}

/// Etiqueta corta de un comando para el chip de controles.
pub(crate) fn short_action(cmd: &MediaCommand) -> &'static str {
    use MediaCommand::*;
    match cmd {
        TogglePause => "play",
        SeekBy { .. } => "seek",
        SeekTo { .. } => "ir a",
        VolumeBy { .. } | SetVolume { .. } => "vol",
        ToggleMute => "mute",
        NextTrack => "sig",
        PrevTrack => "ant",
        ChapterNext | ChapterPrev => "cap",
        SpeedStep { .. } | SetSpeed { .. } => "vel",
        CycleRepeat => "rep",
        ToggleShuffle => "shuf",
        Snapshot => "snap",
        ToggleRecord => "rec",
        Script { .. } => "script",
        EqToggle | EqBandBy { .. } | EqReset => "eq",
        AvSyncBy { .. } | AvSyncReset => "sync",
        ColorToggle | ColorBy { .. } | ColorReset => "color",
        _ => "acción",
    }
}

/// Ícono del set canónico para un [`BarItem`] de acción.
pub(crate) fn bar_item_icon(item: BarItem) -> Option<Icon> {
    Some(match item {
        BarItem::PlayPause => Icon::Play,
        BarItem::Stop => Icon::Stop,
        BarItem::Prev => Icon::SkipBack,
        BarItem::Next => Icon::SkipForward,
        BarItem::SeekBack => Icon::Rewind,
        BarItem::SeekForward => Icon::FastForward,
        BarItem::VolumeDown => Icon::Minus,
        BarItem::VolumeUp => Icon::Plus,
        BarItem::Mute => Icon::VolumeMute,
        BarItem::Repeat => Icon::Repeat,
        BarItem::Shuffle => Icon::Shuffle,
        BarItem::SpeedDown => Icon::ChevronDown,
        BarItem::SpeedUp => Icon::ChevronUp,
        BarItem::SpeedReset => Icon::Gauge,
        BarItem::Snapshot => Icon::Camera,
        BarItem::Record => Icon::Record,
        BarItem::Equalizer => Icon::Equalizer,
        BarItem::Settings => Icon::Settings,
        _ => return None,
    })
}

/// Chip del editor de barras.
pub(crate) fn editor_item_chip(item: BarItem, bg: Color, msg: Msg) -> View<Msg> {
    let fg = Color::from_rgba8(225, 232, 245, 255);
    let mut kids: Vec<View<Msg>> = Vec::new();
    if let Some(ic) = bar_item_icon(item) {
        kids.push(
            View::new(Style {
                size: Size {
                    width: length(20.0_f32),
                    height: length(22.0_f32),
                },
                ..Default::default()
            })
            .children(vec![icon_view::<Msg>(ic, fg, 1.8)]),
        );
    }
    kids.push(
        View::new(Style {
            size: Size {
                width: length(84.0_f32),
                height: length(22.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(item.label().to_string(), 11.5, fg),
    );
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: length(124.0_f32),
            height: length(30.0_f32),
        },
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(4.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(Color::from_rgba8(80, 100, 130, 255))
    .radius(7.0)
    .on_click(msg)
    .children(kids)
}

pub(crate) fn tab_bars(model: &Model) -> Vec<View<Msg>> {
    let t = rimay_localize::t;
    let tb = &model.config.toolbar;
    let mut out: Vec<View<Msg>> = vec![settings_header(&t("media-bars-header"))];

    for (bi, bar) in tb.bars.iter().enumerate() {
        let head = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            gap: Size {
                width: length(8.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![
            bar_label(bar.display_name(bi + 1), 86.0, Color::from_rgba8(118, 182, 232, 255)),
            wide_chip(
                bar.position.label(),
                Color::from_rgba8(48, 66, 80, 255),
                Msg::BarEdit(BarEdit::TogglePosition(bi)),
            ),
            wide_chip(
                &if bar.enabled {
                    rimay_localize::t("media-bars-on")
                } else {
                    rimay_localize::t("media-bars-off")
                },
                if bar.enabled { Color::from_rgba8(46, 84, 64, 255) } else { Color::from_rgba8(70, 58, 64, 255) },
                Msg::BarEdit(BarEdit::ToggleEnabled(bi)),
            ),
            wide_chip(
                &if bar.autohide {
                    rimay_localize::t("media-bars-autohide-on")
                } else {
                    rimay_localize::t("media-bars-autohide-off")
                },
                if bar.autohide { Color::from_rgba8(60, 64, 46, 255) } else { Color::from_rgba8(48, 54, 66, 255) },
                Msg::BarEdit(BarEdit::ToggleAutohide(bi)),
            ),
            wide_chip(&rimay_localize::t("media-bars-remove-bar"), Color::from_rgba8(74, 58, 64, 255), Msg::BarEdit(BarEdit::RemoveBar(bi))),
        ]);
        let chips: Vec<View<Msg>> = bar
            .items
            .iter()
            .enumerate()
            .flat_map(|(pi, &it)| {
                vec![
                    editor_item_chip(
                        it,
                        Color::from_rgba8(52, 60, 74, 255),
                        Msg::BarEdit(BarEdit::RemoveItem(bi, pi)),
                    ),
                    small_chip("‹", Msg::BarEdit(BarEdit::Nudge(bi, pi, -1))),
                    small_chip("›", Msg::BarEdit(BarEdit::Nudge(bi, pi, 1))),
                ]
            })
            .collect();
        out.push(head);
        out.push(wrap_row(chips));
    }

    let mut targets: Vec<View<Msg>> = (0..tb.bars.len())
        .map(|i| {
            let bg = if i == model.bar_target {
                Color::from_rgba8(60, 110, 150, 255)
            } else {
                Color::from_rgba8(48, 54, 66, 255)
            };
            wide_chip(&format!("→ {} {}", rimay_localize::t("media-bars-bar-label"), i + 1), bg, Msg::BarEdit(BarEdit::SetTarget(i)))
        })
        .collect();
    targets.push(wide_chip(&rimay_localize::t("media-bars-add-bar"), Color::from_rgba8(48, 70, 58, 255), Msg::BarEdit(BarEdit::AddBar)));

    let palette: Vec<View<Msg>> = BarItem::ALL
        .iter()
        .map(|&it| {
            editor_item_chip(
                it,
                Color::from_rgba8(46, 54, 68, 255),
                Msg::BarEdit(BarEdit::AddItem(model.bar_target, it)),
            )
        })
        .collect();

    out.push(settings_header(&t("media-bars-add-items-to")));
    out.push(wrap_row(targets));
    out.push(wrap_row(palette));
    out
}

/// Contenido de la ventana OS de configuración.
pub(crate) fn settings_content(model: &Model) -> View<Msg> {
    let c = &model.config;

    let rows = match model.settings_tab {
        SettingsTab::Audio => tab_audio(c),
        SettingsTab::Video => tab_video(c),
        SettingsTab::Playback => tab_playback(c),
        SettingsTab::Bars => tab_bars(model),
        SettingsTab::Controls => tab_controls(),
    };
    let content = scroll_box(rows, 486.0_f32, model.settings_scroll);

    let labels: Vec<String> = SettingsTab::ALL.iter().map(|t| t.label().to_string()).collect();
    let active = SettingsTab::ALL
        .iter()
        .position(|&t| t == model.settings_tab)
        .unwrap_or(0);
    let tabs = tabs_view(TabsSpec {
        labels,
        active,
        on_select: |i: usize| Msg::SettingsTab(SettingsTab::ALL[i]),
        content,
        tab_height: 40.0,
        palette: TabsPalette::from_theme(&llimphi_theme::Theme::dark()),
        tab_width: None,
    });

    let footer = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(
        rimay_localize::t("media-settings-footer"),
        11.5,
        Color::from_rgba8(140, 152, 170, 255),
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        padding: TaffyRect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(14.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(24, 28, 36, 255))
    .children(vec![tabs, footer])
}

/// Contenido de la ventana OS de lista de reproducción.
pub(crate) fn playlist_content() -> View<Msg> {
    use crate::estado::playlist_labels_slot;
    let labels = playlist_labels_slot().lock().clone();
    let cur = playback_snapshot().idx;
    let header = settings_header(&rimay_localize::t("media-playlist-header"));

    let rows: Vec<View<Msg>> = match (!labels.is_empty()).then_some(&labels) {
        Some(ls) if !ls.is_empty() => ls
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let active = i == cur;
                let bg = if active {
                    Color::from_rgba8(48, 86, 120, 255)
                } else {
                    Color::from_rgba8(30, 36, 46, 255)
                };
                let fg = if active {
                    Color::from_rgba8(236, 243, 250, 255)
                } else {
                    Color::from_rgba8(196, 206, 222, 255)
                };
                View::new(Style {
                    flex_direction: FlexDirection::Row,
                    size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
                    padding: TaffyRect {
                        left: length(10.0_f32),
                        right: length(10.0_f32),
                        top: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    align_items: Some(AlignItems::Center),
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .fill(bg)
                .hover_fill(Color::from_rgba8(60, 72, 90, 255))
                .radius(6.0)
                .text(format!("{:>2}.  {name}", i + 1), 13.0, fg)
                .on_click(Msg::JumpTrack(i))
            })
            .collect(),
        _ => vec![View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(rimay_localize::t("media-playlist-empty"), 13.0, Color::from_rgba8(150, 162, 182, 255))],
    };

    let list = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: auto() },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        ..Default::default()
    })
    .children(rows);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        padding: TaffyRect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(24, 28, 36, 255))
    .clip(true)
    .children(vec![header, list])
}
