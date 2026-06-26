//! # llimphi-anim-studio — el autor de máquinas de animación, con interfaz
//!
//! Editor visual del grafo de estados estilo Rive de [`llimphi_anim`]: los
//! **estados** son nodos en un lienzo (`llimphi-widget-nodegraph`), las
//! **transiciones** son cables que se trazan arrastrando pin→pin, y un panel de
//! **inputs en vivo** (toggles/sliders/triggers) maneja una `Instance` real cuyo
//! estado actual **se ilumina en el grafo** y se pinta en un preview sintético.
//! El documento ([`doc::Doc`]) es la fuente de verdad; el `StateMachine` es su
//! proyección ejecutable, recompilada en cada edición.
//!
//! ```bash
//! cargo run -p llimphi-anim-studio --release   # ventana interactiva
//! ```
//!
//! Persistencia en `anim-studio.ron` (texto editable a mano).

use std::collections::HashMap;
use std::time::Duration;

use llimphi_anim::{Instance, RenderFrame};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, AlignItems, Dimension, FlexDirection, Size, Style,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::vello;
use llimphi_ui::{
    App, DragPhase, Handle, KeyEvent, View,
};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_nodegraph::{
    nodegraph_view_styled, NodeId, NodeSpec, NodegraphMetrics, NodegraphPalette, NodeTint, Wire,
};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

mod doc;
use doc::{CmpOp, CondDef, Doc, InputDef, InputKind, StateDef, TransDef};

/// Dónde se guarda/carga el grafo (relativo al cwd).
const PROJECT_PATH: &str = "anim-studio.ron";
/// Paso de simulación del preview (~30 fps).
const DT: f64 = 1.0 / 30.0;

// =============================================================================
//  Selección
// =============================================================================

/// Qué elemento está bajo edición en el inspector.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sel {
    None,
    State(usize),
    Trans(usize),
}

// =============================================================================
//  Modelo
// =============================================================================

struct Model {
    doc: Doc,
    theme: Theme,
    sel: Sel,

    /// Instancia ejecutable (recompilada del `doc` en cada edición estructural).
    instance: Instance,
    /// Índice del estado actualmente activo (para iluminar el nodo en el grafo).
    current_idx: Option<usize>,
    /// Valores en vivo de los inputs (siembran los controles y se aplican cada tick).
    live_bools: HashMap<String, bool>,
    live_numbers: HashMap<String, f64>,
    /// ¿Corre la simulación del preview?
    playing: bool,

    /// Campo de nombre del estado seleccionado.
    name_input: TextInputState,
    name_focused: bool,
    /// Campo de nombre para crear un input nuevo.
    new_input: TextInputState,
    new_input_focused: bool,

    status: String,
}

impl Model {
    /// Recompila el `doc` a una `Instance` fresca, reaplicando los inputs en vivo.
    fn rebuild(&mut self) {
        self.instance = self.doc.compile().instance();
        self.seed_live_inputs();
        self.apply_live_inputs();
        self.current_idx = self.find_current();
    }

    /// Asegura que cada input declarado tenga una entrada en los mapas en vivo
    /// (con su default), y descarta los que ya no existen.
    fn seed_live_inputs(&mut self) {
        let mut bools = HashMap::new();
        let mut numbers = HashMap::new();
        for i in &self.doc.inputs {
            match i.kind {
                InputKind::Bool => {
                    let v = self.live_bools.get(&i.name).copied().unwrap_or(i.bool_default);
                    bools.insert(i.name.clone(), v);
                }
                InputKind::Number => {
                    let v = self.live_numbers.get(&i.name).copied().unwrap_or(i.num_default);
                    numbers.insert(i.name.clone(), v);
                }
                InputKind::Trigger => {}
            }
        }
        self.live_bools = bools;
        self.live_numbers = numbers;
    }

    fn apply_live_inputs(&mut self) {
        for (k, v) in &self.live_bools {
            self.instance.set_bool(k.clone(), *v);
        }
        for (k, v) in &self.live_numbers {
            self.instance.set_number(k.clone(), *v);
        }
    }

    fn find_current(&self) -> Option<usize> {
        let name = self.instance.current_state();
        self.doc.states.iter().position(|s| s.name == name)
    }

    fn selected_state(&self) -> Option<usize> {
        match self.sel {
            Sel::State(i) if i < self.doc.states.len() => Some(i),
            _ => None,
        }
    }
    fn selected_trans(&self) -> Option<usize> {
        match self.sel {
            Sel::Trans(i) if i < self.doc.transitions.len() => Some(i),
            _ => None,
        }
    }

    /// Sincroniza el campo de nombre con el estado seleccionado.
    fn sync_name_input(&mut self) {
        if let Some(i) = self.selected_state() {
            self.name_input.set_text(self.doc.states[i].name.clone());
        }
    }
}

// =============================================================================
//  Mensajes
// =============================================================================

#[derive(Clone)]
enum Msg {
    Tick,
    // --- grafo ---
    DragNode(NodeId, DragPhase, f32, f32),
    Connect(NodeId, NodeId),
    SelectState(usize),
    SelectTrans(usize),
    // --- CRUD estados ---
    AddState,
    DeleteSelected,
    SetEntry,
    RenameFocus,
    RenameKey(KeyEvent),
    ToggleLoop,
    SetSpeed(f64),
    SetClipLen(f64),
    // --- transiciones ---
    SetTransDur(f64),
    ToggleAnyState,
    AddCondFor(String),
    AddCondClipDone,
    DeleteCond(usize),
    ToggleCondBool(usize),
    CycleCondOp(usize),
    SetCondNum(usize, f64),
    // --- inputs ---
    NewInputFocus,
    NewInputKey(KeyEvent),
    AddInput(InputKind),
    DeleteInput(usize),
    // --- controles en vivo ---
    SetLiveBool(String, bool),
    SetLiveNumber(String, f64),
    FireTrigger(String),
    TogglePlay,
    Restart,
    // --- persistencia ---
    Save,
    Load,
}

// =============================================================================
//  App
// =============================================================================

struct Studio;

impl App for Studio {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi-anim-studio — autor de máquinas de animación"
    }
    fn initial_size() -> (u32, u32) {
        (1240, 760)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(Duration::from_millis(33), || Msg::Tick);
        let doc = Doc::starter();
        let mut model = Model {
            instance: doc.compile().instance(),
            doc,
            theme: Theme::dark(),
            sel: Sel::None,
            current_idx: Some(0),
            live_bools: HashMap::new(),
            live_numbers: HashMap::new(),
            playing: true,
            name_input: TextInputState::new(),
            name_focused: false,
            new_input: TextInputState::new(),
            new_input_focused: false,
            status: "listo — arrastrá pin→pin para conectar; tocá los inputs en vivo".into(),
        };
        model.rebuild();
        model
    }

    fn on_key(model: &Model, ev: &KeyEvent) -> Option<Msg> {
        if model.name_focused {
            return Some(Msg::RenameKey(ev.clone()));
        }
        if model.new_input_focused {
            return Some(Msg::NewInputKey(ev.clone()));
        }
        None
    }

    fn update(mut model: Model, msg: Msg, _handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => {
                if model.playing {
                    model.apply_live_inputs();
                    model.instance.advance(DT);
                    model.current_idx = model.find_current();
                }
            }

            // ---------------- grafo ----------------
            Msg::DragNode(id, phase, dx, dy) => {
                if matches!(phase, DragPhase::Move | DragPhase::End) {
                    if let Some(s) = model.doc.states.get_mut(id as usize) {
                        s.x = (s.x + dx).max(0.0);
                        s.y = (s.y + dy).max(0.0);
                    }
                }
            }
            Msg::Connect(from, to) => {
                let (from, to) = (from as usize, to as usize);
                if from != to && from < model.doc.states.len() && to < model.doc.states.len() {
                    model.doc.transitions.push(TransDef {
                        from: Some(from),
                        to,
                        conditions: Vec::new(),
                        duration_secs: 0.2,
                    });
                    let idx = model.doc.transitions.len() - 1;
                    model.sel = Sel::Trans(idx);
                    model.status =
                        "transición creada — agregale una condición o nunca dispara".into();
                    model.rebuild();
                }
            }
            Msg::SelectState(i) => {
                model.sel = Sel::State(i);
                model.name_focused = false;
                model.sync_name_input();
            }
            Msg::SelectTrans(i) => {
                model.sel = Sel::Trans(i);
                model.name_focused = false;
            }

            // ---------------- CRUD estados ----------------
            Msg::AddState => {
                let n = model.doc.states.len();
                let x = 80.0 + (n as f32 % 4.0) * 180.0;
                let y = 80.0 + (n as f32 / 4.0).floor() * 150.0;
                model
                    .doc
                    .states
                    .push(StateDef::new(format!("estado{n}"), x, y));
                model.sel = Sel::State(n);
                model.sync_name_input();
                model.rebuild();
            }
            Msg::DeleteSelected => match model.sel {
                Sel::State(i) => {
                    remove_state(&mut model.doc, i);
                    model.sel = Sel::None;
                    model.rebuild();
                }
                Sel::Trans(i) => {
                    if i < model.doc.transitions.len() {
                        model.doc.transitions.remove(i);
                    }
                    model.sel = Sel::None;
                    model.rebuild();
                }
                Sel::None => {}
            },
            Msg::SetEntry => {
                if let Some(i) = model.selected_state() {
                    model.doc.entry = i;
                    model.rebuild();
                    model.status = format!("entry = {}", model.doc.states[i].name);
                }
            }
            Msg::RenameFocus => {
                model.name_focused = true;
                model.new_input_focused = false;
            }
            Msg::RenameKey(ev) => {
                if model.name_input.apply_key(&ev) {
                    if let Some(i) = model.selected_state() {
                        let new = model.name_input.text();
                        // Renombrar el estado y propagar a sus referencias no hace
                        // falta: las transiciones referencian por índice, no nombre.
                        model.doc.states[i].name = new;
                        model.rebuild();
                    }
                }
            }
            Msg::ToggleLoop => {
                if let Some(i) = model.selected_state() {
                    model.doc.states[i].looping = !model.doc.states[i].looping;
                    model.rebuild();
                }
            }
            Msg::SetSpeed(v) => {
                if let Some(i) = model.selected_state() {
                    model.doc.states[i].speed = v.clamp(0.0, 4.0);
                    model.rebuild();
                }
            }
            Msg::SetClipLen(v) => {
                if let Some(i) = model.selected_state() {
                    model.doc.states[i].clip_len = v.clamp(0.1, 10.0);
                    model.rebuild();
                }
            }

            // ---------------- transiciones ----------------
            Msg::SetTransDur(v) => {
                if let Some(i) = model.selected_trans() {
                    model.doc.transitions[i].duration_secs = v.clamp(0.0, 2.0);
                    model.rebuild();
                }
            }
            Msg::ToggleAnyState => {
                if let Some(i) = model.selected_trans() {
                    let t = &mut model.doc.transitions[i];
                    t.from = match t.from {
                        Some(_) => None,
                        None => Some(model.doc.entry),
                    };
                    model.rebuild();
                }
            }
            Msg::AddCondFor(name) => {
                if let Some(i) = model.selected_trans() {
                    let kind = model
                        .doc
                        .inputs
                        .iter()
                        .find(|x| x.name == name)
                        .map(|x| x.kind);
                    let cond = match kind {
                        Some(InputKind::Bool) => CondDef::Bool { input: name, value: true },
                        Some(InputKind::Number) => CondDef::Number {
                            input: name,
                            op: CmpOp::Gt,
                            value: 0.0,
                        },
                        Some(InputKind::Trigger) => CondDef::Trigger { input: name },
                        None => return model,
                    };
                    model.doc.transitions[i].conditions.push(cond);
                    model.rebuild();
                }
            }
            Msg::AddCondClipDone => {
                if let Some(i) = model.selected_trans() {
                    model.doc.transitions[i].conditions.push(CondDef::ClipDone);
                    model.rebuild();
                }
            }
            Msg::DeleteCond(ci) => {
                if let Some(i) = model.selected_trans() {
                    if ci < model.doc.transitions[i].conditions.len() {
                        model.doc.transitions[i].conditions.remove(ci);
                        model.rebuild();
                    }
                }
            }
            Msg::ToggleCondBool(ci) => {
                if let Some(i) = model.selected_trans() {
                    if let Some(CondDef::Bool { value, .. }) =
                        model.doc.transitions[i].conditions.get_mut(ci)
                    {
                        *value = !*value;
                        model.rebuild();
                    }
                }
            }
            Msg::CycleCondOp(ci) => {
                if let Some(i) = model.selected_trans() {
                    if let Some(CondDef::Number { op, .. }) =
                        model.doc.transitions[i].conditions.get_mut(ci)
                    {
                        let cur = CmpOp::ALL.iter().position(|o| o == op).unwrap_or(0);
                        *op = CmpOp::ALL[(cur + 1) % CmpOp::ALL.len()];
                        model.rebuild();
                    }
                }
            }
            Msg::SetCondNum(ci, v) => {
                if let Some(i) = model.selected_trans() {
                    if let Some(CondDef::Number { value, .. }) =
                        model.doc.transitions[i].conditions.get_mut(ci)
                    {
                        *value = v;
                        model.rebuild();
                    }
                }
            }

            // ---------------- inputs ----------------
            Msg::NewInputFocus => {
                model.new_input_focused = true;
                model.name_focused = false;
            }
            Msg::NewInputKey(ev) => {
                model.new_input.apply_key(&ev);
            }
            Msg::AddInput(kind) => {
                let name = model.new_input.text();
                let name = name.trim();
                if !name.is_empty() && !model.doc.inputs.iter().any(|i| i.name == name) {
                    model.doc.inputs.push(InputDef::new(name, kind));
                    model.new_input.clear();
                    model.rebuild();
                } else {
                    model.status = "nombre de input vacío o repetido".into();
                }
            }
            Msg::DeleteInput(i) => {
                if i < model.doc.inputs.len() {
                    model.doc.inputs.remove(i);
                    model.rebuild();
                }
            }

            // ---------------- controles en vivo ----------------
            Msg::SetLiveBool(name, v) => {
                model.live_bools.insert(name, v);
                model.apply_live_inputs();
            }
            Msg::SetLiveNumber(name, v) => {
                model.live_numbers.insert(name, v);
                model.apply_live_inputs();
            }
            Msg::FireTrigger(name) => {
                model.instance.fire(name);
            }
            Msg::TogglePlay => model.playing = !model.playing,
            Msg::Restart => {
                model.rebuild();
                model.status = "reiniciado al estado de entrada".into();
            }

            // ---------------- persistencia ----------------
            Msg::Save => {
                model.status = match model.doc.to_ron() {
                    Ok(s) => match std::fs::write(PROJECT_PATH, s) {
                        Ok(_) => format!("guardado en {PROJECT_PATH}"),
                        Err(e) => format!("error al escribir: {e}"),
                    },
                    Err(e) => format!("error al serializar: {e}"),
                };
            }
            Msg::Load => match std::fs::read_to_string(PROJECT_PATH) {
                Ok(s) => match Doc::from_ron(&s) {
                    Ok(d) => {
                        model.doc = d;
                        model.sel = Sel::None;
                        model.rebuild();
                        model.status = format!("cargado de {PROJECT_PATH}");
                    }
                    Err(e) => model.status = format!("RON inválido: {e}"),
                },
                Err(e) => model.status = format!("no se pudo leer: {e}"),
            },
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0),
                height: percent(1.0),
            },
            ..Default::default()
        })
        .fill(model.theme.bg_app)
        .children(vec![left_panel(model), graph_panel(model), right_panel(model)])
    }
}

// =============================================================================
//  Panel izquierdo — listas + CRUD
// =============================================================================

fn left_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let btn = ButtonPalette::from_theme(theme);
    let mut rows: Vec<View<Msg>> = Vec::new();

    rows.push(section_title("ESTADOS", theme));
    for (i, s) in model.doc.states.iter().enumerate() {
        let is_sel = model.sel == Sel::State(i);
        let is_entry = model.doc.entry == i;
        let label = if is_entry {
            format!("▶ {}", s.name)
        } else {
            s.name.clone()
        };
        rows.push(selectable_row(&label, is_sel, Msg::SelectState(i), theme));
    }
    rows.push(spacer(6.0));
    rows.push(button_view("+ estado", &btn, Msg::AddState));

    rows.push(spacer(14.0));
    rows.push(section_title("TRANSICIONES", theme));
    for (i, t) in model.doc.transitions.iter().enumerate() {
        let is_sel = model.sel == Sel::Trans(i);
        let from = match t.from {
            Some(f) => model.doc.states.get(f).map(|s| s.name.as_str()).unwrap_or("?"),
            None => "∗",
        };
        let to = model
            .doc
            .states
            .get(t.to)
            .map(|s| s.name.as_str())
            .unwrap_or("?");
        let mark = if t.conditions.is_empty() { " ⚠" } else { "" };
        let label = format!("{from} → {to}{mark}");
        rows.push(selectable_row(&label, is_sel, Msg::SelectTrans(i), theme));
    }

    rows.push(spacer(14.0));
    rows.push(section_title("INPUTS", theme));
    for (i, inp) in model.doc.inputs.iter().enumerate() {
        rows.push(input_row(i, inp, theme));
    }
    rows.push(spacer(6.0));
    rows.push(text_input_view(
        &model.new_input,
        "nombre del input…",
        model.new_input_focused,
        &TextInputPalette::from_theme(theme),
        Msg::NewInputFocus,
    ));
    rows.push(spacer(4.0));
    rows.push(
        row(vec![
            button_view("+bool", &btn, Msg::AddInput(InputKind::Bool)),
            button_view("+núm", &btn, Msg::AddInput(InputKind::Number)),
            button_view("+trig", &btn, Msg::AddInput(InputKind::Trigger)),
        ]),
    );

    panel_column(rows, 250.0, theme.bg_panel)
}

fn input_row(i: usize, inp: &InputDef, theme: &Theme) -> View<Msg> {
    let btn = ButtonPalette::from_theme(theme);
    row(vec![
        View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: Dimension::auto(),
                height: length(24.0),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(format!("{} · {}", inp.name, inp.kind.label()), 12.0, theme.fg_text),
        View::new(Style {
            size: Size {
                width: length(30.0),
                height: length(24.0),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![button_view("✕", &btn, Msg::DeleteInput(i))]),
    ])
}

// =============================================================================
//  Panel central — el lienzo de nodos
// =============================================================================

fn graph_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let palette = NodegraphPalette::from_theme(theme);
    let metrics = NodegraphMetrics::default();

    let nodes: Vec<NodeSpec> = model
        .doc
        .states
        .iter()
        .enumerate()
        .map(|(i, s)| NodeSpec {
            id: i as NodeId,
            label: s.name.clone(),
            x: s.x,
            y: s.y,
            inputs: vec!["in".into()],
            outputs: vec!["out".into()],
        })
        .collect();

    // Sólo las transiciones con origen concreto se dibujan como cable; las
    // any-state no tienen nodo de origen (viven sólo en la lista).
    let wires: Vec<Wire> = model
        .doc
        .transitions
        .iter()
        .filter_map(|t| {
            t.from.map(|f| Wire {
                from_node: f as NodeId,
                from_output: 0,
                to_node: t.to as NodeId,
                to_input: 0,
            })
        })
        .collect();

    let current = model.current_idx;
    let selected = model.selected_state();
    let accent = theme.accent;
    let sel_bg = theme.bg_selected;

    let tint = move |id: NodeId| -> Option<NodeTint> {
        let i = id as usize;
        if current == Some(i) {
            // Estado activo: título encendido en accent (se ve "prendido" en vivo).
            Some(NodeTint {
                bg_title: Some(accent),
                fg_title: Some(Color::from_rgba8(20, 20, 24, 255)),
                ..Default::default()
            })
        } else if selected == Some(i) {
            Some(NodeTint {
                bg_node: Some(sel_bg),
                ..Default::default()
            })
        } else {
            None
        }
    };

    let graph = nodegraph_view_styled(
        &nodes,
        &wires,
        &palette,
        &metrics,
        |id, phase, dx, dy| Some(Msg::DragNode(id, phase, dx, dy)),
        |from_node, _from_out, to_node, _to_in| Some(Msg::Connect(from_node, to_node)),
        Some(|id: NodeId| Some(Msg::SelectState(id as usize))),
        Some(&tint as &dyn Fn(NodeId) -> Option<NodeTint>),
        None,
    );

    View::new(Style {
        flex_grow: 1.0,
        flex_direction: FlexDirection::Column,
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![graph])
}

// =============================================================================
//  Panel derecho — preview en vivo + inspector
// =============================================================================

fn right_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let btn = ButtonPalette::from_theme(theme);
    let mut rows: Vec<View<Msg>> = Vec::new();

    // --- Preview ---
    rows.push(section_title("PREVIEW", theme));
    rows.push(preview_canvas(model));
    let cur = model
        .current_idx
        .and_then(|i| model.doc.states.get(i))
        .map(|s| s.name.as_str())
        .unwrap_or("—");
    let trans = if model.instance.is_transitioning() {
        "  (mezclando…)"
    } else {
        ""
    };
    rows.push(
        View::new(auto_h(22.0)).text(format!("estado: {cur}{trans}"), 13.0, theme.accent),
    );
    rows.push(spacer(4.0));
    rows.push(row(vec![
        button_view(if model.playing { "⏸ pausa" } else { "▶ play" }, &btn, Msg::TogglePlay),
        button_view("⟲ reiniciar", &btn, Msg::Restart),
    ]));

    // --- Controles en vivo ---
    rows.push(spacer(12.0));
    rows.push(section_title("INPUTS EN VIVO", theme));
    if model.doc.inputs.is_empty() {
        rows.push(muted("declará inputs en el panel izquierdo", theme));
    }
    let sp = SliderPalette::from_theme(theme);
    for inp in &model.doc.inputs {
        match inp.kind {
            InputKind::Bool => {
                let on = model.live_bools.get(&inp.name).copied().unwrap_or(false);
                let name = inp.name.clone();
                let lbl = format!("{}: {}", inp.name, if on { "true" } else { "false" });
                rows.push(button_view(lbl, &btn, Msg::SetLiveBool(name, !on)));
            }
            InputKind::Number => {
                let v = model.live_numbers.get(&inp.name).copied().unwrap_or(0.0) as f32;
                let name = inp.name.clone();
                rows.push(slider_view(
                    inp.name.clone(),
                    v,
                    0.0,
                    10.0,
                    &sp,
                    move |_p, nv| Some(Msg::SetLiveNumber(name.clone(), nv as f64)),
                ));
            }
            InputKind::Trigger => {
                let name = inp.name.clone();
                rows.push(button_view(format!("⚡ {}", inp.name), &btn, Msg::FireTrigger(name)));
            }
        }
        rows.push(spacer(4.0));
    }

    // --- Inspector ---
    rows.push(spacer(10.0));
    rows.extend(inspector(model));

    // --- Persistencia + estado ---
    rows.push(spacer(12.0));
    rows.push(row(vec![
        button_view("guardar", &btn, Msg::Save),
        button_view("cargar", &btn, Msg::Load),
    ]));
    rows.push(spacer(8.0));
    rows.push(View::new(auto_h(0.0)).text(model.status.clone(), 11.0, theme.fg_placeholder).max_lines(3));

    panel_column(rows, 320.0, theme.bg_panel)
}

/// El inspector del elemento seleccionado (estado o transición).
fn inspector(model: &Model) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let btn = ButtonPalette::from_theme(theme);
    let sp = SliderPalette::from_theme(theme);
    let mut rows: Vec<View<Msg>> = Vec::new();

    match model.sel {
        Sel::State(i) if i < model.doc.states.len() => {
            let s = &model.doc.states[i];
            rows.push(section_title("ESTADO", theme));
            rows.push(text_input_view(
                &model.name_input,
                "nombre…",
                model.name_focused,
                &TextInputPalette::from_theme(theme),
                Msg::RenameFocus,
            ));
            rows.push(spacer(6.0));
            rows.push(button_view(
                if s.looping { "loop: sí" } else { "loop: no" },
                &btn,
                Msg::ToggleLoop,
            ));
            rows.push(spacer(4.0));
            let speed = s.speed as f32;
            rows.push(slider_view(
                format!("velocidad {:.2}", s.speed),
                speed,
                0.0,
                4.0,
                &sp,
                move |_p, nv| Some(Msg::SetSpeed(nv as f64)),
            ));
            if !s.looping {
                let len = s.clip_len as f32;
                rows.push(slider_view(
                    format!("duración {:.2}s", s.clip_len),
                    len,
                    0.1,
                    10.0,
                    &sp,
                    move |_p, nv| Some(Msg::SetClipLen(nv as f64)),
                ));
            }
            rows.push(spacer(6.0));
            rows.push(row(vec![
                button_view("entry", &btn, Msg::SetEntry),
                button_view("borrar", &btn, Msg::DeleteSelected),
            ]));
        }
        Sel::Trans(i) if i < model.doc.transitions.len() => {
            let t = &model.doc.transitions[i];
            rows.push(section_title("TRANSICIÓN", theme));
            let from = match t.from {
                Some(f) => model.doc.states.get(f).map(|s| s.name.clone()).unwrap_or_default(),
                None => "∗ (any-state)".into(),
            };
            let to = model.doc.states.get(t.to).map(|s| s.name.clone()).unwrap_or_default();
            rows.push(muted(&format!("{from}  →  {to}"), theme));
            rows.push(spacer(4.0));
            let dur = t.duration_secs as f32;
            rows.push(slider_view(
                format!("blend {:.2}s", t.duration_secs),
                dur,
                0.0,
                2.0,
                &sp,
                move |_p, nv| Some(Msg::SetTransDur(nv as f64)),
            ));
            rows.push(spacer(4.0));
            rows.push(button_view(
                if t.from.is_some() { "→ volver any-state" } else { "← darle origen (entry)" },
                &btn,
                Msg::ToggleAnyState,
            ));

            // Condiciones (AND).
            rows.push(spacer(8.0));
            rows.push(muted("CONDICIONES (AND)", theme));
            if t.conditions.is_empty() {
                rows.push(muted("⚠ sin condición → nunca dispara", theme));
            }
            for (ci, c) in t.conditions.iter().enumerate() {
                rows.push(cond_row(ci, c, theme));
            }
            // Agregar condición: un botón por input + clip-terminó.
            rows.push(spacer(6.0));
            rows.push(muted("agregar:", theme));
            let mut add_btns: Vec<View<Msg>> = Vec::new();
            for inp in &model.doc.inputs {
                let name = inp.name.clone();
                add_btns.push(button_view(format!("+{}", inp.name), &btn, Msg::AddCondFor(name)));
            }
            if !add_btns.is_empty() {
                rows.push(wrap_row(add_btns));
            }
            rows.push(spacer(4.0));
            rows.push(button_view("+ clip terminó", &btn, Msg::AddCondClipDone));
            rows.push(spacer(6.0));
            rows.push(button_view("borrar transición", &btn, Msg::DeleteSelected));
        }
        _ => {
            rows.push(muted("seleccioná un estado o transición", theme));
        }
    }
    rows
}

/// Una fila de condición editable.
fn cond_row(ci: usize, c: &CondDef, theme: &Theme) -> View<Msg> {
    let btn = ButtonPalette::from_theme(theme);
    let sp = SliderPalette::from_theme(theme);
    let mut items: Vec<View<Msg>> = Vec::new();
    match c {
        CondDef::Bool { input, value } => {
            items.push(grow_text(format!("{input} =="), theme));
            items.push(fixed_btn(
                if *value { "true" } else { "false" },
                Msg::ToggleCondBool(ci),
                &btn,
                64.0,
            ));
        }
        CondDef::Number { input, op, value } => {
            items.push(grow_text(format!("{input} {} {value:.1}", op.symbol()), theme));
            items.push(fixed_btn(op.symbol(), Msg::CycleCondOp(ci), &btn, 40.0));
        }
        CondDef::Trigger { input } => {
            items.push(grow_text(format!("⚡ {input}"), theme));
        }
        CondDef::ClipDone => {
            items.push(grow_text("clip terminó".to_string(), theme));
        }
    }
    items.push(fixed_btn("✕", Msg::DeleteCond(ci), &btn, 30.0));

    let mut col = vec![row(items)];
    // Slider para el valor numérico (debajo).
    if let CondDef::Number { value, .. } = c {
        let v = *value as f32;
        col.push(slider_view(
            "valor",
            v,
            0.0,
            10.0,
            &sp,
            move |_p, nv| Some(Msg::SetCondNum(ci, nv as f64)),
        ));
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .children(col)
}

// =============================================================================
//  Preview sintético
// =============================================================================

/// Lienzo que pinta lo que emite la `Instance`: un disco por clip cuyo color =
/// el color del estado y cuyo movimiento (bob + orbe) avanza con `time_secs`.
/// Durante una transición pinta el clip entrante encima con alpha = mezcla — el
/// crossfade del runtime se ve literalmente.
fn preview_canvas(model: &Model) -> View<Msg> {
    let rf: RenderFrame = model.instance.render_frame();
    let n = model.doc.states.len().max(1);
    let colors: Vec<Color> = (0..n).map(state_color).collect();
    let bg = self_color(model.theme.bg_app, model.theme.bg_panel_alt);

    View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(220.0),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(8.0)
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle, Point};
        use llimphi_ui::llimphi_raster::peniko::Fill;

        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.5) as f64;
        let unit = (rect.w.min(rect.h)) as f64;
        let r = unit * 0.20;

        let draw = |scene: &mut vello::Scene, sample_clip: usize, time: f64, alpha: f32| {
            let color = colors
                .get(sample_clip)
                .copied()
                .unwrap_or(Color::from_rgba8(180, 180, 190, 255))
                .multiply_alpha(alpha);
            // Bob vertical con el tiempo del clip.
            let bob = (time * 2.4).sin() * unit * 0.16;
            let center = Point::new(cx, cy + bob);
            scene.fill(Fill::NonZero, Affine::IDENTITY, &color, None, &Circle::new(center, r));
            // Orbe que marca el avance del tiempo (gira con time).
            let a = time * 2.0;
            let orb = Point::new(cx + a.cos() * r * 1.6, cy + bob + a.sin() * r * 1.6);
            let orb_c = Color::from_rgba8(255, 255, 255, 230).multiply_alpha(alpha);
            scene.fill(Fill::NonZero, Affine::IDENTITY, &orb_c, None, &Circle::new(orb, unit * 0.03));
        };

        // Primario a alpha pleno; entrante encima con su mezcla.
        draw(scene, rf.primary.clip as usize, rf.primary.time_secs, 1.0);
        if let Some((incoming, mix)) = rf.blend {
            draw(scene, incoming.clip as usize, incoming.time_secs, mix);
        }
    })
}

/// Color estable por índice de estado (paleta de tintes distinguibles).
fn state_color(i: usize) -> Color {
    const PAL: [(u8, u8, u8); 8] = [
        (94, 168, 255),  // azul
        (120, 210, 140), // verde
        (255, 178, 92),  // naranja
        (220, 120, 220), // magenta
        (240, 220, 110), // amarillo
        (120, 210, 220), // cyan
        (240, 130, 130), // rojo
        (170, 150, 240), // violeta
    ];
    let (r, g, b) = PAL[i % PAL.len()];
    Color::from_rgba8(r, g, b, 255)
}

// =============================================================================
//  Helpers de layout
// =============================================================================

fn auto_h(h: f32) -> Style {
    Style {
        size: Size {
            width: percent(1.0),
            height: if h > 0.0 { length(h) } else { Dimension::auto() },
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    }
}

fn spacer(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(h),
        },
        ..Default::default()
    })
}

fn section_title(text: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(20.0),
        },
        ..Default::default()
    })
    .text(text.to_string(), 11.0, theme.fg_muted)
}

fn muted(text: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .text(text.to_string(), 11.0, theme.fg_placeholder)
    .max_lines(2)
}

fn grow_text(text: String, theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: Dimension::auto(),
            height: length(26.0),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(text, 12.0, theme.fg_text)
}

fn fixed_btn(label: &str, msg: Msg, btn: &ButtonPalette, w: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(w),
            height: length(26.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![button_view(label.to_string(), btn, msg)])
}

fn selectable_row(label: &str, selected: bool, msg: Msg, theme: &Theme) -> View<Msg> {
    let bg = if selected {
        theme.bg_selected
    } else {
        theme.bg_panel_alt
    };
    let fg = if selected { theme.fg_text } else { theme.fg_muted };
    View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(26.0),
        },
        align_items: Some(AlignItems::Center),
        padding: pad(8.0, 0.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(4.0)
    .text(label.to_string(), 12.0, fg)
    .on_click(msg)
}

fn row(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0),
            height: Dimension::auto(),
        },
        gap: gap(6.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(children)
}

/// Fila que envuelve (varios botones chicos).
fn wrap_row(children: Vec<View<Msg>>) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::prelude::FlexWrap;
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        size: Size {
            width: percent(1.0),
            height: Dimension::auto(),
        },
        gap: gap(4.0),
        ..Default::default()
    })
    .children(children)
}

fn panel_column(rows: Vec<View<Msg>>, width: f32, bg: Color) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(width),
            height: percent(1.0),
        },
        flex_shrink: 0.0,
        padding: pad(12.0, 12.0),
        gap: gap(2.0),
        ..Default::default()
    })
    .fill(bg)
    .children(rows)
}

fn pad(
    x: f32,
    y: f32,
) -> llimphi_ui::llimphi_layout::taffy::prelude::Rect<
    llimphi_ui::llimphi_layout::taffy::prelude::LengthPercentage,
> {
    use llimphi_ui::llimphi_layout::taffy::prelude::Rect;
    Rect {
        left: length(x),
        right: length(x),
        top: length(y),
        bottom: length(y),
    }
}

fn gap(
    g: f32,
) -> Size<llimphi_ui::llimphi_layout::taffy::prelude::LengthPercentage> {
    Size {
        width: length(g),
        height: length(g),
    }
}

/// Mezcla simple de dos colores al 50% (para el fondo del preview).
fn self_color(a: Color, b: Color) -> Color {
    let ca = a.to_rgba8();
    let cb = b.to_rgba8();
    Color::from_rgba8(
        ((ca.r as u16 + cb.r as u16) / 2) as u8,
        ((ca.g as u16 + cb.g as u16) / 2) as u8,
        ((ca.b as u16 + cb.b as u16) / 2) as u8,
        255,
    )
}

// =============================================================================
//  Operaciones sobre el documento
// =============================================================================

/// Borra el estado `idx` y reindexa transiciones/entry consistentemente.
fn remove_state(doc: &mut Doc, idx: usize) {
    if idx >= doc.states.len() {
        return;
    }
    doc.states.remove(idx);
    // Descartar transiciones que tocan el estado borrado; reindexar el resto.
    doc.transitions.retain(|t| t.from != Some(idx) && t.to != idx);
    for t in &mut doc.transitions {
        if let Some(f) = t.from {
            if f > idx {
                t.from = Some(f - 1);
            }
        }
        if t.to > idx {
            t.to -= 1;
        }
    }
    if doc.entry == idx {
        doc.entry = 0;
    } else if doc.entry > idx {
        doc.entry -= 1;
    }
}

fn main() {
    llimphi_ui::run::<Studio>();
}
