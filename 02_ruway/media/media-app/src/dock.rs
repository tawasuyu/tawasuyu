//! dock — sidebars de **dientes** in-app del reproductor.
//!
//! Un rail vertical de pestañas (dientes, `llimphi-widget-dock-rail`) flota al
//! borde interno izquierdo; al activar un diente se despliega su panel al
//! costado con las features/controles de esa sección (Cola, Config,
//! Visualizadores, Ayuda). Mismo patrón canónico que cosmos
//! (`cosmos-app-llimphi/src/chrome/dock.rs`): rail como overlay absoluto +
//! panel del item activo como pane al costado.

use llimphi_icons::{icon_view, Icon};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    style::Position,
    AlignItems, JustifyContent, Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette};

use crate::estado::settings;
use crate::modelo::Model;
use crate::tipos::{InputTarget, Msg};
use crate::vista::{
    fulltrack_waveform_view, meters_panel, playlist_content, settings_content, waterfall_panel,
};

/// Ancho de la franja del rail (px).
pub(crate) const DOCK_RAIL_W: f32 = 40.0;
/// Ancho del panel desplegado (px).
pub(crate) const DOCK_PANEL_W: f32 = 380.0;

/// Los dientes del rail, en orden de presentación.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DockTooth {
    Cola,
    Perfiles,
    Config,
    Visualizadores,
    Ayuda,
}

impl DockTooth {
    pub(crate) const ALL: &'static [DockTooth] = &[
        DockTooth::Cola,
        DockTooth::Perfiles,
        DockTooth::Config,
        DockTooth::Visualizadores,
        DockTooth::Ayuda,
    ];

    pub(crate) fn id(self) -> u64 {
        match self {
            DockTooth::Cola => 0,
            DockTooth::Perfiles => 1,
            DockTooth::Config => 2,
            DockTooth::Visualizadores => 3,
            DockTooth::Ayuda => 4,
        }
    }

    pub(crate) fn from_id(id: u64) -> Option<Self> {
        DockTooth::ALL.iter().copied().find(|t| t.id() == id)
    }

    fn icon(self) -> Icon {
        match self {
            DockTooth::Cola => Icon::Music,
            DockTooth::Perfiles => Icon::Home,
            DockTooth::Config => Icon::Settings,
            DockTooth::Visualizadores => Icon::Equalizer,
            DockTooth::Ayuda => Icon::Info,
        }
    }

    fn title(self) -> String {
        let t = rimay_localize::t;
        match self {
            DockTooth::Cola => t("media-menu-playlist"),
            DockTooth::Perfiles => t("media-dock-perfiles"),
            DockTooth::Config => t("settings"),
            DockTooth::Visualizadores => t("media-menu-visualizers"),
            DockTooth::Ayuda => t("help"),
        }
    }
}

/// El rail de dientes, como **overlay absoluto** pegado al borde interno
/// izquierdo (flota sobre el panel/canvas).
pub(crate) fn dock_rail_overlay(model: &Model, theme: &Theme) -> View<Msg> {
    let active = model.dock_active;
    let items: Vec<DockRailItem> = DockTooth::ALL
        .iter()
        .map(|t| DockRailItem {
            id: t.id(),
            active: active == Some(t.id()),
        })
        .collect();
    let rail = dock_rail_view(
        &items,
        DOCK_RAIL_W,
        &DockRailPalette::from_theme(theme),
        |id, size, color| {
            let icon = DockTooth::from_id(id).map(|t| t.icon()).unwrap_or(Icon::Info);
            icon_view::<Msg>(icon, color, size / 12.0)
        },
        Msg::DockActivate,
        |payload| Some(Msg::DockDrop(payload)),
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
            width: length(DOCK_RAIL_W),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![rail])
}

/// El panel del diente activo (sin el rail), o `None` si está colapsado.
pub(crate) fn dock_panel(model: &Model, theme: &Theme) -> Option<View<Msg>> {
    let tooth = DockTooth::from_id(model.dock_active?)?;
    let body = match tooth {
        DockTooth::Cola => playlist_content(),
        DockTooth::Perfiles => perfiles_panel(model, theme),
        DockTooth::Config => settings_content(model),
        DockTooth::Visualizadores => visualizers_panel(),
        DockTooth::Ayuda => help_panel(),
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: TaffyRect {
            left: length(DOCK_RAIL_W),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text(tooth.title(), 14.5, Color::from_rgba8(118, 182, 232, 255));

    Some(
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: length(DOCK_PANEL_W),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            gap: Size {
                width: length(0.0_f32),
                height: length(6.0_f32),
            },
            padding: TaffyRect {
                left: length(6.0_f32),
                right: length(6.0_f32),
                top: length(8.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .children(vec![header, body]),
    )
}

/// Panel de visualizadores: onda completa + waterfall + medidores.
fn visualizers_panel() -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .children(vec![fulltrack_waveform_view(), waterfall_panel(), meters_panel()])
}

/// Botón ancho del panel de perfiles.
fn pbtn(label: String, bg: Color, fg: Color, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: auto(),
            height: length(28.0_f32),
        },
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
    .hover_fill(Color::from_rgba8(70, 92, 120, 255))
    .radius(6.0)
    .text(label, 12.5, fg)
    .on_click(msg)
}

/// Botón cuadrado pequeño (✕ / candado).
fn psquare(label: &str, bg: Color, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(30.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(Color::from_rgba8(70, 92, 120, 255))
    .radius(6.0)
    .text(label.to_string(), 13.0, Color::from_rgba8(225, 232, 245, 255))
    .on_click(msg)
}

/// Fila horizontal de botones.
fn prow(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

/// Cabecera de sección del panel.
fn psection(title: String) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(title, 13.0, Color::from_rgba8(118, 182, 232, 255))
}

/// Panel de Perfiles: crear/seleccionar/bloquear perfiles + sus playlists
/// (cargadas recursivamente de directorios y guardadas por perfil).
fn perfiles_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let t = rimay_localize::t;
    let mut kids: Vec<View<Msg>> = Vec::new();

    // Input abierto (nombre / clave / ruta).
    if let Some(target) = model.prof_focus.as_ref() {
        let ph = match target {
            InputTarget::NewProfile => t("media-prof-name-ph"),
            InputTarget::Unlock(_) | InputTarget::SetPass => t("media-prof-pass-ph"),
            InputTarget::AddDir => t("media-prof-dir-ph"),
            InputTarget::OpenPath => t("media-prof-open-ph"),
        };
        kids.push(psection(t("media-prof-input-hint")));
        let pal = TextInputPalette::from_theme(theme);
        let input = text_input_view(
            &model.prof_input,
            &ph,
            true,
            &pal,
            Msg::ProfileFocus(target.clone()),
        );
        kids.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(34.0_f32),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .children(vec![input]),
        );
    }

    // Línea de estado (errores / confirmaciones).
    if let Some(msg) = model.prof_msg.as_ref() {
        kids.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(22.0_f32),
                },
                flex_shrink: 0.0,
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text(msg.clone(), 12.0, Color::from_rgba8(232, 200, 130, 255)),
        );
    }

    // Abrir un medio suelto en caliente (también por arrastre a la ventana).
    kids.push(prow(vec![pbtn(
        t("media-prof-open"),
        Color::from_rgba8(48, 60, 78, 255),
        Color::from_rgba8(220, 230, 245, 255),
        Msg::ProfileFocus(InputTarget::OpenPath),
    )]));

    // Nuevo perfil.
    kids.push(prow(vec![pbtn(
        t("media-prof-new"),
        Color::from_rgba8(48, 70, 58, 255),
        Color::from_rgba8(220, 235, 226, 255),
        Msg::ProfileFocus(InputTarget::NewProfile),
    )]));

    // Lista de perfiles.
    kids.push(psection(t("media-dock-perfiles")));
    if model.profiles.profiles.is_empty() {
        kids.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(24.0_f32),
                },
                flex_shrink: 0.0,
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text(t("media-prof-none"), 12.0, Color::from_rgba8(150, 162, 182, 255)),
        );
    }
    let active_name = model.profiles.active.clone();
    for p in &model.profiles.profiles {
        let is_active = active_name.as_deref() == Some(p.name.as_str());
        let label = if p.is_locked() {
            format!("{} · {}", p.name, t("media-prof-locked"))
        } else {
            p.name.clone()
        };
        let bg = if is_active {
            Color::from_rgba8(48, 86, 120, 255)
        } else {
            Color::from_rgba8(34, 40, 52, 255)
        };
        kids.push(prow(vec![
            pbtn(
                label,
                bg,
                Color::from_rgba8(224, 233, 245, 255),
                Msg::ProfileSelect(p.name.clone()),
            ),
            psquare("✕", Color::from_rgba8(74, 58, 64, 255), Msg::ProfileDelete(p.name.clone())),
        ]));
    }

    // Sección del perfil activo: candado + sus playlists.
    if let Some(ap) = model.profiles.active_profile() {
        let candado = if ap.is_locked() {
            pbtn(
                t("media-prof-clear-pass"),
                Color::from_rgba8(70, 58, 64, 255),
                Color::from_rgba8(235, 220, 226, 255),
                Msg::ProfileClearPass,
            )
        } else {
            pbtn(
                t("media-prof-set-pass"),
                Color::from_rgba8(48, 54, 66, 255),
                Color::from_rgba8(220, 230, 245, 255),
                Msg::ProfileFocus(InputTarget::SetPass),
            )
        };
        kids.push(prow(vec![candado]));

        kids.push(psection(format!("{} · {}", ap.name, t("media-prof-playlists"))));
        if ap.playlists.is_empty() {
            kids.push(
                View::new(Style {
                    size: Size {
                        width: percent(1.0_f32),
                        height: length(22.0_f32),
                    },
                    flex_shrink: 0.0,
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                })
                .text(t("media-prof-no-playlists"), 12.0, Color::from_rgba8(150, 162, 182, 255)),
            );
        }
        for (i, pl) in ap.playlists.iter().enumerate() {
            kids.push(prow(vec![
                pbtn(
                    format!("▶  {}  ({})", pl.name, pl.len()),
                    Color::from_rgba8(34, 44, 40, 255),
                    Color::from_rgba8(220, 235, 226, 255),
                    Msg::PlaylistLoad(i),
                ),
                psquare("✕", Color::from_rgba8(74, 58, 64, 255), Msg::PlaylistDelete(i)),
            ]));
        }
        kids.push(prow(vec![pbtn(
            t("media-prof-add-dir"),
            Color::from_rgba8(48, 70, 58, 255),
            Color::from_rgba8(220, 235, 226, 255),
            Msg::ProfileFocus(InputTarget::AddDir),
        )]));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .clip(true)
    .children(kids)
}

/// Panel de ayuda: lista de atajos del keymap activo.
fn help_panel() -> View<Msg> {
    let s = settings();
    let mut rows: Vec<View<Msg>> = Vec::new();
    for b in &s.keymap.bindings {
        rows.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(26.0_f32),
                },
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(10.0_f32),
                    height: length(0.0_f32),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .children(vec![
                View::new(Style {
                    size: Size {
                        width: length(120.0_f32),
                        height: length(24.0_f32),
                    },
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                })
                .fill(Color::from_rgba8(40, 46, 58, 255))
                .radius(6.0)
                .text(b.chord.display(), 12.0, Color::from_rgba8(200, 212, 228, 255)),
                View::new(Style {
                    size: Size {
                        width: auto(),
                        height: length(24.0_f32),
                    },
                    flex_grow: 1.0,
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                })
                .text(
                    b.command.describe(),
                    12.5,
                    Color::from_rgba8(180, 195, 215, 255),
                ),
            ]),
        );
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        flex_grow: 1.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(3.0_f32),
        },
        ..Default::default()
    })
    .clip(true)
    .children(rows)
}
