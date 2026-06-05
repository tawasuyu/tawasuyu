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
    AlignItems, FlexWrap, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, KeyEvent, View};
use llimphi_theme::Theme;

use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_switch::{switch_view, SwitchPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

use std::collections::BTreeMap;

/// Ancho del rail de dientes (px). El diente es la pestañita que sobresale: un
/// icono, no un rótulo (el nombre lo lleva el encabezado del panel desplegado).
const RAIL_W: f32 = 52.0;

/// Cuántas opciones de un [`Control::Dropdown`] caben cómodas como botones
/// segmentados en una fila. Por encima de esto el segmented se amontona y
/// desborda (p. ej. 7 modos de tiling), así que el renderer cae a un
/// **radio-group vertical** (una fila seleccionable por opción).
const SEGMENTED_MAX: usize = 4;

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
    /// Se enfocó la celda `(row, col)` de un agregado (lista/tabla) en `path`.
    /// El host siembra el buffer con [`AllichayState::focus_cell`] pasándole el
    /// [`FieldValue`] actual del campo.
    FocusCell(FieldPath, usize, usize),
    /// Un campo cambió de valor. El host lo aplica a su config y persiste.
    Change(FieldPath, FieldValue),
    /// El panel se desplazó: nuevo offset absoluto (ya clampeado).
    ScrollTo(f32),
}

// =====================================================================
// Estado del módulo
// =====================================================================

/// El estado que el host guarda en su modelo: qué diente está activo y los
/// buffers de edición de los campos de texto (con su foco).
#[derive(Debug, Clone, Default)]
pub struct AllichayState {
    selected: usize,
    /// Buffers de los campos de texto, por clave de foco. Para un campo escalar
    /// la clave es su `FieldPath` serializado; para una celda de agregado es una
    /// clave compuesta (ver [`cell_key`]). Sólo existe entrada para lo que se
    /// está editando.
    inputs: BTreeMap<String, TextInputState>,
    /// La clave del campo/celda de texto focado, si hay uno.
    focused: Option<String>,
    /// Contexto de edición cuando lo focado es una celda de agregado: el campo,
    /// su valor base (la lista/tabla completa, fija durante la edición) y la
    /// coordenada. `None` cuando se edita un campo de texto escalar.
    edit_cell: Option<EditCell>,
    /// Desplazamiento vertical (px) del panel activo, si su contenido excede el
    /// viewport. Se reinicia al cambiar de diente.
    scroll: f32,
}

/// Lo que el estado guarda mientras se teclea una celda de lista/tabla: con esto
/// reconstruye el [`FieldValue`] entero y nuevo en cada tecla (protocolo "valor
/// entero"), sin que el host tenga que conocer las coordenadas.
#[derive(Debug, Clone)]
struct EditCell {
    path: FieldPath,
    base: FieldValue,
    row: usize,
    col: usize,
}

/// La clave de buffer de una celda de agregado. Usa separadores que no aparecen
/// en un `FieldPath` (sus segmentos van con `.`).
fn cell_key(path: &FieldPath, row: usize, col: usize) -> String {
    format!("{path}\u{1}{row}\u{1}{col}")
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

    /// Selecciona la sección `i`. Limpia el foco de texto y el scroll (cambiar
    /// de diente arranca arriba, sin edición en curso).
    pub fn select(&mut self, i: usize) {
        self.selected = i;
        self.scroll = 0.0;
        self.blur();
    }

    /// Offset de scroll actual del panel (px).
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    /// Fija el offset de scroll (el valor ya viene clampeado por el renderer).
    pub fn set_scroll(&mut self, offset: f32) {
        self.scroll = offset;
    }

    /// Enfoca un campo de texto, sembrando su buffer con el valor actual `seed`.
    pub fn focus(&mut self, path: &FieldPath, seed: &str) {
        let key = path.to_string();
        let mut st = TextInputState::new();
        st.set_text(seed);
        self.inputs.insert(key.clone(), st);
        self.focused = Some(key);
        self.edit_cell = None;
    }

    /// Enfoca la celda `(row, col)` de un agregado. El host le pasa el
    /// [`FieldValue`] actual del campo (`value`); el estado lee de ahí el texto
    /// inicial de la celda y guarda el valor como base para reconstruir el
    /// cambio entero en cada tecla. Si la coordenada no corresponde a una celda,
    /// no enfoca nada.
    pub fn focus_cell(&mut self, path: &FieldPath, value: FieldValue, row: usize, col: usize) {
        let Some(seed) = value.cell(row, col).map(str::to_string) else {
            return;
        };
        let key = cell_key(path, row, col);
        let mut st = TextInputState::new();
        st.set_text(&seed);
        self.inputs.insert(key.clone(), st);
        self.focused = Some(key);
        self.edit_cell = Some(EditCell {
            path: path.clone(),
            base: value,
            row,
            col,
        });
    }

    /// Quita el foco de texto (sin descartar nada — el valor ya viajó por
    /// `Change` en cada tecla).
    pub fn blur(&mut self) {
        self.focused = None;
        self.edit_cell = None;
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

    /// `true` si la celda `(row, col)` de `path` es la focada.
    pub fn is_focused_cell(&self, path: &FieldPath, row: usize, col: usize) -> bool {
        self.focused.as_deref() == Some(cell_key(path, row, col).as_str())
    }

    /// Enruta una tecla al campo/celda de texto focado. Devuelve el cambio
    /// resultante (`FieldPath`, [`FieldValue`]) si el contenido cambió, para que
    /// el host lo aplique y persista. Para un campo escalar es un
    /// [`FieldValue::Text`]; para una celda de agregado es el agregado entero
    /// con esa celda reemplazada. `None` si no hay foco o la tecla no editó.
    pub fn apply_key(&mut self, event: &KeyEvent) -> Option<(FieldPath, FieldValue)> {
        let key = self.focused.clone()?;
        let st = self.inputs.get_mut(&key)?;
        if !st.apply_key(event) {
            return None;
        }
        let text = st.text();
        match &self.edit_cell {
            None => Some((FieldPath::from(key.as_str()), FieldValue::Text(text))),
            Some(c) => Some((
                c.path.clone(),
                c.base.clone().with_cell(c.row, c.col, &text),
            )),
        }
    }

    /// Acceso interno al buffer de un campo escalar focado para pintarlo.
    fn input(&self, path: &FieldPath) -> Option<&TextInputState> {
        self.inputs.get(&path.to_string())
    }

    /// Acceso interno al buffer de una celda focada para pintarla.
    fn input_cell(&self, path: &FieldPath, row: usize, col: usize) -> Option<&TextInputState> {
        self.inputs.get(&cell_key(path, row, col))
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

/// Rail de **dientes** (el widget `dock-rail`): una sección = un diente. El
/// diente sobresale del rail, lleva su icono y se arrastra; al activarse muestra
/// su panel (las demás secciones quedan ocultas hasta que se las elige).
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
    let icons: Vec<String> = schema.sections.iter().map(|s| s.icon.clone()).collect();
    let activate = on_msg;
    dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| tooth_icon(icons.get(id as usize).cloned(), size, color),
        move |id| activate(AllichayMsg::SelectSection(id as usize)),
        |_| None,
    )
}

/// Dibuja el icono de un diente (un glifo emoji que la fuente tenga), con el
/// color ya resuelto por el widget según el estado activo/inactivo.
fn tooth_icon<Msg: Clone + 'static>(glyph: Option<String>, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(size),
            height: length(size),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(
        glyph.unwrap_or_else(|| "•".to_string()),
        size * 0.9,
        color,
        Alignment::Center,
    )
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
    // La ruta base = id de la sección; los campos cuelgan de ahí.
    let base = FieldPath::empty().push(section.id.clone());
    section_view(section, &base, state, theme, on_msg)
}

/// Pinta el panel de **una** sección: su encabezado, sus campos y sus
/// subsecciones (un nivel), construyendo cada `FieldPath` a partir de `base`
/// (la ruta que ya incluye el id de la sección — p. ej. `["mirada::teselado"]`
/// cuando el host compone varias apps en un solo rail).
///
/// Lo expone aparte de [`allichay_view`] para que un host con su propio rail
/// (el panel de control central, que mezcla dientes de varias apps) reutilice
/// el renderizado de campos sin el rail interno del módulo.
pub fn section_view<Msg, F>(
    section: &allichay::Section,
    base: &FieldPath,
    state: &AllichayState,
    theme: &Theme,
    on_msg: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    View::new(panel_style()).children(section_items(section, base, state, theme, on_msg))
}

/// Los views de una sección (encabezado + campos + subsecciones), sin el
/// contenedor — para apilar varias secciones en un mismo panel ([`schema_panel`]).
fn section_items<Msg, F>(
    section: &allichay::Section,
    base: &FieldPath,
    state: &AllichayState,
    theme: &Theme,
    on_msg: F,
) -> Vec<View<Msg>>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let mut children: Vec<View<Msg>> = Vec::new();
    children.push(section_head(&section.title, &section.help, theme));

    for field in &section.fields {
        let path = base.clone().push(field.id.clone());
        children.push(field_row(field, path, state, theme, on_msg.clone()));
    }

    for sub in &section.subsections {
        children.push(subsection_head(&sub.title, theme));
        let sub_base = base.clone().push(sub.id.clone());
        for field in &sub.fields {
            let path = sub_base.clone().push(field.id.clone());
            children.push(field_row(field, path, state, theme, on_msg.clone()));
        }
    }
    children
}

/// Pinta el panel de un diente: **todas** las secciones de `schema` apiladas
/// (cada una con su encabezado de grupo), con scroll vertical si el contenido
/// excede `viewport_h`. Es el panel de un diente del panel de control — el id
/// de cada sección es el prefijo de ruteo (`"app::seccion"`), así que cada
/// campo emite su `FieldPath` completo.
///
/// `state.scroll()` lleva el offset; el renderer emite [`AllichayMsg::ScrollTo`]
/// con el offset ya clampeado para que el host sólo lo guarde.
pub fn schema_panel<Msg, F>(
    schema: &Schema,
    state: &AllichayState,
    theme: &Theme,
    viewport_h: f32,
    on_msg: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let mut items: Vec<View<Msg>> = Vec::new();
    for section in &schema.sections {
        let base = FieldPath::empty().push(section.id.clone());
        items.extend(section_items(section, &base, state, theme, on_msg.clone()));
    }

    let content = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        ..Default::default()
    })
    .children(items);

    let content_len = estimate_height(schema);
    let offset = state.scroll().min(content_len);
    let on_scroll = on_msg;
    let scroller = scroll_y(
        offset,
        content_len,
        viewport_h,
        content,
        move |delta| {
            on_scroll(AllichayMsg::ScrollTo(clamp_offset(
                offset + delta,
                content_len,
                viewport_h,
            )))
        },
        &ScrollPalette::from_theme(theme),
    );

    View::new(panel_style()).children(vec![scroller])
}

/// Estimación (generosa) del alto del contenido de un schema, para el scroll.
fn estimate_height(schema: &Schema) -> f32 {
    let mut h = 0.0_f32;
    for section in &schema.sections {
        h += 44.0; // encabezado de sección
        for f in &section.fields {
            h += field_height(f) + 13.0; // + label/gap/separación
        }
        for sub in &section.subsections {
            h += 26.0; // encabezado de subsección
            for f in &sub.fields {
                h += field_height(f) + 13.0;
            }
        }
    }
    h + 28.0 // padding del panel
}

/// Alto del control de un campo (px). Para los agregados (lista/tabla) depende
/// del número de filas, por eso toma el [`Field`] entero y no sólo su control.
const AGG_ROW_H: f32 = 30.0; // alto de una fila editable de lista/tabla
const AGG_CHROME_H: f32 = 24.0 + 32.0; // encabezado de tabla + botón "agregar"

fn field_height(field: &Field) -> f32 {
    match &field.control {
        Control::Toggle => 22.0,
        Control::Slider { .. } => 22.0,
        // Segmented (1 fila) para pocas opciones; radio-group (N filas) si son
        // muchas. Ver [`dropdown_control`].
        Control::Dropdown { options } => {
            if options.len() <= SEGMENTED_MAX {
                28.0
            } else {
                options.len() as f32 * 30.0
            }
        }
        Control::TextInput => 34.0,
        // swatch + paleta de chips (2 filas a 22px) + 4 sliders RGBA
        Control::ColorPicker => 16.0 + 54.0 + 4.0 * 24.0,
        Control::List { .. } | Control::Table { .. } => {
            field.value.row_count() as f32 * AGG_ROW_H + AGG_CHROME_H
        }
        Control::Display => 8.0, // fila compacta sin label arriba
    }
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
        Control::List { item_label } => {
            list_control(field, path, item_label, state, theme, on_msg)
        }
        Control::Table { columns } => table_control(field, path, columns, state, theme, on_msg),
        Control::Display => return display_row(field, theme),
    };

    let mut kids = vec![label_view(&field.label, theme), control];
    if !field.help.is_empty() {
        kids.push(help_view(&field.help, theme));
    }
    View::new(field_col_style()).children(kids)
}

/// Fila de sólo lectura: rótulo a la izquierda, valor a la derecha. Para items
/// de información del sistema (no editables).
fn display_row<Msg: Clone + 'static>(field: &Field, theme: &Theme) -> View<Msg> {
    let label = View::new(Style {
        size: Size {
            width: length(150.0_f32),
            height: length(20.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(field.label.clone(), 12.0, theme.fg_muted, Alignment::Start);
    let value = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(20.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        field.value.as_str().unwrap_or("").to_string(),
        12.5,
        theme.fg_text,
        Alignment::Start,
    );
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![label, value])
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
    // Pocas opciones: botones segmentados (compacto, una fila).
    if options.len() <= SEGMENTED_MAX {
        let labels: Vec<&str> = options.iter().map(|o| o.label.as_str()).collect();
        let selected = options.iter().position(|o| o.id == cur).unwrap_or(0);
        let ids: Vec<String> = options.iter().map(|o| o.id.clone()).collect();
        return segmented_view(
            &labels,
            selected,
            move |i| on_msg(AllichayMsg::Change(path.clone(), FieldValue::Enum(ids[i].clone()))),
            &SegmentedPalette::from_theme(theme),
        );
    }
    // Muchas opciones: radio-group vertical. Cada fila emite el mismo
    // `Change` que el segmented — sin overlay, sin estado nuevo, sin tocar el
    // `update` del host. El segmented se amontonaba con >4 (locales, modos…).
    let rows: Vec<View<Msg>> = options
        .iter()
        .map(|o| {
            let selected = o.id == cur;
            let msg = on_msg(AllichayMsg::Change(
                path.clone(),
                FieldValue::Enum(o.id.clone()),
            ));
            radio_row(&o.label, selected, msg, theme)
        })
        .collect();
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

/// Una fila de radio del dropdown largo: punto a la izquierda (relleno si está
/// seleccionada) + rótulo. La fila entera es clickeable y resalta en hover; la
/// seleccionada lleva fondo y rótulo en negrita.
fn radio_row<Msg: Clone + 'static>(
    label: &str,
    selected: bool,
    msg: Msg,
    theme: &Theme,
) -> View<Msg> {
    let dot_inner = if selected {
        vec![View::new(Style {
            size: Size { width: length(8.0_f32), height: length(8.0_f32) },
            ..Default::default()
        })
        .radius(4.0)
        .fill(theme.accent)]
    } else {
        Vec::new()
    };
    let dot = View::new(Style {
        size: Size { width: length(16.0_f32), height: length(16.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .radius(8.0)
    .border(1.5, if selected { theme.accent } else { theme.border })
    .children(dot_inner);

    let mut lbl = View::new(Style {
        size: Size { width: Dimension::auto(), height: length(20.0_f32) },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        label.to_string(),
        12.5,
        if selected { theme.fg_text } else { theme.fg_muted },
        Alignment::Start,
    )
    .ellipsis(1);
    if selected {
        lbl = lbl.bold();
    }

    let mut row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(6.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(msg)
    .children(vec![dot, lbl]);
    if selected {
        row = row.fill(theme.bg_selected);
    }
    row
}

fn color_control<Msg, F>(field: &Field, path: FieldPath, theme: &Theme, on_msg: F) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let cur = field.value.as_color().unwrap_or([0, 0, 0, 255]);
    let palette = SliderPalette::from_theme(theme);

    let mut rows: Vec<View<Msg>> = Vec::with_capacity(6);
    rows.push(swatch_view(cur));
    rows.push(swatch_palette(cur, &path, theme, &on_msg));
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

/// Paleta de colores preestablecidos: grises + una rampa de tonos saturados,
/// los típicos para marcos/acentos. Clic en un chip fija el RGB conservando el
/// alfa actual — el camino rápido frente a los sliders RGBA.
const SWATCHES: &[[u8; 3]] = &[
    [0xEC, 0xEC, 0xEC],
    [0x9E, 0x9E, 0x9E],
    [0x42, 0x42, 0x42],
    [0x5C, 0x8F, 0xEB],
    [0x00, 0xBC, 0xD4],
    [0x4C, 0xAF, 0x50],
    [0xFF, 0xC1, 0x07],
    [0xFF, 0x98, 0x00],
    [0xF4, 0x43, 0x36],
    [0xE9, 0x1E, 0x63],
    [0x9C, 0x27, 0xB0],
    [0x79, 0x55, 0x48],
];

/// La fila (envuelta) de chips de la paleta de swatches.
fn swatch_palette<Msg, F>(cur: [u8; 4], path: &FieldPath, theme: &Theme, on_msg: &F) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let chips: Vec<View<Msg>> = SWATCHES
        .iter()
        .map(|rgb| swatch_chip(*rgb, cur, path, theme, on_msg))
        .collect();
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(5.0_f32),
            height: length(5.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(3.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(chips)
}

/// Un chip de la paleta: cuadrado de color clickeable. Si su RGB coincide con el
/// color actual, lleva borde de acento (el chip activo); si no, borde sutil.
fn swatch_chip<Msg, F>(
    rgb: [u8; 3],
    cur: [u8; 4],
    path: &FieldPath,
    theme: &Theme,
    on_msg: &F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let active = cur[0] == rgb[0] && cur[1] == rgb[1] && cur[2] == rgb[2];
    // Fija el RGB del chip conservando el alfa actual.
    let new_color = [rgb[0], rgb[1], rgb[2], cur[3]];
    View::new(Style {
        size: Size {
            width: length(22.0_f32),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(rgb[0], rgb[1], rgb[2], 255))
    .radius(5.0)
    .border(
        if active { 2.0 } else { 1.0 },
        if active { theme.accent } else { theme.border },
    )
    .on_click(on_msg(AllichayMsg::Change(
        path.clone(),
        FieldValue::Color(new_color),
    )))
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

// =====================================================================
// Agregados: lista y tabla
// =====================================================================

/// Pinta una **lista** editable: una fila de texto por item (+ quitar) y un
/// botón "agregar" al pie. Cada edición/alta/baja emite el [`FieldValue::List`]
/// entero y nuevo (protocolo "valor entero"), construido a partir del valor
/// actual del campo.
fn list_control<Msg, F>(
    field: &Field,
    path: FieldPath,
    item_label: &str,
    state: &AllichayState,
    theme: &Theme,
    on_msg: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let value = &field.value;
    let rows_n = value.row_count();
    let palette = TextInputPalette::from_theme(theme);

    let mut rows: Vec<View<Msg>> = Vec::with_capacity(rows_n + 1);
    for r in 0..rows_n {
        let cell = cell_input(value, &path, r, 0, &palette, state, &on_msg);
        let remove = remove_button(value.with_row_removed(r), path.clone(), theme, &on_msg);
        rows.push(agg_row(vec![flex_cell(cell), remove]));
    }
    let add_label = if item_label.is_empty() {
        "Agregar".to_string()
    } else {
        format!("+ {item_label}")
    };
    rows.push(add_button(
        add_label,
        value.with_row_pushed(1),
        path,
        theme,
        on_msg,
    ));
    agg_column(rows)
}

/// Pinta una **tabla** editable: una fila de encabezados (las columnas) + una
/// fila de celdas-texto por registro (+ quitar) + botón "agregar fila". Igual
/// que la lista, cada cambio emite el [`FieldValue::Table`] entero.
fn table_control<Msg, F>(
    field: &Field,
    path: FieldPath,
    columns: &[allichay::Column],
    state: &AllichayState,
    theme: &Theme,
    on_msg: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let value = &field.value;
    let rows_n = value.row_count();
    let ncols = columns.len();
    let palette = TextInputPalette::from_theme(theme);

    let mut rows: Vec<View<Msg>> = Vec::with_capacity(rows_n + 2);

    // Encabezados: una etiqueta por columna + hueco de la columna de "quitar".
    let mut head: Vec<View<Msg>> = columns
        .iter()
        .map(|c| flex_cell(column_head(&c.label, theme)))
        .collect();
    head.push(remove_spacer());
    rows.push(agg_row(head));

    for r in 0..rows_n {
        let mut cells: Vec<View<Msg>> = Vec::with_capacity(ncols + 1);
        for c in 0..ncols {
            cells.push(flex_cell(cell_input(value, &path, r, c, &palette, state, &on_msg)));
        }
        cells.push(remove_button(
            value.with_row_removed(r),
            path.clone(),
            theme,
            &on_msg,
        ));
        rows.push(agg_row(cells));
    }
    rows.push(add_button(
        "+ fila".to_string(),
        value.with_row_pushed(ncols),
        path,
        theme,
        on_msg,
    ));
    agg_column(rows)
}

/// Un campo de texto de celda: igual que [`text_control`] pero su foco es por
/// coordenada (`FocusCell`), no por `FieldPath`.
fn cell_input<Msg, F>(
    value: &FieldValue,
    path: &FieldPath,
    row: usize,
    col: usize,
    palette: &TextInputPalette,
    state: &AllichayState,
    on_msg: &F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    let focus_msg = on_msg(AllichayMsg::FocusCell(path.clone(), row, col));
    if state.is_focused_cell(path, row, col) {
        if let Some(st) = state.input_cell(path, row, col) {
            return text_input_view(st, "", true, palette, focus_msg);
        }
    }
    let mut tmp = TextInputState::new();
    tmp.set_text(value.cell(row, col).unwrap_or(""));
    text_input_view(&tmp, "", false, palette, focus_msg)
}

/// Envuelve una celda en un contenedor que reparte el ancho en partes iguales
/// entre columnas (`flex_grow:1, flex_basis:0`).
fn flex_cell<Msg: Clone + 'static>(child: View<Msg>) -> View<Msg> {
    View::new(Style {
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        size: Size {
            width: Dimension::auto(),
            height: Dimension::auto(),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .children(vec![child])
}

/// Una fila horizontal de un agregado (celdas + botón quitar).
fn agg_row<Msg: Clone + 'static>(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

/// El contenedor vertical del agregado (encabezado/filas/botón agregar).
fn agg_column<Msg: Clone + 'static>(rows: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(rows)
}

/// El encabezado de una columna de tabla.
fn column_head<Msg: Clone + 'static>(label: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(label.to_string(), 10.5, theme.fg_placeholder, Alignment::Start)
}

/// El botón cuadrado de "quitar fila" (✕).
fn remove_button<Msg, F>(
    new_value: FieldValue,
    path: FieldPath,
    theme: &Theme,
    on_msg: &F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    View::new(Style {
        size: Size {
            width: length(26.0_f32),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .radius(5.0)
    .hover_fill(theme.bg_row_hover)
    .on_click(on_msg(AllichayMsg::Change(path, new_value)))
    // `×` (U+00D7) en vez de `✕` (U+2715): la fuente del SO sí trae el de
    // Latin-1. Ver la regla "usar glifos que la fuente tenga".
    .text_aligned("×".to_string(), 15.0, theme.fg_muted, Alignment::Center)
}

/// Un hueco del ancho del botón quitar, para alinear el encabezado de tabla.
fn remove_spacer<Msg: Clone + 'static>() -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(26.0_f32),
            height: length(16.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
}

/// El botón "agregar" al pie de un agregado.
fn add_button<Msg, F>(
    label: String,
    new_value: FieldValue,
    path: FieldPath,
    theme: &Theme,
    on_msg: F,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(AllichayMsg) -> Msg + Clone + Send + Sync + 'static,
{
    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(26.0_f32),
        },
        align_self: Some(AlignItems::Start),
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(2.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(6.0)
    .border(1.0, theme.border)
    .hover_fill(theme.bg_row_hover)
    .on_click(on_msg(AllichayMsg::Change(path, new_value)))
    .text_aligned(label, 11.5, theme.accent, Alignment::Center)
}
