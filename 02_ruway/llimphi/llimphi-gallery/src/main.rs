//! `llimphi-gallery` — demo único del kit transversal de elegancia.
//!
//! Una sola ventana que muestra cómo se ven los widgets del kit
//! juntos sobre el theme dark. Útil para verificar paleta, escala,
//! cinética y consistencia visual de un vistazo.
//!
//! `cargo run -p llimphi-gallery --release`
//!
//! Controles:
//! - Click en switches/segments/breadcrumb: dispatchea Msg
//! - Click en "Mostrar toast": apila un toast en bottom-right
//! - Click en "Abrir modal": muestra el modal
//! - `?`: abre/cierra el overlay de atajos
//! - Esc: cierra overlay activo

use std::time::{Duration, Instant};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};

use llimphi_icons::{icon_view, Icon};
use llimphi_theme::Theme;

use llimphi_widget_avatar::avatar_view;
use llimphi_widget_badge::{count_badge_view, dot_badge_view, BadgeKind};
use llimphi_widget_breadcrumb::{breadcrumb_view, BreadcrumbPalette};
use llimphi_widget_empty::{empty_view, EmptyPalette};
use llimphi_widget_field::{field_view, FieldPalette, FieldSpec};
use llimphi_widget_modal::{modal_view, ModalButton, ModalPalette, ModalSpec};
use llimphi_widget_panel::{panel_signature_painter, PanelStyle};
use llimphi_widget_progress::{linear_progress_view, radial_progress_view};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};
use llimphi_widget_shortcuts_help::{
    shortcuts_help_view, ShortcutEntry, ShortcutGroup, ShortcutsHelpPalette, ShortcutsHelpSpec,
};
use llimphi_widget_skeleton::{skeleton_box_view, skeleton_line_view, SkeletonPalette};
use llimphi_widget_spinner::spinner_view;
use llimphi_widget_splash::splash_view;
use llimphi_widget_status_bar::{status_bar_view, StatusBarPalette, StatusSegment};
use llimphi_widget_switch::{switch_view, SwitchPalette};
use llimphi_widget_toast::{toast_stack_view, Toast};
use llimphi_widget_tooltip::{tooltip_view, Side, TooltipPalette, TooltipSpec};
use llimphi_widget_wawa_mark::{wawa_mark_view, WawaMarkPalette};

#[derive(Clone)]
enum Msg {
    /// Tick para forzar repaint (animaciones por reloj absoluto).
    Tick,
    ToggleA,
    ToggleB,
    SelectSeg(usize),
    #[allow(dead_code)]
    BreadcrumbJump(usize),
    PushToast,
    DismissToast(u64),
    OpenModal,
    CloseModal,
    ConfirmModal,
    ToggleShortcuts,
}

struct Model {
    started_at: Instant,
    switch_a: bool,
    switch_b: bool,
    seg: usize,
    toasts: Vec<Toast>,
    next_toast_id: u64,
    modal_open: bool,
    shortcuts_open: bool,
    viewport: (f32, f32),
}

struct Gallery;

impl App for Gallery {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · gallery"
    }

    fn initial_size() -> (u32, u32) {
        (1280, 800)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        // Loop infinito de ticks para animar spinner/skeleton/splash.
        // En una app real esto se gateaba según haya animaciones vivas.
        handle.spawn_periodic(Duration::from_millis(50), || Msg::Tick);
        Model {
            started_at: Instant::now(),
            switch_a: true,
            switch_b: false,
            seg: 1,
            toasts: Vec::new(),
            next_toast_id: 0,
            modal_open: false,
            shortcuts_open: false,
            viewport: (1280.0, 800.0),
        }
    }

    fn update(model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        // Filtrar toasts expirados oportunamente.
        let now = Instant::now();
        m.toasts.retain(|t| t.is_alive(now));
        match msg {
            Msg::Tick => {}
            Msg::ToggleA => m.switch_a = !m.switch_a,
            Msg::ToggleB => m.switch_b = !m.switch_b,
            Msg::SelectSeg(i) => m.seg = i,
            Msg::BreadcrumbJump(_) => {} // sólo demo
            Msg::PushToast => {
                let kinds = [
                    (BadgeKind::Info, "guardado en disco"),
                    (BadgeKind::Success, "publicado correctamente"),
                    (BadgeKind::Warning, "espacio bajo en cache"),
                    (BadgeKind::Error, "no se pudo conectar"),
                ];
                let (kind, text) = kinds[(m.next_toast_id as usize) % kinds.len()];
                let id = m.next_toast_id;
                m.next_toast_id += 1;
                let toast = match kind {
                    BadgeKind::Info => Toast::info(id, text, Duration::from_secs(4)),
                    BadgeKind::Success => Toast::success(id, text, Duration::from_secs(4)),
                    BadgeKind::Warning => Toast::warning(id, text, Duration::from_secs(4)),
                    BadgeKind::Error => Toast::error(id, text, Duration::from_secs(4)),
                    BadgeKind::Neutral => Toast::info(id, text, Duration::from_secs(4)),
                };
                m.toasts.push(toast);
            }
            Msg::DismissToast(id) => m.toasts.retain(|t| t.id != id),
            Msg::OpenModal => m.modal_open = true,
            Msg::CloseModal => m.modal_open = false,
            Msg::ConfirmModal => m.modal_open = false,
            Msg::ToggleShortcuts => m.shortcuts_open = !m.shortcuts_open,
        }
        m
    }

    fn on_key(_model: &Self::Model, ev: &KeyEvent) -> Option<Self::Msg> {
        if ev.state != KeyState::Pressed {
            return None;
        }
        match &ev.key {
            Key::Named(NamedKey::Escape) => Some(Msg::CloseModal),
            Key::Character(s) if s == "?" => Some(Msg::ToggleShortcuts),
            _ => None,
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = Theme::dark();

        // Tres columnas equilibradas + status bar inferior.
        let left = column_left(model, &theme);
        let center = column_center(model, &theme);
        let right = column_right(model, &theme);

        let cols = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            gap: Size {
                width: length(16.0_f32),
                height: length(0.0_f32),
            },
            padding: Rect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(16.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        })
        .children(vec![left, center, right]);

        let status = status_bar_view(
            vec![
                StatusSegment::text("llimphi-gallery").with_icon(Icon::Home),
                StatusSegment::text(if model.switch_a { "modo: pleno" } else { "modo: simple" })
                    .emphasized(),
            ],
            vec![],
            vec![
                StatusSegment::text("Ln 1, Col 1"),
                StatusSegment::text("UTF-8"),
                StatusSegment::text("?  atajos")
                    .clickable(Msg::ToggleShortcuts)
                    .with_icon(Icon::Info),
            ],
            &StatusBarPalette::from_theme(&theme),
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![cols, status])
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        let theme = Theme::dark();
        // Prioridad: modal > shortcuts > toasts.
        if model.modal_open {
            return Some(modal_view(ModalSpec {
                title: "Confirmar acción".to_string(),
                body: modal_body_view(&theme),
                buttons: vec![
                    ModalButton::cancel("Cancelar", Msg::CloseModal),
                    ModalButton::primary("Aplicar", Msg::ConfirmModal),
                ],
                size: (440.0, 220.0),
                viewport: model.viewport,
                on_dismiss: Msg::CloseModal,
                palette: ModalPalette::from_theme(&theme),
            }));
        }
        if model.shortcuts_open {
            return Some(shortcuts_help_view(ShortcutsHelpSpec {
                title: "Atajos de teclado".to_string(),
                groups: vec![
                    ShortcutGroup::new(
                        "General",
                        vec![
                            ShortcutEntry::new("?", "Mostrar/ocultar esta ayuda"),
                            ShortcutEntry::new("Esc", "Cerrar overlay activo"),
                        ],
                    ),
                    ShortcutGroup::new(
                        "Demo",
                        vec![
                            ShortcutEntry::new("Click", "Toasts, modal y switches"),
                            ShortcutEntry::new("Hover", "Tooltips sobre los avatares"),
                        ],
                    ),
                ],
                viewport: model.viewport,
                on_dismiss: Msg::ToggleShortcuts,
                palette: ShortcutsHelpPalette::from_theme(&theme),
            }));
        }
        if !model.toasts.is_empty() {
            return Some(toast_stack_view(
                &model.toasts,
                model.viewport,
                Msg::DismissToast,
            ));
        }
        None
    }
}

// ---------------------------------------------------------------------
// Columnas
// ---------------------------------------------------------------------

fn column_left(model: &Model, theme: &Theme) -> View<Msg> {
    let mut children: Vec<View<Msg>> = Vec::new();

    children.push(section_title("Identidad"));
    // Sello wawa en chico + grande.
    children.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(128.0_f32),
            },
            gap: Size {
                width: length(16.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![
            wawa_frame(48.0),
            wawa_frame(96.0),
            wawa_frame(128.0),
        ]),
    );

    children.push(section_title("Splash"));
    children.push(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(220.0_f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(llimphi_theme::radius::MD)
        .children(vec![splash_view(model.started_at, theme.bg_panel, theme.fg_text)]),
    );

    children.push(section_title("Empty state"));
    children.push(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(200.0_f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(llimphi_theme::radius::MD)
        .children(vec![empty_view(
            Icon::Folder,
            "Sin documentos abiertos",
            Some("Abrí uno con Ctrl+O o creá un nuevo lienzo para empezar."),
            &EmptyPalette::from_theme(theme),
        )]),
    );

    panel_view(children, theme)
}

fn column_center(model: &Model, theme: &Theme) -> View<Msg> {
    let mut children: Vec<View<Msg>> = Vec::new();

    children.push(section_title("Navegación"));
    children.push(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            ..Default::default()
        })
        .children(vec![breadcrumb_view(
            &["home", "docs", "2026", "elegancia.md"],
            Msg::BreadcrumbJump,
            &BreadcrumbPalette::from_theme(theme),
        )]),
    );

    children.push(section_title("Controles"));
    children.push(switch_row("Modo pleno", model.switch_a, Msg::ToggleA, theme));
    children.push(switch_row("Telemetría", model.switch_b, Msg::ToggleB, theme));
    children.push(spacer_v(8.0));
    children.push(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            ..Default::default()
        })
        .children(vec![segmented_view(
            &["lista", "grilla", "kanban"],
            model.seg,
            Msg::SelectSeg,
            &SegmentedPalette::from_theme(theme),
        )]),
    );

    children.push(section_title("Formulario"));
    children.push(field_view(FieldSpec {
        label: "Nombre del lienzo".to_string(),
        control: fake_text_input("introducción a wawa", theme),
        required: true,
        helper: Some("Aparece como título en la pestaña.".to_string()),
        error: None,
        palette: FieldPalette::from_theme(theme),
    }));
    children.push(spacer_v(12.0));
    children.push(field_view(FieldSpec {
        label: "Slug".to_string(),
        control: fake_text_input("intro-wawa-x@123", theme),
        required: false,
        helper: None,
        error: Some("Sólo letras, números y guiones.".to_string()),
        palette: FieldPalette::from_theme(theme),
    }));

    children.push(section_title("Acciones"));
    children.push(button_row(theme));

    panel_view(children, theme)
}

fn column_right(_model: &Model, theme: &Theme) -> View<Msg> {
    let mut children: Vec<View<Msg>> = Vec::new();

    children.push(section_title("Identidades"));
    // Avatares en línea con badge encima.
    children.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(48.0_f32),
            },
            gap: Size {
                width: length(8.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![
            avatar_view("sergio", 40.0),
            avatar_view("calcetín", 40.0),
            avatar_view("amaru", 40.0),
            avatar_view("pacha", 40.0),
            avatar_view("inti", 40.0),
        ]),
    );

    children.push(section_title("Badges"));
    children.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            gap: Size {
                width: length(10.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![
            count_badge_view(3, BadgeKind::Info),
            count_badge_view(12, BadgeKind::Success),
            count_badge_view(99, BadgeKind::Warning),
            count_badge_view(120, BadgeKind::Error),
            dot_badge_view(BadgeKind::Success),
            dot_badge_view(BadgeKind::Warning),
            dot_badge_view(BadgeKind::Error),
        ]),
    );

    children.push(section_title("Carga"));
    children.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(48.0_f32),
            },
            gap: Size {
                width: length(16.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![
            View::new(Style {
                size: Size {
                    width: length(40.0_f32),
                    height: length(40.0_f32),
                },
                ..Default::default()
            })
            .children(vec![spinner_view(theme.accent, 0.12, 1.0)]),
            View::new(Style {
                size: Size {
                    width: length(40.0_f32),
                    height: length(40.0_f32),
                },
                ..Default::default()
            })
            .children(vec![radial_progress_view(
                0.66,
                theme.bg_button,
                theme.accent,
                0.14,
            )]),
            linear_progress_view(0.42, theme.bg_button, theme.accent, 8.0),
        ]),
    );

    children.push(section_title("Skeleton"));
    let palette = SkeletonPalette::from_theme(theme);
    children.push(skeleton_line_view::<Msg>(200.0, &palette));
    children.push(spacer_v(6.0));
    children.push(skeleton_line_view::<Msg>(280.0, &palette));
    children.push(spacer_v(6.0));
    children.push(skeleton_line_view::<Msg>(160.0, &palette));
    children.push(spacer_v(10.0));
    children.push(skeleton_box_view::<Msg>(percent_to_px(0.9, 360.0), 60.0, &palette));

    children.push(section_title("Iconografía"));
    children.push(icon_grid(theme));

    panel_view(children, theme)
}

// ---------------------------------------------------------------------
// Helpers de composición
// ---------------------------------------------------------------------

fn section_title(text: &str) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(
        text.to_uppercase(),
        10.0,
        Color::from_rgba8(140, 160, 200, 255),
        Alignment::Start,
    )
}

fn panel_view(children: Vec<View<Msg>>, theme: &Theme) -> View<Msg> {
    let style = PanelStyle::from_theme(theme);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(14.0_f32),
            bottom: length(14.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        ..Default::default()
    })
    .paint_with(panel_signature_painter(style))
    .radius(style.radius)
    .clip(true)
    .children(children)
}

fn switch_row(label: &str, value: bool, msg: Msg, theme: &Theme) -> View<Msg> {
    let progress = if value { 1.0 } else { 0.0 };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(label.to_string(), 12.0, theme.fg_text, Alignment::Start),
        switch_view(progress, msg, &SwitchPalette::from_theme(theme)),
    ])
}

fn fake_text_input(text: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_input)
    .radius(llimphi_theme::radius::SM)
    .text_aligned(text.to_string(), 12.0, theme.fg_text, Alignment::Start)
}

fn button_row(theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(32.0_f32),
        },
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![
        btn("Mostrar toast", theme.accent, theme.bg_app, Msg::PushToast),
        btn("Abrir modal", theme.bg_button, theme.fg_text, Msg::OpenModal),
        btn("Atajos (?)", theme.bg_button, theme.fg_text, Msg::ToggleShortcuts),
    ])
}

fn btn(label: &str, bg: Color, fg: Color, msg: Msg) -> View<Msg> {
    let w = label.chars().count() as f32 * 7.5 + 24.0;
    View::new(Style {
        size: Size {
            width: length(w),
            height: length(32.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(bg)
    .radius(llimphi_theme::radius::SM)
    .text_aligned(label.to_string(), 12.0, fg, Alignment::Center)
    .on_click(msg)
}

fn icon_grid(theme: &Theme) -> View<Msg> {
    let icons = [
        Icon::File, Icon::Folder, Icon::Save, Icon::Open, Icon::Search,
        Icon::Plus, Icon::Minus, Icon::X, Icon::Check, Icon::Edit,
        Icon::Trash, Icon::Home, Icon::Settings, Icon::Bell, Icon::More,
        Icon::Info, Icon::Warning, Icon::Error, Icon::ChevronUp,
        Icon::ChevronDown, Icon::ChevronLeft, Icon::ChevronRight,
        Icon::FolderOpen,
    ];
    let cells: Vec<View<Msg>> = icons
        .iter()
        .map(|i| {
            View::new(Style {
                size: Size {
                    width: length(28.0_f32),
                    height: length(28.0_f32),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .fill(theme.bg_panel_alt)
            .radius(llimphi_theme::radius::XS)
            .children(vec![icon_view(*i, theme.fg_text, 1.6)])
        })
        .collect();

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(6.0_f32),
            height: length(6.0_f32),
        },
        flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
        ..Default::default()
    })
    .children(cells)
}

fn modal_body_view(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        "Esta acción reescribirá la configuración local. \
         Sólo dura mientras no salgas — al guardar quedará persistida en disco."
            .to_string(),
        12.0,
        theme.fg_muted,
        Alignment::Start,
    )
}

fn wawa_frame(side: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(side),
            height: length(side),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![wawa_mark_view(&WawaMarkPalette::default())])
}

fn spacer_v(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
}

fn percent_to_px(p: f32, base: f32) -> f32 {
    p * base
}

// Tooltip placeholder — la demo no instrumenta hover-to-show porque
// requeriría más Msgs; queda como código de referencia para apps reales.
#[allow(dead_code)]
fn demo_tooltip(viewport: (f32, f32), text: &str, theme: &Theme) -> View<Msg> {
    tooltip_view::<Msg>(TooltipSpec {
        anchor: (viewport.0 * 0.5, viewport.1 * 0.5),
        viewport,
        side: Side::Bottom,
        text: text.to_string(),
        palette: TooltipPalette::from_theme(theme),
    })
}

fn main() {
    llimphi_ui::run::<Gallery>();
}
