//! Chrome del piano roll: barra de herramientas (transporte + edición +
//! archivo), rails de **dientes** (`llimphi-widget-dock-rail`) a ambos
//! lados y los paneles acoplables que abre cada diente — mixer de pistas,
//! instrumento, efectos de master, tonalidad/snap y automación.
//!
//! Mismo patrón que cosmos (`cosmos-app-llimphi/src/chrome/dock.rs`): un
//! diente **representa un panel**; clickearlo lo abre al costado del rail.
//! A diferencia de cosmos —cuya rueda deja márgenes y permite que el rail
//! flote como overlay— el piano roll ocupa el área de borde a borde, así
//! que acá el rail va **acoplado** como columna fija (estilo activity bar)
//! para no tapar el teclado. El widget y la composición rail→panel son los
//! canónicos; sólo cambia que el rail es columna en vez de overlay.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, View};
use llimphi_icons::{icon_view, Icon};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_panel::{panel_view, PanelStyle};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_switch::{switch_view, SwitchPalette};
use llimphi_widget_toolbar::{toolbar_view, ToolbarGroup, ToolbarItem, ToolbarPalette};

use takiy_app::{describe_key, gm_program_name, EditMsg, Snap};

use crate::appmodel::Model;
// Los tipos viven en `msg.rs` (autocontenido para el example); acá se
// re-exportan para que el resto del binario los use como `chrome::Dock*`.
pub(crate) use crate::msg::{DockItem, DockSide};
use crate::msg::Msg;

/// Alto de la barra de herramientas (px).
pub(crate) const TOOLBAR_H: f32 = 38.0;
/// Ancho de la tira de dientes de un rail (px).
pub(crate) const RAIL_W: f32 = 46.0;
/// Ancho por default de un panel de sidebar (px).
pub(crate) const DEFAULT_PANEL_W: f32 = 244.0;
/// Límites del ancho de un panel arrastrable.
pub(crate) const PANEL_W_MIN: f32 = 168.0;
pub(crate) const PANEL_W_MAX: f32 = 480.0;

/// Presets de tiempo del delay master (beats). Espejo de los que cicla
/// `cycle_master_delay_time` en `model/apply.rs`.
const DELAY_PRESETS: [f32; 5] = [0.5, 1.0, 1.5, 0.75, 0.25];
const DELAY_LABELS: [&str; 5] = ["1/8", "1/4", "1/4·", "1/8·", "1/16"];
/// Presets de sala del reverb master.
const REVERB_PRESETS: [f32; 3] = [0.25, 0.5, 0.85];
const REVERB_LABELS: [&str; 3] = ["cuarto", "sala", "catedral"];
/// Snaps en el orden que los muestra el segmented del panel de tonalidad.
const SNAPS: [Snap; 6] = [
    Snap::Free,
    Snap::Beat,
    Snap::Half,
    Snap::Quarter,
    Snap::Eighth,
    Snap::Triplet8,
];
const SNAP_LABELS: [&str; 6] = ["free", "1/1", "1/2", "1/4", "1/8", "1/8t"];

// =====================================================================
// Dock: lados y dientes (tipos en `msg.rs`, impls de presentación acá)
// =====================================================================

/// Dientes del rail izquierdo (legacy — el rail izquierdo lo reemplaza
/// ahora `proyecto::rail`; se conserva para no romper `chrome::rail`).
const LEFT_ITEMS: [DockItem; 1] = [DockItem::Pistas];
/// Dientes del rail derecho, en orden. Instrumento se mudó acá al pasar
/// el rail izquierdo a proyectos.
const RIGHT_ITEMS: [DockItem; 4] =
    [DockItem::Instrumento, DockItem::Efectos, DockItem::Tonalidad, DockItem::Automacion];

impl DockItem {
    pub(crate) fn to_u64(self) -> u64 {
        match self {
            DockItem::Pistas => 0,
            DockItem::Instrumento => 1,
            DockItem::Efectos => 2,
            DockItem::Tonalidad => 3,
            DockItem::Automacion => 4,
        }
    }

    pub(crate) fn from_u64(id: u64) -> Option<Self> {
        Some(match id {
            0 => DockItem::Pistas,
            1 => DockItem::Instrumento,
            2 => DockItem::Efectos,
            3 => DockItem::Tonalidad,
            4 => DockItem::Automacion,
            _ => return None,
        })
    }

    fn icon(self) -> Icon {
        match self {
            DockItem::Pistas => Icon::Rows,
            DockItem::Instrumento => Icon::Music,
            DockItem::Efectos => Icon::Equalizer,
            DockItem::Tonalidad => Icon::Grid,
            DockItem::Automacion => Icon::Gauge,
        }
    }

    fn title(self) -> &'static str {
        match self {
            DockItem::Pistas => "Pistas",
            DockItem::Instrumento => "Instrumento",
            DockItem::Efectos => "Efectos",
            DockItem::Tonalidad => "Tonalidad",
            DockItem::Automacion => "Automación",
        }
    }
}

/// Diente activo de un lado (lee el campo correspondiente del modelo).
pub(crate) fn active_of(model: &Model, side: DockSide) -> Option<DockItem> {
    match side {
        DockSide::Left => model.left_active,
        DockSide::Right => model.right_active,
    }
}

// =====================================================================
// Barra de herramientas
// =====================================================================

/// Helper: ítem de toolbar con un icono de `llimphi-icons`.
fn tb(icon: Icon, msg: Msg) -> ToolbarItem<Msg> {
    ToolbarItem::new(move |_s, c| icon_view(icon, c, 2.0), msg)
}

/// Barra de herramientas bajo el menú: transporte, edición, pistas y
/// archivo. Toda acción mapea a un `Msg`/`EditMsg` ya existente.
pub(crate) fn toolbar_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let pal = ToolbarPalette::from_theme(theme);
    let playing = model.playing;
    let metro_on = model.editor.metronome_beats_per_bar.is_some();
    let loop_on = model.editor.loop_region.is_some();
    let snap_lbl = model.editor.snap.label();

    let transport = ToolbarGroup::new(vec![
        tb(if playing { Icon::Stop } else { Icon::Play }, Msg::TogglePlay)
            .with_label(if playing { "stop" } else { "play" })
            .active(playing),
        tb(Icon::SkipForward, Msg::PlayWithCountIn).with_label("count-in"),
        tb(Icon::Bell, Msg::ToggleMetronome).active(metro_on),
        tb(Icon::Repeat, Msg::ToggleLoop).active(loop_on),
        tb(Icon::Grid, Msg::CycleSnap).with_label(snap_lbl),
    ]);

    let edit = ToolbarGroup::new(vec![
        tb(Icon::Rewind, Msg::Undo),
        tb(Icon::FastForward, Msg::Redo),
        tb(Icon::Columns, Msg::Edit(EditMsg::DuplicateSelected)),
        tb(Icon::Trash, Msg::Edit(EditMsg::DeleteSelected)),
    ]);

    let tracks = ToolbarGroup::new(vec![
        tb(Icon::Plus, Msg::Edit(EditMsg::NewTrack)).with_label("pista"),
        tb(Icon::VolumeMute, Msg::Edit(EditMsg::ToggleMuteActive))
            .active(active_track_muted(model)),
        tb(Icon::Volume, Msg::Edit(EditMsg::ToggleSoloActive))
            .active(active_track_soloed(model)),
    ]);

    let file = ToolbarGroup::new(vec![
        tb(Icon::Save, Msg::Save),
        tb(Icon::FileText, Msg::ExportMidi).with_label("mid"),
        tb(Icon::Music, Msg::ExportWav).with_label("wav"),
    ]);

    let mut groups = Vec::new();
    // En el editor de una pista: «‹ pistas» vuelve al panorama y «● grab»
    // entra/sale del modo grabación (resaltado mientras graba).
    if model.screen == crate::appmodel::Screen::Track {
        groups.push(ToolbarGroup::new(vec![
            tb(Icon::ChevronLeft, Msg::OpenOverview).with_label("pistas"),
            tb(Icon::Record, Msg::ToggleRecord)
                .with_label("grab")
                .active(model.recording.is_some()),
        ]));
    }
    groups.extend([transport, edit, tracks, file]);
    toolbar_view(groups, TOOLBAR_H, &pal)
}

fn active_track_muted(model: &Model) -> bool {
    model
        .editor
        .score
        .track(model.editor.active_track)
        .is_some_and(|t| t.mute)
}

fn active_track_soloed(model: &Model) -> bool {
    model
        .editor
        .score
        .track(model.editor.active_track)
        .is_some_and(|t| t.solo)
}

// =====================================================================
// Rail de dientes (columna acoplada)
// =====================================================================

/// Rail de un lado: la tira de dientes del widget envuelta en una columna
/// fija de alto completo con el fondo del rail.
pub(crate) fn rail(side: DockSide, model: &Model, theme: &Theme) -> View<Msg> {
    let items: &[DockItem] = match side {
        DockSide::Left => &LEFT_ITEMS,
        DockSide::Right => &RIGHT_ITEMS,
    };
    let active = active_of(model, side);
    let rail_items: Vec<DockRailItem> = items
        .iter()
        .map(|&it| DockRailItem {
            id: it.to_u64(),
            active: active == Some(it),
        })
        .collect();
    let pal = DockRailPalette::from_theme(theme);
    let strip = dock_rail_view(
        &rail_items,
        RAIL_W,
        &pal,
        |id, _size, color| {
            icon_view(
                DockItem::from_u64(id).map(DockItem::icon).unwrap_or(Icon::More),
                color,
                2.0,
            )
        },
        move |id| Msg::DockActivate(side, DockItem::from_u64(id).unwrap_or(DockItem::Pistas)),
        // Sin drag-entre-rails por ahora: cada lado tiene sus dientes fijos.
        |_id| None,
    );
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(RAIL_W),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(2.0_f32),
            right: length(2.0_f32),
            top: length(4.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![strip])
}

/// Panel de contenido del diente activo de un lado, o `None` si está
/// colapsado. Va en el pane resizable al costado del rail.
pub(crate) fn panel(side: DockSide, model: &Model, theme: &Theme) -> Option<View<Msg>> {
    let item = active_of(model, side)?;
    let rows = match item {
        DockItem::Pistas => mixer_rows(model, theme),
        DockItem::Instrumento => instrumento_rows(model, theme),
        DockItem::Efectos => efectos_rows(model, theme),
        DockItem::Tonalidad => tonalidad_rows(model, theme),
        DockItem::Automacion => automacion_rows(model, theme),
    };
    let mut kids = vec![panel_title(item.title(), theme)];
    kids.extend(rows);
    let column = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(kids);
    Some(panel_view(vec![column], PanelStyle::from_theme(theme)))
}

// =====================================================================
// Helpers de fila
// =====================================================================

fn panel_title(text: &str, theme: &Theme) -> View<Msg> {
    row_h(26.0).text_aligned(text.to_string(), 14.0, theme.fg_text, Alignment::Start)
}

fn section_label(text: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(8.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), 11.0, theme.fg_muted, Alignment::Start)
}

fn line(text: String, theme: &Theme) -> View<Msg> {
    row_h(18.0).text_aligned(text, 11.0, theme.fg_muted, Alignment::Start)
}

/// Fila base de alto fijo, full-width, centrada verticalmente.
fn row_h(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
}

/// Fila horizontal de hijos, alto fijo, con gap.
fn hrow(h: f32, gap: f32, kids: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(h),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(gap),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(kids)
}

/// Botón compacto de ancho fijo (mute/solo, ±).
fn mini_button(label: &str, w: f32, active: bool, theme: &Theme, msg: Msg) -> View<Msg> {
    let mut pal = ButtonPalette::from_theme(theme);
    if active {
        pal.bg = theme.accent;
        pal.bg_hover = theme.accent;
        pal.fg = theme.bg_app;
    }
    View::new(Style {
        size: Size {
            width: length(w),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![button_view(label, &pal, msg)])
}

/// Switch con etiqueta a la izquierda (mismo patrón que cosmos config).
fn switch_row(label: &str, on: bool, msg: Msg, theme: &Theme) -> View<Msg> {
    let pal = SwitchPalette::from_theme(theme);
    let lbl = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(label.to_string(), 12.0, theme.fg_text, Alignment::Start);
    let sw = View::new(Style {
        size: Size {
            width: length(44.0_f32),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![switch_view(if on { 1.0 } else { 0.0 }, msg, &pal)]);
    hrow(28.0, 6.0, vec![lbl, sw])
}

// =====================================================================
// Paneles
// =====================================================================

/// Mixer: una tarjeta por pista (nombre clickable, mute/solo, vol, pan).
fn mixer_rows(model: &Model, theme: &Theme) -> Vec<View<Msg>> {
    let sl_pal = SliderPalette::from_theme(theme);
    let active = model.editor.active_track;
    let mut rows: Vec<View<Msg>> = Vec::new();
    for (i, track) in model.editor.score.tracks().iter().enumerate() {
        let is_active = i == active;
        // Cabecera: nombre (selecciona la pista) + M + S.
        let name_label = if is_active {
            format!("▶ {}", track.name)
        } else {
            format!("  {}", track.name)
        };
        let mut name_pal = ButtonPalette::from_theme(theme);
        if is_active {
            name_pal.bg = theme.bg_selected;
        }
        let name_btn = View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(0.0_f32),
                height: length(26.0_f32),
            },
            ..Default::default()
        })
        .children(vec![button_view(
            name_label,
            &name_pal,
            Msg::Edit(EditMsg::SetActiveTrack { track: i }),
        )]);
        let mute = mini_button(
            "M",
            30.0,
            track.mute,
            theme,
            Msg::Edit(EditMsg::ToggleMuteTrack { track: i }),
        );
        let solo = mini_button(
            "S",
            30.0,
            track.solo,
            theme,
            Msg::Edit(EditMsg::ToggleSoloTrack { track: i }),
        );
        rows.push(hrow(28.0, 4.0, vec![name_btn, mute, solo]));
        // Faders: el slider emite un delta por movimiento (ver
        // `slider_view`), así que lo ruteamos a un Nudge por-índice.
        rows.push(slider_view(
            "vol",
            track.volume,
            0.0,
            1.5,
            &sl_pal,
            move |phase, dv| match phase {
                DragPhase::Move => Some(Msg::Edit(EditMsg::NudgeTrackVolume { track: i, delta: dv })),
                DragPhase::End => None,
            },
        ));
        rows.push(slider_view(
            "pan",
            track.pan,
            -1.0,
            1.0,
            &sl_pal,
            move |phase, dv| match phase {
                DragPhase::Move => Some(Msg::Edit(EditMsg::NudgeTrackPan { track: i, delta: dv })),
                DragPhase::End => None,
            },
        ));
        rows.push(line(String::new(), theme)); // separador visual
    }
    rows.push(mini_full_button("+ pista nueva", theme, Msg::Edit(EditMsg::NewTrack)));
    rows
}

/// Instrumento (programa GM) de la pista activa. Requiere SF2 cargado.
fn instrumento_rows(model: &Model, theme: &Theme) -> Vec<View<Msg>> {
    let active = model.editor.active_track;
    let mut rows: Vec<View<Msg>> = Vec::new();
    let name = model
        .editor
        .score
        .track(active)
        .map(|t| t.name.clone())
        .unwrap_or_default();
    rows.push(line(format!("pista activa: {name}"), theme));
    match model.sf2.as_ref() {
        Some(sf2) => {
            let prog = sf2.program_for_track(active);
            rows.push(section_label("Programa General MIDI", theme));
            rows.push(line(format!("{prog} · {}", gm_program_name(prog)), theme));
            rows.push(hrow(
                30.0,
                6.0,
                vec![
                    mini_button("−", 40.0, false, theme, Msg::NudgeProgram { delta: -1 }),
                    mini_button("+", 40.0, false, theme, Msg::NudgeProgram { delta: 1 }),
                ],
            ));
        }
        None => {
            rows.push(line("sin SF2 cargado — síntesis por osciladores".into(), theme));
            rows.push(line("(TAKIY_SF2=<soundfont> para timbres GM)".into(), theme));
        }
    }
    rows
}

/// Efectos de master: delay + reverb con sus presets.
fn efectos_rows(model: &Model, theme: &Theme) -> Vec<View<Msg>> {
    let seg_pal = SegmentedPalette::from_theme(theme);
    let mut rows: Vec<View<Msg>> = Vec::new();

    rows.push(section_label("Delay", theme));
    let delay = model.editor.score.master_delay.as_ref();
    rows.push(switch_row(
        "Delay master",
        delay.is_some(),
        Msg::Edit(EditMsg::ToggleMasterDelay),
        theme,
    ));
    if let Some(d) = delay {
        let idx = nearest_idx(&DELAY_PRESETS, d.time_beats);
        rows.push(segmented_view(
            &DELAY_LABELS,
            idx,
            |i| Msg::Edit(EditMsg::SetMasterDelayTime { idx: i }),
            &seg_pal,
        ));
    }

    rows.push(section_label("Reverb", theme));
    let reverb = model.editor.score.master_reverb.as_ref();
    rows.push(switch_row(
        "Reverb master",
        reverb.is_some(),
        Msg::Edit(EditMsg::ToggleMasterReverb),
        theme,
    ));
    if let Some(r) = reverb {
        let idx = nearest_idx(&REVERB_PRESETS, r.room_size);
        rows.push(segmented_view(
            &REVERB_LABELS,
            idx,
            |i| Msg::Edit(EditMsg::SetMasterReverbRoom { idx: i }),
            &seg_pal,
        ));
    }
    rows
}

/// Tonalidad (raíz + modo) y granularidad de snap.
fn tonalidad_rows(model: &Model, theme: &Theme) -> Vec<View<Msg>> {
    let seg_pal = SegmentedPalette::from_theme(theme);
    let mut rows: Vec<View<Msg>> = Vec::new();

    rows.push(section_label("Tonalidad", theme));
    rows.push(line(
        format!("actual: {}", describe_key(&model.editor.score.key)),
        theme,
    ));
    rows.push(hrow(
        30.0,
        6.0,
        vec![
            mini_full_button("Raíz", theme, Msg::Edit(EditMsg::CycleKeyRoot)),
            mini_full_button("Modo", theme, Msg::Edit(EditMsg::CycleKeyMode)),
        ],
    ));
    rows.push(switch_row(
        "Snap a la tonalidad",
        model.editor.snap_to_key,
        Msg::Edit(EditMsg::ToggleSnapToKey),
        theme,
    ));

    rows.push(section_label("Snap de edición", theme));
    let idx = SNAPS
        .iter()
        .position(|s| *s == model.editor.snap)
        .unwrap_or(1);
    rows.push(segmented_view(
        &SNAP_LABELS,
        idx,
        |i| Msg::Edit(EditMsg::SetSnap { snap: SNAPS.get(i).copied().unwrap_or(Snap::Beat) }),
        &seg_pal,
    ));
    rows
}

/// Automación de la pista activa: anclar volumen/pan, limpiar.
fn automacion_rows(model: &Model, theme: &Theme) -> Vec<View<Msg>> {
    let active = model.editor.active_track;
    let mut rows: Vec<View<Msg>> = Vec::new();
    let track = model.editor.score.track(active);
    let vol_pts = track
        .and_then(|t| t.volume_automation.as_ref())
        .map(|l| l.len())
        .unwrap_or(0);
    let pan_pts = track
        .and_then(|t| t.pan_automation.as_ref())
        .map(|l| l.len())
        .unwrap_or(0);
    rows.push(line(format!("pista activa #{active}"), theme));
    rows.push(line(format!("vol: {vol_pts} pt · pan: {pan_pts} pt"), theme));
    rows.push(section_label("Anclar punto (al playhead/selección)", theme));
    rows.push(mini_full_button("Anclar volumen", theme, Msg::AnchorVolumeAutomation));
    rows.push(mini_full_button("Anclar pan", theme, Msg::AnchorPanAutomation));
    rows.push(section_label("Limpiar", theme));
    rows.push(mini_full_button(
        "Borrar automación",
        theme,
        Msg::Edit(EditMsg::ClearActiveAutomation),
    ));
    rows.push(line("(arrastrá los dots en el grid para editar)".into(), theme));
    rows
}

/// Botón full-width de alto estándar.
fn mini_full_button(label: &str, theme: &Theme, msg: Msg) -> View<Msg> {
    let pal = ButtonPalette::from_theme(theme);
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: length(30.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![button_view(label, &pal, msg)])
}

/// Índice del preset más cercano a `v` en `presets`.
fn nearest_idx(presets: &[f32], v: f32) -> usize {
    presets
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            (**a - v).abs().partial_cmp(&(**b - v).abs()).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}
