//! `llimphi-module-allichay` — el renderizador único de la configuración
//! declarativa.
//!
//! Toma un [`allichay::Schema`] y lo pinta con el **rail de dientes**
//! (`llimphi-widget-dock-rail`) para las secciones top-level y controles
//! escalares para los campos (toggle, slider, dropdown, color, texto). Los
//! cambios salen como un [`AllichayMsg`] que el host mapea a su propio `Msg`.
//!
//! Es un módulo al estilo de `command-palette`: aporta `state + Msg + apply_key
//! + view`. El host:
//!
//! 1. guarda un [`AllichayState`] en su modelo;
//! 2. llama [`allichay_view`] en su `view`, mapeando [`AllichayMsg`] → su `Msg`;
//! 3. enruta las teclas a [`AllichayState::apply_key`] cuando hay un campo de
//!    texto focado;
//! 4. ante un [`AllichayMsg::Change`], aplica el `(FieldPath, FieldValue)` a su
//!    config (vía [`allichay::Configurable::apply`]) y la persiste con su propio
//!    `save()` — allichay no toca el disco.
//!
//! Subsecciones: en v1 se pintan como grupos con encabezado dentro del panel de
//! la sección activa (un solo nivel de dientes). El panel central combina las
//! apps en un esquema donde cada app es una sección-diente y sus secciones
//! reales bajan a subsecciones — así un mismo rail navega "apps → secciones".

#![forbid(unsafe_code)]

use allichay::{Control, Field, FieldPath, FieldValue, Schema};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, KeyEvent, View};
use llimphi_theme::Theme;

use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_switch::{switch_view, SwitchPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

use std::collections::BTreeMap;

/// Ancho del rail de dientes (px).
const RAIL_W: f32 = 52.0;

// =====================================================================
// Mensajes del módulo
// =====================================================================

/// Lo que emite el renderizador. El host lo envuelve en su propio `Msg`
/// (típicamente `Msg::Allichay(AllichayMsg)`) y lo resuelve en su `update`.
#[derive(Debug, Clone, PartialEq)]
pub enum AllichayMsg {
    /// Se clickeó el diente de la sección `índice`.
    SelectSection(usize),
    /// Se enfocó un campo de texto (para empezar a teclear sobre él).
    Focus(FieldPath),
    /// Un campo cambió de valor. El host lo aplica a su config y persiste.
    Change(FieldPath, FieldValue),
}

// =====================================================================
// Estado del módulo
// =====================================================================

/// El estado que el host guarda en su modelo: qué diente está activo y los
/// buffers de edición de los campos de texto (con su foco).
#[derive(Debug, Clone, Default)]
pub struct AllichayState {
    selected: usize,
    /// Buffers de los campos de texto, por `FieldPath` serializado. Sólo existe
    /// entrada para el campo que se está editando.
    inputs: BTreeMap<String, TextInputState>,
    /// El `FieldPath` (serializado) del campo de texto focado, si hay uno.
    focused: Option<String>,
}

impl AllichayState {
    /// Estado inicial: primera sección, sin foco.
    pub fn new() -> Self {
        Self::default()
    }

    /// Índice de la sección activa.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Selecciona la sección `i`. Limpia el foco de texto (cambiar de diente
    /// cierra cualquier edición en curso).
    pub fn select(&mut self, i: usize) {
        self.selected = i;
        self.blur();
    }

    /// Enfoca un campo de texto, sembrando su buffer con el valor actual `seed`.
    pub fn focus(&mut self, path: &FieldPath, seed: &str) {
        let key = path.to_string();
        let mut st = TextInputState::new();
        st.set_text(seed);
        self.inputs.insert(key.clone(), st);
        self.focused = Some(key);
    }

    /// Quita el foco de texto (sin descartar nada — el valor ya viajó por
    /// `Change` en cada tecla).
    pub fn blur(&mut self) {
        self.focused = None;
        self.inputs.clear();
    }

    /// `true` si hay un campo de texto en edición (para que el host enrute las
    /// teclas a [`AllichayState::apply_key`] sólo cuando hace falta).
    pub fn is_editing(&self) -> bool {
        self.focused.is_some()
    }

    /// `true` si `path` es el campo de texto focado.
    pub fn is_focused(&self, path: &FieldPath) -> bool {
        self.focused.as_deref() == Some(path.to_string().as_str())
    }

    /// Enruta una tecla al campo de texto focado. Devuelve el cambio resultante
    /// (`FieldPath`, [`FieldValue::Text`]) si el contenido cambió, para que el
    /// host lo aplique y persista. `None` si no hay foco o la tecla no editó.
    pub fn apply_key(&mut self, event: &KeyEvent) -> Option<(FieldPath, FieldValue)> {
        let key = self.focused.clone()?;
        let st = self.inputs.get_mut(&key)?;
        if st.apply_key(event) {
            Some((FieldPath::from(key.as_str()), FieldValue::Text(st.text())))
        } else {
            None
        }
    }

    /// Acceso interno al buffer focado para pintarlo.
    fn input(&self, path: &FieldPath) -> Option<&TextInputState> {
        self.inputs.get(&path.to_string())
    }
}

// =====================================================================
// Vista
// =====================================================================

/// Pinta el esquema completo: rail de dientes (secciones) + panel de la sección
/// activa. `on_msg` mapea los [`AllichayMsg`] del módulo al `Msg` del host.
pub fn allichay_view<Msg, F>(
    schema: &Schema,
    state: &AllichayState,
    theme: &Theme,
    on_msg: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let rail = build_rail(schema, state, theme, on_msg.clone());
    let panel = build_panel(schema, state, theme, on_msg);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![rail, panel])
}

fn build_rail<Msg, F>(schema: &Schema, state: &AllichayState, theme: &Theme, on_msg: F) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let sel = state.selected.min(schema.sections.len().saturating_sub(1));
    let items: Vec<DockRailItem> = schema
        .sections
        .iter()
        .enumerate()
        .map(|(i, _)| DockRailItem {
            id: i as u64,
            active: i == sel,
        })
        .collect();

    // Glifos de cada diente (cae a "●" si la sección no declara icono).
    let icons: Vec<String> = schema
        .sections
        .iter()
        .map(|s| {
            if s.icon.is_empty() {
                "●".to_string()
            } else {
                s.icon.clone()
            }
        })
        .collect();

    let activate = on_msg;
    dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| icon_view(icons.get(id as usize).cloned(), size, color),
        move |id| activate(AllichayMsg::SelectSection(id as usize)),
        |_| None,
    )
}

fn icon_view<Msg: Clone + 'static>(glyph: Option<String>, size: f32, color: Color) -> View<Msg> {
    let g = glyph.unwrap_or_else(|| "●".to_string());
    View::new(Style {
        size: Size {
            width: length(size),
            height: length(size),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(g, size * 0.85, color, Alignment::Center)
}

fn build_panel<Msg, F>(schema: &Schema, state: &AllichayState, theme: &Theme, on_msg: F) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    if schema.sections.is_empty() {
        return View::new(panel_style()).text_aligned(
            "Sin configuración".to_string(),
            13.0,
            theme.fg_muted,
            Alignment::Start,
        );
    }
    let sel = state.selected.min(schema.sections.len() - 1);
    let section = &schema.sections[sel];

    let mut children: Vec<View<Msg>> = Vec::new();
    // Encabezado de la sección.
    children.push(section_head(&section.title, &section.help, theme));

    // Ruta base = id de la sección.
    let base = FieldPath::empty().push(section.id.clone());

    // Campos directos.
    for field in &section.fields {
        let path = base.clone().push(field.id.clone());
        children.push(field_row(field, path, state, theme, on_msg.clone()));
    }

    // Subsecciones (un nivel): encabezado + sus campos.
    for sub in &section.subsections {
        children.push(subsection_head(&sub.title, theme));
        let sub_base = base.clone().push(sub.id.clone());
        for field in &sub.fields {
            let path = sub_base.clone().push(field.id.clone());
            children.push(field_row(field, path, state, theme, on_msg.clone()));
        }
    }

    View::new(panel_style()).children(children)
}

fn panel_style() -> Style {
    Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(14.0_f32),
            bottom: length(14.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        ..Default::default()
    }
}

// =====================================================================
// Encabezados y campos
// =====================================================================

fn section_head<Msg: Clone + 'static>(title: &str, help: &str, theme: &Theme) -> View<Msg> {
    let title_v = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(title.to_string(), 16.0, theme.fg_text, Alignment::Start);
    let mut kids = vec![title_v];
    if !help.is_empty() {
        kids.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(16.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(help.to_string(), 11.0, theme.fg_muted, Alignment::Start),
        );
    }
    kids.push(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(1.0_f32),
            },
            margin: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(6.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.border),
    );
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .children(kids)
}

fn subsection_head<Msg: Clone + 'static>(title: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(title.to_string(), 12.5, theme.accent, Alignment::Start)
}

fn label_view<Msg: Clone + 'static>(label: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(label.to_string(), 11.5, theme.fg_muted, Alignment::Start)
}

fn help_view<Msg: Clone + 'static>(help: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(help.to_string(), 10.0, theme.fg_placeholder, Alignment::Start)
}

fn field_col_style() -> Style {
    Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(3.0_f32),
        },
        ..Default::default()
    }
}

/// Una fila de campo: rótulo arriba, control abajo, ayuda opcional.
fn field_row<Msg, F>(
    field: &Field,
    path: FieldPath,
    state: &AllichayState,
    theme: &Theme,
    on_msg: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let control = match &field.control {
        Control::Toggle => toggle_control(field, path, theme, on_msg),
        Control::Slider { min, max, step } => {
            slider_control(field, path, *min, *max, *step, theme, on_msg)
        }
        Control::Dropdown { options } => dropdown_control(field, path, options, theme, on_msg),
        Control::ColorPicker => color_control(field, path, theme, on_msg),
        Control::TextInput => text_control(field, path, state, theme, on_msg),
    };

    let mut kids = vec![label_view(&field.label, theme), control];
    if !field.help.is_empty() {
        kids.push(help_view(&field.help, theme));
    }
    View::new(field_col_style()).children(kids)
}

// =====================================================================
// Controles por tipo
// =====================================================================

fn toggle_control<Msg, F>(field: &Field, path: FieldPath, theme: &Theme, on_msg: F) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let cur = field.value.as_bool().unwrap_or(false);
    let progress = if cur { 1.0 } else { 0.0 };
    let msg = on_msg(AllichayMsg::Change(path, FieldValue::Bool(!cur)));
    switch_view(progress, msg, &SwitchPalette::from_theme(theme))
}

fn slider_control<Msg, F>(
    field: &Field,
    path: FieldPath,
    min: f64,
    max: f64,
    step: f64,
    theme: &Theme,
    on_msg: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let is_int = matches!(field.value, FieldValue::Int(_));
    let cur = field.value.as_float().unwrap_or(0.0);
    let palette = SliderPalette::from_theme(theme);
    slider_view(
        String::new(),
        cur as f32,
        min as f32,
        max as f32,
        &palette,
        move |phase, dv| match phase {
            DragPhase::Move => {
                let mut nv = (cur + dv as f64).clamp(min, max);
                if step > 0.0 {
                    nv = (nv / step).round() * step;
                }
                let value = if is_int {
                    FieldValue::Int(nv as i64)
                } else {
                    FieldValue::Float(nv)
                };
                Some(on_msg(AllichayMsg::Change(path.clone(), value)))
            }
            DragPhase::End => None,
        },
    )
}

fn dropdown_control<Msg, F>(
    field: &Field,
    path: FieldPath,
    options: &[allichay::EnumOption],
    theme: &Theme,
    on_msg: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let cur = field.value.as_str().unwrap_or("");
    let labels: Vec<&str> = options.iter().map(|o| o.label.as_str()).collect();
    let selected = options.iter().position(|o| o.id == cur).unwrap_or(0);
    let ids: Vec<String> = options.iter().map(|o| o.id.clone()).collect();
    segmented_view(
        &labels,
        selected,
        move |i| on_msg(AllichayMsg::Change(path.clone(), FieldValue::Enum(ids[i].clone()))),
        &SegmentedPalette::from_theme(theme),
    )
}

fn color_control<Msg, F>(field: &Field, path: FieldPath, theme: &Theme, on_msg: F) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let cur = field.value.as_color().unwrap_or([0, 0, 0, 255]);
    let palette = SliderPalette::from_theme(theme);

    let mut rows: Vec<View<Msg>> = Vec::with_capacity(5);
    rows.push(swatch_view(cur));
    for (ci, name) in [(0usize, "R"), (1, "G"), (2, "B"), (3, "A")] {
        let f = on_msg.clone();
        let p = path.clone();
        rows.push(slider_view(
            name.to_string(),
            cur[ci] as f32,
            0.0,
            255.0,
            &palette,
            move |phase, dv| match phase {
                DragPhase::Move => {
                    let nv = (cur[ci] as f64 + dv as f64).clamp(0.0, 255.0) as u8;
                    let mut c = cur;
                    c[ci] = nv;
                    Some(f(AllichayMsg::Change(p.clone(), FieldValue::Color(c))))
                }
                DragPhase::End => None,
            },
        ));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(rows)
}

fn swatch_view<Msg: Clone + 'static>(rgba: [u8; 4]) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(40.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]))
    .radius(3.0)
}

fn text_control<Msg, F>(
    field: &Field,
    path: FieldPath,
    state: &AllichayState,
    theme: &Theme,
    on_msg: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let cur = field.value.as_str().unwrap_or("");
    let palette = TextInputPalette::from_theme(theme);
    let focus_msg = on_msg(AllichayMsg::Focus(path.clone()));

    if state.is_focused(&path) {
        if let Some(st) = state.input(&path) {
            return text_input_view(st, "", true, &palette, focus_msg);
        }
    }
    let mut tmp = TextInputState::new();
    tmp.set_text(cur);
    text_input_view(&tmp, "", false, &palette, focus_msg)
}
