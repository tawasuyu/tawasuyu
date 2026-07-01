//! # llimphi-voxel-studio — el creador de mundos, con interfaz
//!
//! Editor por **niveles de composición**: Leyes → Materiales → Seres → Biomas →
//! Mundos → Escenas. Cada nivel tiene su lista de items **creables, editables,
//! renombrables, borrables y duplicables**, su editor a la derecha y (donde aplica)
//! preview 3D en vivo. La jerarquía vive en el [`Project`] agnóstico de
//! `llimphi-voxel`; esta app sólo la pinta.
//!
//! ```bash
//! cargo run -p llimphi-voxel-studio --release            # ventana interactiva
//! cargo run -p llimphi-voxel-studio --release -- --shot  # PNG headless a /tmp
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{push_cube, Camera3d};
use llimphi_icons::{icon_view, Icon};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, AlignItems, Dimension, FlexDirection, Position, Size, Style,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{
    App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta,
};
use llimphi_voxel::{
    window_origin_for_cast, world_dim, ActorKeySpec, ActorScript, ActorSpec, Age, BinOp, Bioma,
    BiomaPalette, CharSpec, Clip, FieldDef, FieldEngine, Forma, LeyKind, LeyUso, MatRole, Material,
    MaterialDef, Mundo, MundoRender, ParamDef, Program, Project, Reduce, SceneSpec, ShotKind,
    ShotSpec, UnOp, PREVIEW_DIM_XZ,
};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_dock_rail::{dock_rail_view_side, DockRailItem, DockRailPalette, DockRailSide};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_nodegraph::{nodegraph_view_ex, NodeId, NodegraphMetrics, NodegraphPalette, PinIdx};
use llimphi_image::{from_rgba8, Image};
use graph::{EqGraph, NodeOp};

mod ai;
mod graph;
mod preview;
mod render;
mod shot;
mod soundtrack;
use ai::MatRefs;
use preview::WorldPreview;

/// Dónde se guarda/carga el proyecto (relativo al cwd).
const PROJECT_PATH: &str = "voxel-studio.ron";
/// Paso de tiempo de la reproducción de escenas (~30 fps).
const DT: f32 = 1.0 / 30.0;

/// Lado de la grilla 2D del **laboratorio de leyes** (petri dish del nivel Leyes).
const LAB_DIM: u32 = 64;
/// Sub‑pasos del laboratorio por cuadro (la reacción‑difusión evoluciona lento).
const LAB_SUBSTEPS: usize = 4;

/// **Laboratorio de una ley `Ecuacion`**: un campo escalar 2D corriendo la ecuación
/// autorada, para *ver* qué hace sin depender de materiales ni biomas. El heatmap del
/// campo visible se pinta en el centro del nivel Leyes. Se reconstruye cuando cambia
/// la ley (`ley_id`) o su estructura (`gen`); los parámetros se leen en vivo al pasar.
struct LawLab {
    /// Ley que refleja.
    ley_id: u64,
    /// Generación estructural con la que se construyó (campos/fuentes).
    gen: u64,
    /// Estado de los campos.
    engine: FieldEngine,
    /// Programa compilado (o el error de parseo, para mostrarlo).
    program: Result<Program, String>,
    /// Campo que se visualiza en el heatmap.
    vis: usize,
}

impl LawLab {
    /// Construye el laboratorio para una ley `Ecuacion` y lo siembra.
    fn build(ley_id: u64, gen: u64, campos: &[FieldDef], program: Result<Program, String>, vis: usize) -> Self {
        let mut engine = FieldEngine::new([LAB_DIM, 1, LAB_DIM], campos.to_vec());
        seed_engine(&mut engine);
        let vis = vis.min(campos.len().saturating_sub(1));
        Self { ley_id, gen, engine, program, vis }
    }

    /// Reinicia los campos a su siembra (sin recompilar).
    fn reseed(&mut self) {
        seed_engine(&mut self.engine);
    }

    /// Avanza `LAB_SUBSTEPS` sub‑pasos si el programa compiló.
    fn step(&mut self, params: &[f32]) {
        if let Ok(prog) = &self.program {
            for _ in 0..LAB_SUBSTEPS {
                self.engine.step(prog, params, 1.0);
            }
        }
    }
}

/// (Re)construye el laboratorio si la ley seleccionada es una `Ecuacion` y cambió
/// (por id o por generación estructural). Compila **sólo al reconstruir**, no cada
/// cuadro. Deja `model.lab = None` si el nivel no tiene una ley por ecuación.
fn ensure_lab(model: &mut Model) {
    let id = model.sel[0];
    let is_ec = matches!(
        model.project.ley(id).map(|l| &l.kind),
        Some(LeyKind::Ecuacion { campos, .. }) if !campos.is_empty()
    );
    if !is_ec {
        model.lab = None;
        return;
    }
    let need = match &model.lab {
        Some(lab) => lab.ley_id != id || lab.gen != model.lab_gen,
        None => true,
    };
    if !need {
        return;
    }
    let (campos, program) = match &model.project.ley(id).unwrap().kind {
        k @ LeyKind::Ecuacion { campos, .. } => (campos.clone(), k.compile_ecuacion().unwrap()),
        _ => unreachable!("is_ec ya lo garantiza"),
    };
    let vis = model.lab.as_ref().map(|x| x.vis).unwrap_or(0);
    model.lab = Some(LawLab::build(id, model.lab_gen, &campos, program, vis));
}

/// (Re)construye el grafo de nodos a partir de las fórmulas de la ley seleccionada.
fn rebuild_eq_graph(model: &mut Model) {
    let built = sel_ley(model).and_then(|l| match &l.kind {
        LeyKind::Ecuacion { fuentes, .. } => {
            l.kind.ecuacion_symbols().map(|sym| EqGraph::from_fuentes(fuentes, &sym))
        }
        _ => None,
    });
    model.eq_graph = built;
    model.eq_graph_gen = model.lab_gen;
}

/// Recompila el grafo a fórmulas y las escribe en la ley. Bump de `lab_gen` (para que
/// el laboratorio recompile) marcando el grafo como en‑sync (no se reconstruye solo).
fn sync_graph_to_fuentes(model: &mut Model) {
    let Some(sym) = sel_ley(model).and_then(|l| l.kind.ecuacion_symbols()) else {
        return;
    };
    let n = sym.campos.len();
    let fuentes = match &model.eq_graph {
        Some(g) => g.to_fuentes(n, &sym),
        None => return,
    };
    if let Some(l) = sel_ley_mut(model) {
        if let LeyKind::Ecuacion { fuentes: f, .. } = &mut l.kind {
            *f = fuentes;
        }
    }
    model.lab_gen += 1;
    model.eq_graph_gen = model.lab_gen;
}

/// Siembra un motor de campo: cada campo a su `init` + una mancha central (para
/// disparar difusión/reacción). La mancha = 60 % del rango, en un cuadro central.
fn seed_engine(engine: &mut FieldEngine) {
    let dim = engine.dim();
    let defs: Vec<(f32, f32)> = engine.fields().iter().map(|d| (d.min, d.max)).collect();
    let (cx, cz) = (dim[0] / 2, dim[2] / 2);
    let r = (dim[0] / 8).max(2);
    for (f, (mn, mx)) in defs.iter().enumerate() {
        let blob = mn + 0.6 * (mx - mn);
        for z in cz.saturating_sub(r)..(cz + r).min(dim[2]) {
            for x in cx.saturating_sub(r)..(cx + r).min(dim[0]) {
                engine.set(f as u16, x, 0, z, blob);
            }
        }
    }
}

// =============================================================================
//  Niveles de composición (rail izquierdo)
// =============================================================================

/// Los niveles de composición, de lo más básico a lo más compuesto.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Leyes,
    Materiales,
    Seres,
    Biomas,
    Mundos,
    Escenas,
}

impl Level {
    const ALL: [Level; 6] = [
        Level::Leyes,
        Level::Materiales,
        Level::Seres,
        Level::Biomas,
        Level::Mundos,
        Level::Escenas,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|&l| l == self).unwrap_or(0)
    }
    fn from_index(i: usize) -> Level {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }
    fn label(self) -> &'static str {
        match self {
            Level::Leyes => "Leyes",
            Level::Materiales => "Materiales",
            Level::Seres => "Seres",
            Level::Biomas => "Biomas",
            Level::Mundos => "Mundos",
            Level::Escenas => "Escenas",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Level::Leyes => Icon::Droplet,
            Level::Materiales => Icon::Leaf,
            Level::Seres => Icon::User,
            Level::Biomas => Icon::Mountain,
            Level::Mundos => Icon::Globe,
            Level::Escenas => Icon::Film,
        }
    }
    /// Color de acento del nivel (íconos coloridos).
    fn color(self) -> Color {
        match self {
            Level::Leyes => Color::from_rgba8(150, 130, 230, 255),
            Level::Materiales => Color::from_rgba8(220, 170, 80, 255),
            Level::Seres => Color::from_rgba8(110, 200, 130, 255),
            Level::Biomas => Color::from_rgba8(90, 200, 200, 255),
            Level::Mundos => Color::from_rgba8(90, 150, 230, 255),
            Level::Escenas => Color::from_rgba8(230, 120, 110, 255),
        }
    }
    /// Sets de herramientas (pestañas del rail derecho): rótulo + ícono.
    fn tools(self) -> Vec<(&'static str, Icon)> {
        match self {
            Level::Leyes => vec![("Parámetros", Icon::Gauge)],
            Level::Materiales => vec![("Aspecto", Icon::Image), ("Leyes", Icon::Droplet)],
            Level::Seres => vec![
                ("Cuerpo", Icon::User),
                ("Andar", Icon::Gauge),
                ("Conducta", Icon::Settings),
                ("Piel", Icon::Image),
                ("Camiseta", Icon::Image),
                ("Pantalón", Icon::Image),
            ],
            Level::Biomas => vec![
                ("Relieve", Icon::Mountain),
                ("Materiales", Icon::Leaf),
                ("Objetos", Icon::Grid),
                ("Seres", Icon::User),
            ],
            Level::Mundos => vec![("Semilla", Icon::Globe), ("Biomas", Icon::Mountain)],
            Level::Escenas => vec![
                ("Escena", Icon::Film),
                ("Reparto", Icon::User),
                ("Cámara", Icon::Camera),
                ("Video", Icon::Play),
            ],
        }
    }
}

/// Parte coloreable de un ser.
#[derive(Clone, Copy)]
enum Part {
    Skin,
    Shirt,
    Pants,
}

/// Parámetro `f32` del relieve de un bioma.
#[derive(Clone, Copy)]
enum BiomaField {
    Base,
    Dune,
    Relief,
    Mountains,
    Water,
    Rivers,
    PeakAt,
}

#[derive(Clone)]
enum Msg {
    // Navegación de niveles + items.
    SelectLevel(Level),
    SelectItem(u64),
    SelectTool(usize),
    NewItem,
    DupItem,
    DelItem,
    // Renombrar (campo de texto).
    RenameFocus,
    RenameKey(KeyEvent),
    // Leyes.
    CycleLeyKind,
    SetLeyParam(usize, f32),
    // Leyes por ecuación (autorables).
    LeyPreset,
    SetEcuParam(usize, f32),
    FormulaFocus(usize),
    FormulaKey(KeyEvent),
    AddCampo,
    RemoveCampo,
    AddEcuParam,
    RemoveEcuParam,
    CycleLabField,
    ReseedLab,
    // Vista de grafo de nodos de la ley.
    ToggleLeyesView,
    GraphDrag(NodeId, f32, f32),
    GraphConnect(NodeId, PinIdx, NodeId, PinIdx),
    GraphAddNode(NodeOp),
    GraphDeleteNode(NodeId),
    // Materiales.
    CycleMatRole,
    SetMatColor(usize, f32),
    SetMatGrain(f32),
    CycleMatParent,
    AddMatLey,
    RemoveMatLey,
    // Seres.
    CycleSereAge,
    CycleSereCuerpo,
    SetSereColor(Part, usize, f32),
    // Andares (capa 2) del ser-rig.
    CycleAndarEstado,
    SetAndarCadencia(f32),
    SetAndarAmplitud(usize, f32),
    // Conducta (capa 3).
    SetConducta(usize, f32),
    ToggleManada,
    // Biomas.
    SetBiomaField(BiomaField, f32),
    CycleBiomaGround,
    CycleBiomaCliff,
    CycleBiomaPeak,
    AddBiomaObjeto,
    RemoveBiomaObjeto,
    SetObjetoDensidad(usize, f32),
    AddBiomaSere,
    RemoveBiomaSere,
    CycleBiomaSere(usize),
    SetBiomaSereProb(usize, f32),
    // Mundos.
    SeedFocus,
    SeedKey(KeyEvent),
    SeedRandom,
    CycleMundoBioma,
    // Escenas — reparto y recorrido (filmografía).
    SelectActor(usize),
    AddActor,
    RemoveActor,
    CycleActorSere,
    SelectKey(usize),
    AddKey,
    RemoveKey,
    SetKeyPos(bool, f32), // true=gx, false=gz
    SetKeyTime(f32),
    CycleKeyClip,
    PlaceKeyAt(f32, f32), // click en el terreno → mueve la clave seleccionada
    // Escenas.
    CycleSceneMundo,
    SetSceneDur(f32),
    TogglePlay,
    Scrub(f32),
    ToggleSceneCam,
    AddShot,
    RemoveShot,
    ExportVideo,
    ExportDone(Result<String, String>),
    // Simulación de la ley Fluir (agua).
    ToggleSim,
    // Cámara.
    Orbit(f32, f32),
    Zoom(f32),
    // IA.
    AiFocus,
    AiKey(KeyEvent),
    AiGenerate,
    AiBioma(Bioma),
    AiSere(CharSpec),
    AiScene(SceneSpec),
    // Persistencia.
    Save,
    Load,
    // Reloj.
    Tick,
}

struct Model {
    theme: Theme,
    project: Project,
    /// Nivel activo.
    level: Level,
    /// Item seleccionado (id) por nivel.
    sel: [u64; 6],
    /// Pestaña de herramientas activa (rail derecho).
    tool_tab: usize,
    /// Generación: se incrementa en cada edición para regenerar el preview.
    gen: u64,
    /// Cámara de órbita.
    yaw: f32,
    pitch: f32,
    dist: f32,
    /// Preview perezoso (se construye en la 1ª pintada GPU).
    preview: Arc<Mutex<Option<WorldPreview>>>,
    /// Mensaje de estado.
    status: String,
    /// Campo de renombrado + foco.
    name_input: TextInputState,
    name_focused: bool,
    /// Campo de semilla (mundos) + foco.
    seed_input: TextInputState,
    seed_focused: bool,
    /// Campo de la IA + foco + "generando".
    ai_input: TextInputState,
    ai_focused: bool,
    ai_busy: bool,
    /// Reproducción de escenas.
    time: f32,
    playing: bool,
    script_cam: bool,
    exporting: bool,
    /// Simulación de agua (ley Fluir) corriendo en el preview de mundo/bioma.
    simulating: bool,
    /// Estado del andar que se edita/previsualiza en Seres (0=quieto,1=caminar,2=correr).
    andar_estado: usize,
    /// Manada viva (conducta) corriendo en el preview de Seres.
    manada: bool,
    /// Reparto: actor y clave (waypoint) seleccionados al dirigir una escena.
    actor_sel: usize,
    key_sel: usize,
    /// Semilla del random de mundos (LCG; no hay `Math.random`).
    rng: u32,
    /// Decisión global: dientes DENTRO (overlay) o FUERA (franja reservada).
    dientes_outside: bool,
    /// Laboratorio de la ley `Ecuacion` seleccionada (nivel Leyes). Perezoso.
    lab: Option<LawLab>,
    /// Generación estructural de la ley en edición: bump al cambiar campos/fórmulas
    /// (para reconstruir el laboratorio, sin resetear al tocar sólo parámetros).
    lab_gen: u64,
    /// Edición de una fórmula: buffer compartido + foco + campo objetivo.
    formula_input: TextInputState,
    formula_focused: bool,
    formula_target: usize,
    /// Nivel Leyes: modo de la vista central — `false` laboratorio (heatmap),
    /// `true` grafo de nodos (la segunda superficie de autoría).
    leyes_node_mode: bool,
    /// Grafo de nodos de la ley (derivado de las fórmulas; estado de UI).
    eq_graph: Option<EqGraph>,
    /// `lab_gen` con el que se armó el grafo (para saber si el texto cambió afuera).
    eq_graph_gen: u64,
}

impl Model {
    /// Id del item seleccionado en el nivel activo (0 = ninguno).
    fn selected(&self) -> u64 {
        self.sel[self.level.index()]
    }
    fn set_selected(&mut self, id: u64) {
        let i = self.level.index();
        self.sel[i] = id;
    }

    /// Los items `(id, nombre)` del nivel activo, en orden.
    fn items(&self) -> Vec<(u64, String)> {
        match self.level {
            Level::Leyes => self.project.leyes.iter().map(|x| (x.id, x.name.clone())).collect(),
            Level::Materiales => self.project.materiales.iter().map(|x| (x.id, x.name.clone())).collect(),
            Level::Seres => self.project.seres.iter().map(|x| (x.id, x.name.clone())).collect(),
            Level::Biomas => self.project.biomas.iter().map(|x| (x.id, x.name.clone())).collect(),
            Level::Mundos => self.project.mundos.iter().map(|x| (x.id, x.name.clone())).collect(),
            Level::Escenas => self.project.escenas.iter().map(|x| (x.id, x.name.clone())).collect(),
        }
    }

    /// Nombre del item seleccionado (para cargar el campo de renombrar).
    fn selected_name(&self) -> String {
        let id = self.selected();
        self.items()
            .into_iter()
            .find(|(i, _)| *i == id)
            .map(|(_, n)| n)
            .unwrap_or_default()
    }

    /// MundoRender que el centro previsualiza (según nivel/selección), con un
    /// fallback verde si no hay artefacto válido.
    fn preview_render(&self) -> MundoRender {
        let pick = match self.level {
            Level::Mundos | Level::Escenas => {
                let mundo = if self.level == Level::Mundos {
                    self.selected()
                } else {
                    self.project.escenas.iter().find(|e| e.id == self.selected()).map(|e| e.mundo).unwrap_or(0)
                };
                self.project.render_mundo(mundo)
            }
            Level::Biomas => self.project.render_bioma(self.selected()),
            // Seres: parados sobre el primer bioma disponible.
            Level::Seres => self.project.biomas.first().and_then(|b| self.project.render_bioma(b.id)),
            _ => None,
        };
        pick.unwrap_or_else(fallback_render)
    }
}

/// MundoRender por defecto (pradera neutra) cuando no hay artefacto que mostrar.
fn fallback_render() -> MundoRender {
    let bioma = Bioma {
        id: 0,
        name: String::new(),
        base: 0.22,
        dune: 0.10,
        relief: 0.5,
        mountains: 0.4,
        water_level: 0.30,
        rivers: 0.20,
        peak_at: 0.80,
        ground: 0,
        cliff: 0,
        peak: None,
        objetos: vec![],
        seres: vec![],
    };
    let palette = BiomaPalette::flat(Material::Grass.color(), Material::Grass.grain());
    MundoRender { bioma, seed: 1, palette }
}

/// Distancia de órbita inicial/clamp en función del lado del mundo.
fn default_dist() -> f32 {
    PREVIEW_DIM_XZ as f32 * 1.6
}

struct Studio;

impl App for Studio {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi-voxel-studio — creador de mundos"
    }
    fn initial_size() -> (u32, u32) {
        (1180, 760)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(Duration::from_millis(33), || Msg::Tick);
        demo_model()
    }

    fn on_wheel(_m: &Model, delta: WheelDelta, _c: (f32, f32), _mods: Modifiers) -> Option<Msg> {
        Some(Msg::Zoom(delta.y))
    }

    fn on_key(model: &Model, ev: &KeyEvent) -> Option<Msg> {
        // El teclado alimenta el campo enfocado (renombrar / semilla / IA).
        if model.name_focused {
            return Some(Msg::RenameKey(ev.clone()));
        }
        if model.formula_focused {
            return Some(Msg::FormulaKey(ev.clone()));
        }
        if model.seed_focused {
            return Some(Msg::SeedKey(ev.clone()));
        }
        if model.ai_focused {
            if ev.state == KeyState::Pressed && matches!(&ev.key, Key::Named(NamedKey::Enter)) {
                return Some(Msg::AiGenerate);
            }
            return Some(Msg::AiKey(ev.clone()));
        }
        None
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::SelectLevel(l) => {
                model.level = l;
                model.tool_tab = 0;
                model.name_focused = false;
                model.seed_focused = false;
                model.gen += 1;
                sync_inputs(&mut model);
            }
            Msg::SelectItem(id) => {
                model.set_selected(id);
                model.time = 0.0;
                model.gen += 1;
                sync_inputs(&mut model);
            }
            Msg::SelectTool(i) => model.tool_tab = i,
            Msg::NewItem => {
                let id = new_item(&mut model);
                model.set_selected(id);
                model.gen += 1;
                model.status = format!("nuevo en {}", model.level.label());
                sync_inputs(&mut model);
            }
            Msg::DupItem => {
                if let Some(id) = duplicate_item(&mut model) {
                    model.set_selected(id);
                    model.gen += 1;
                    model.status = "duplicado".into();
                    sync_inputs(&mut model);
                }
            }
            Msg::DelItem => {
                delete_selected(&mut model);
                // Reseleccionar el primero que quede.
                let first = model.items().first().map(|(i, _)| *i).unwrap_or(0);
                model.set_selected(first);
                model.gen += 1;
                model.status = "borrado".into();
                sync_inputs(&mut model);
            }
            Msg::RenameFocus => {
                model.name_focused = true;
                model.seed_focused = false;
                model.ai_focused = false;
            }
            Msg::RenameKey(ev) => {
                model.name_input.apply_key(&ev);
                let txt = model.name_input.text();
                set_selected_name(&mut model, txt);
            }
            Msg::CycleLeyKind => {
                if let Some(l) = model.project.leyes.iter_mut().find(|x| x.id == model.sel[0]) {
                    l.kind = l.kind.next();
                }
            }
            Msg::SetLeyParam(i, v) => {
                if let Some(l) = model.project.leyes.iter_mut().find(|x| x.id == model.sel[0]) {
                    l.kind.set_param(i, v);
                }
            }
            Msg::LeyPreset => {
                // Cicla la ley `Ecuacion` seleccionada por un catálogo de leyes de
                // fábrica (autoradas, editables). Cambio estructural → reconstruye el lab.
                if let Some(l) = sel_ley_mut(&mut model) {
                    if matches!(l.kind, LeyKind::Ecuacion { .. }) {
                        l.kind = next_preset(&l.kind);
                        model.formula_focused = false;
                        model.lab_gen += 1;
                    }
                }
            }
            Msg::SetEcuParam(i, v) => {
                if let Some(l) = sel_ley_mut(&mut model) {
                    l.kind.set_ecuacion_param(i, v);
                }
            }
            Msg::FormulaFocus(i) => {
                let src = sel_ley(&model)
                    .and_then(|l| match &l.kind {
                        LeyKind::Ecuacion { fuentes, .. } => fuentes.get(i).cloned(),
                        _ => None,
                    })
                    .unwrap_or_default();
                model.formula_input.set_text(src);
                model.formula_focused = true;
                model.formula_target = i;
                model.name_focused = false;
                model.seed_focused = false;
                model.ai_focused = false;
            }
            Msg::FormulaKey(ev) => {
                // Enter confirma (desenfoca); el resto edita el buffer y se escribe en vivo.
                if ev.state == KeyState::Pressed && matches!(&ev.key, Key::Named(NamedKey::Enter)) {
                    model.formula_focused = false;
                } else {
                    model.formula_input.apply_key(&ev);
                    let txt = model.formula_input.text();
                    let target = model.formula_target;
                    if let Some(l) = sel_ley_mut(&mut model) {
                        if let LeyKind::Ecuacion { fuentes, .. } = &mut l.kind {
                            if let Some(f) = fuentes.get_mut(target) {
                                *f = txt;
                            }
                        }
                    }
                    model.lab_gen += 1;
                }
            }
            Msg::AddCampo => {
                if let Some(l) = sel_ley_mut(&mut model) {
                    if let LeyKind::Ecuacion { campos, fuentes, .. } = &mut l.kind {
                        let name = format!("c{}", campos.len());
                        campos.push(FieldDef::new(name, 0.0, 0.0, 1.0));
                        fuentes.push("0".to_string());
                        model.lab_gen += 1;
                    }
                }
            }
            Msg::RemoveCampo => {
                if let Some(l) = sel_ley_mut(&mut model) {
                    if let LeyKind::Ecuacion { campos, fuentes, .. } = &mut l.kind {
                        if campos.len() > 1 {
                            campos.pop();
                            fuentes.pop();
                            model.formula_focused = false;
                            model.lab_gen += 1;
                        }
                    }
                }
            }
            Msg::AddEcuParam => {
                if let Some(l) = sel_ley_mut(&mut model) {
                    if let LeyKind::Ecuacion { params, .. } = &mut l.kind {
                        let name = format!("p{}", params.len());
                        params.push(ParamDef::new(name, 0.1, 0.0, 1.0));
                        model.lab_gen += 1;
                    }
                }
            }
            Msg::RemoveEcuParam => {
                if let Some(l) = sel_ley_mut(&mut model) {
                    if let LeyKind::Ecuacion { params, .. } = &mut l.kind {
                        params.pop();
                        model.lab_gen += 1;
                    }
                }
            }
            Msg::CycleLabField => {
                if let Some(lab) = &mut model.lab {
                    let n = lab.engine.fields().len().max(1);
                    lab.vis = (lab.vis + 1) % n;
                }
            }
            Msg::ReseedLab => {
                if let Some(lab) = &mut model.lab {
                    lab.reseed();
                }
            }
            Msg::ToggleLeyesView => {
                model.leyes_node_mode = !model.leyes_node_mode;
                model.formula_focused = false;
                if model.leyes_node_mode {
                    rebuild_eq_graph(&mut model);
                } else {
                    model.eq_graph = None;
                }
            }
            Msg::GraphDrag(id, dx, dy) => {
                if let Some(g) = &mut model.eq_graph {
                    g.drag(id, dx, dy); // layout puro: no recompila
                }
            }
            Msg::GraphConnect(from, _fp, to, tp) => {
                let ok = model.eq_graph.as_mut().map(|g| g.connect(from, to, tp)).unwrap_or(false);
                if ok {
                    sync_graph_to_fuentes(&mut model);
                }
            }
            Msg::GraphAddNode(op) => {
                if let Some(g) = &mut model.eq_graph {
                    g.add(op, 40.0, 40.0); // suelto; entra en la fórmula al conectarlo
                }
            }
            Msg::GraphDeleteNode(id) => {
                let had = model.eq_graph.is_some();
                if let Some(g) = &mut model.eq_graph {
                    g.delete(id);
                }
                if had {
                    sync_graph_to_fuentes(&mut model);
                }
            }
            Msg::CycleMatRole => {
                if let Some(m) = sel_material_mut(&mut model) {
                    m.role = match m.role {
                        MatRole::Terreno => MatRole::Objeto(Forma::Columnar),
                        MatRole::Objeto(_) => MatRole::Terreno,
                    };
                    model.gen += 1;
                }
            }
            Msg::SetMatColor(ch, v) => {
                if let Some(m) = sel_material_mut(&mut model) {
                    let mut c = m.color.unwrap_or([0.6, 0.6, 0.6]);
                    if ch < 3 {
                        c[ch] = v.clamp(0.0, 1.0);
                    }
                    m.color = Some(c);
                    model.gen += 1;
                }
            }
            Msg::SetMatGrain(v) => {
                if let Some(m) = sel_material_mut(&mut model) {
                    m.grain = Some(v.clamp(0.0, 1.0));
                    model.gen += 1;
                }
            }
            Msg::CycleMatParent => {
                let id = model.sel[1];
                // Lista de candidatos a padre (todos menos sí mismo).
                let cands: Vec<u64> = model.project.materiales.iter().map(|m| m.id).filter(|&i| i != id).collect();
                if let Some(m) = sel_material_mut(&mut model) {
                    m.parent = next_option(m.parent, &cands);
                    model.gen += 1;
                }
            }
            Msg::AddMatLey => {
                let first_ley = model.project.leyes.first().map(|l| l.id);
                if let (Some(ley), Some(m)) = (first_ley, sel_material_mut(&mut model)) {
                    m.leyes.push(LeyUso { ley, params: vec![] });
                }
            }
            Msg::RemoveMatLey => {
                if let Some(m) = sel_material_mut(&mut model) {
                    m.leyes.pop();
                }
            }
            Msg::CycleSereAge => {
                if let Some(c) = sel_sere_mut(&mut model) {
                    c.age = c.age.next();
                }
            }
            Msg::CycleSereCuerpo => {
                if let Some(c) = sel_sere_mut(&mut model) {
                    c.cycle_cuerpo();
                    model.gen += 1;
                }
            }
            Msg::SetSereColor(part, ch, v) => {
                if let Some(c) = sel_sere_mut(&mut model) {
                    let rgb = match part {
                        Part::Skin => &mut c.skin,
                        Part::Shirt => &mut c.shirt,
                        Part::Pants => &mut c.pants,
                    };
                    if ch < 3 {
                        rgb[ch] = v.clamp(0.0, 1.0);
                    }
                }
            }
            Msg::CycleAndarEstado => {
                model.andar_estado = (model.andar_estado + 1) % 3;
            }
            Msg::SetAndarCadencia(v) => {
                let est = model.andar_estado;
                if let Some(c) = sel_sere_mut(&mut model) {
                    if let Some(m) = &mut c.cuerpo {
                        m.andares.estado_mut(est).cadencia = v.clamp(0.0, 18.0);
                    }
                }
            }
            Msg::SetAndarAmplitud(seg, v) => {
                let est = model.andar_estado;
                if let Some(c) = sel_sere_mut(&mut model) {
                    if let Some(m) = &mut c.cuerpo {
                        if let Some(o) = m.andares.estado_mut(est).osc.get_mut(seg) {
                            o.amplitud = v.clamp(0.0, 1.5);
                        }
                    }
                }
            }
            Msg::SetConducta(i, v) => {
                if let Some(c) = sel_sere_mut(&mut model) {
                    c.conducta.set(i, v);
                }
            }
            Msg::ToggleManada => {
                model.manada = !model.manada;
                model.gen += 1; // repone el terreno y (re)inicia/limpia la manada
                model.status = if model.manada { "manada viva".into() } else { "manada quieta".into() };
            }
            Msg::SetBiomaField(f, v) => {
                if let Some(b) = sel_bioma_mut(&mut model) {
                    match f {
                        BiomaField::Base => b.base = v.clamp(0.0, 0.9),
                        BiomaField::Dune => b.dune = v.clamp(0.0, 0.4),
                        BiomaField::Relief => b.relief = v.clamp(0.0, 1.0),
                        BiomaField::Mountains => b.mountains = v.clamp(0.0, 1.0),
                        BiomaField::Water => b.water_level = v.clamp(0.0, 0.9),
                        BiomaField::Rivers => b.rivers = v.clamp(0.0, 1.0),
                        BiomaField::PeakAt => b.peak_at = v.clamp(0.0, 1.0),
                    }
                    model.gen += 1;
                }
            }
            Msg::CycleBiomaGround => {
                let mats = terreno_ids(&model);
                if let Some(b) = sel_bioma_mut(&mut model) {
                    b.ground = next_in(&mats, b.ground);
                    model.gen += 1;
                }
            }
            Msg::CycleBiomaCliff => {
                let mats = terreno_ids(&model);
                if let Some(b) = sel_bioma_mut(&mut model) {
                    b.cliff = next_in(&mats, b.cliff);
                    model.gen += 1;
                }
            }
            Msg::CycleBiomaPeak => {
                let mats = terreno_ids(&model);
                if let Some(b) = sel_bioma_mut(&mut model) {
                    b.peak = next_option(b.peak, &mats);
                    model.gen += 1;
                }
            }
            Msg::AddBiomaObjeto => {
                let obj = objeto_ids(&model);
                if let (Some(&mat), Some(b)) = (obj.first(), sel_bioma_mut(&mut model)) {
                    b.objetos.push(llimphi_voxel::ObjetoUso { material: mat, densidad: 0.01, forma: Forma::Columnar });
                    model.gen += 1;
                }
            }
            Msg::RemoveBiomaObjeto => {
                if let Some(b) = sel_bioma_mut(&mut model) {
                    b.objetos.pop();
                    model.gen += 1;
                }
            }
            Msg::SetObjetoDensidad(i, v) => {
                if let Some(b) = sel_bioma_mut(&mut model) {
                    if let Some(o) = b.objetos.get_mut(i) {
                        o.densidad = v.clamp(0.0, 0.05);
                    }
                    model.gen += 1;
                }
            }
            Msg::AddBiomaSere => {
                let sids = sere_ids(&model);
                if let (Some(&sere), Some(b)) = (sids.first(), sel_bioma_mut(&mut model)) {
                    b.seres.push(llimphi_voxel::SereUso { sere, probabilidad: 0.5 });
                    model.gen += 1;
                }
            }
            Msg::RemoveBiomaSere => {
                if let Some(b) = sel_bioma_mut(&mut model) {
                    b.seres.pop();
                    model.gen += 1;
                }
            }
            Msg::CycleBiomaSere(i) => {
                let sids = sere_ids(&model);
                if let Some(b) = sel_bioma_mut(&mut model) {
                    if let Some(u) = b.seres.get_mut(i) {
                        u.sere = next_in(&sids, u.sere);
                    }
                    model.gen += 1;
                }
            }
            Msg::SetBiomaSereProb(i, v) => {
                if let Some(b) = sel_bioma_mut(&mut model) {
                    if let Some(u) = b.seres.get_mut(i) {
                        u.probabilidad = v.clamp(0.0, 1.0);
                    }
                    model.gen += 1;
                }
            }
            Msg::SeedFocus => {
                model.seed_focused = true;
                model.name_focused = false;
                model.ai_focused = false;
            }
            Msg::SeedKey(ev) => {
                model.seed_input.apply_key(&ev);
                if let Ok(s) = model.seed_input.text().trim().parse::<u32>() {
                    if let Some(m) = sel_mundo_mut(&mut model) {
                        m.seed = s;
                        model.gen += 1;
                    }
                }
            }
            Msg::SeedRandom => {
                model.rng = model.rng.wrapping_mul(1664525).wrapping_add(1013904223);
                let s = model.rng % 100_000;
                if let Some(m) = sel_mundo_mut(&mut model) {
                    m.seed = s;
                }
                model.seed_input.set_text(&s.to_string());
                model.gen += 1;
            }
            Msg::CycleMundoBioma => {
                let bids = bioma_ids(&model);
                if let Some(m) = sel_mundo_mut(&mut model) {
                    let cur = m.biomas.first().copied().unwrap_or(0);
                    m.biomas = vec![next_in(&bids, cur)];
                    model.gen += 1;
                }
            }
            Msg::SelectActor(i) => {
                model.actor_sel = i;
                model.key_sel = 0;
            }
            Msg::AddActor => {
                let dim = world_dim(PREVIEW_DIM_XZ);
                let (cz, gx0, gx1) = (dim[2] as f32 * 0.5, dim[0] as f32 * 0.35, dim[0] as f32 * 0.65);
                let dur = sel_scene(&model).map(|s| s.duration).unwrap_or(5.0);
                if let Some(s) = sel_scene_mut(&mut model) {
                    s.actors.push(ActorSpec {
                        character: 0,
                        keys: vec![
                            ActorKeySpec { t: 0.0, gx: gx0, gz: cz, clip: Some(Clip::Walk), face: None },
                            ActorKeySpec { t: dur, gx: gx1, gz: cz, clip: Some(Clip::Walk), face: None },
                        ],
                        frame_rate: None,
                    });
                    model.actor_sel = s.actors.len() - 1;
                    model.key_sel = 0;
                    model.gen += 1;
                }
            }
            Msg::RemoveActor => {
                let a = model.actor_sel;
                if let Some(s) = sel_scene_mut(&mut model) {
                    if s.actors.len() > 1 && a < s.actors.len() {
                        s.actors.remove(a);
                    }
                }
                model.actor_sel = model.actor_sel.saturating_sub(1);
                model.key_sel = 0;
                model.gen += 1;
            }
            Msg::CycleActorSere => {
                let a = model.actor_sel;
                let n = model.project.seres.len().max(1);
                if let Some(s) = sel_scene_mut(&mut model) {
                    if let Some(act) = s.actors.get_mut(a) {
                        act.character = (act.character + 1) % n;
                    }
                    model.gen += 1;
                }
            }
            Msg::SelectKey(i) => model.key_sel = i,
            Msg::AddKey => {
                let a = model.actor_sel;
                let dur = sel_scene(&model).map(|s| s.duration).unwrap_or(5.0);
                let dimx = world_dim(PREVIEW_DIM_XZ)[0] as f32;
                if let Some(s) = sel_scene_mut(&mut model) {
                    if let Some(act) = s.actors.get_mut(a) {
                        let mut k = act.keys.last().cloned().unwrap_or(ActorKeySpec {
                            t: 0.0,
                            gx: dimx * 0.5,
                            gz: dimx * 0.5,
                            clip: Some(Clip::Walk),
                            face: None,
                        });
                        k.gx = (k.gx + 12.0).min(dimx - 2.0); // nuevo waypoint adelante
                        act.keys.push(k);
                        respread(act, dur);
                        model.key_sel = act.keys.len() - 1;
                    }
                    model.gen += 1;
                }
            }
            Msg::RemoveKey => {
                let (a, k) = (model.actor_sel, model.key_sel);
                let dur = sel_scene(&model).map(|s| s.duration).unwrap_or(5.0);
                if let Some(s) = sel_scene_mut(&mut model) {
                    if let Some(act) = s.actors.get_mut(a) {
                        if act.keys.len() > 2 && k < act.keys.len() {
                            act.keys.remove(k);
                            respread(act, dur);
                        }
                    }
                    model.key_sel = model.key_sel.saturating_sub(1);
                    model.gen += 1;
                }
            }
            Msg::SetKeyPos(is_gx, v) => {
                let (a, k) = (model.actor_sel, model.key_sel);
                let lim = world_dim(PREVIEW_DIM_XZ)[0] as f32 - 1.0;
                if let Some(s) = sel_scene_mut(&mut model) {
                    if let Some(key) = s.actors.get_mut(a).and_then(|act| act.keys.get_mut(k)) {
                        if is_gx {
                            key.gx = v.clamp(1.0, lim);
                        } else {
                            key.gz = v.clamp(1.0, lim);
                        }
                    }
                    model.gen += 1;
                }
            }
            Msg::SetKeyTime(v) => {
                let (a, k) = (model.actor_sel, model.key_sel);
                let dur = sel_scene(&model).map(|s| s.duration).unwrap_or(5.0);
                if let Some(s) = sel_scene_mut(&mut model) {
                    if let Some(key) = s.actors.get_mut(a).and_then(|act| act.keys.get_mut(k)) {
                        key.t = v.clamp(0.0, dur);
                    }
                    model.gen += 1;
                }
            }
            Msg::CycleKeyClip => {
                let (a, k) = (model.actor_sel, model.key_sel);
                if let Some(s) = sel_scene_mut(&mut model) {
                    if let Some(key) = s.actors.get_mut(a).and_then(|act| act.keys.get_mut(k)) {
                        key.clip = Some(key.clip.unwrap_or(Clip::Idle).next());
                    }
                    model.gen += 1;
                }
            }
            Msg::PlaceKeyAt(gx, gz) => {
                let (a, k) = (model.actor_sel, model.key_sel);
                if let Some(s) = sel_scene_mut(&mut model) {
                    if let Some(key) = s.actors.get_mut(a).and_then(|act| act.keys.get_mut(k)) {
                        key.gx = gx;
                        key.gz = gz;
                    }
                    model.gen += 1;
                }
            }
            Msg::CycleSceneMundo => {
                let mids = mundo_ids(&model);
                if let Some(s) = sel_scene_mut(&mut model) {
                    s.mundo = next_in(&mids, s.mundo);
                    model.gen += 1;
                }
            }
            Msg::SetSceneDur(v) => {
                if let Some(s) = sel_scene_mut(&mut model) {
                    s.duration = v.clamp(1.0, 20.0);
                }
            }
            Msg::TogglePlay => model.playing = !model.playing,
            Msg::Scrub(v) => {
                model.playing = false;
                let dur = sel_scene(&model).map(|s| s.duration).unwrap_or(1.0);
                model.time = v.clamp(0.0, dur);
            }
            Msg::ToggleSceneCam => model.script_cam = !model.script_cam,
            Msg::AddShot => {
                let t = model.time;
                if let Some(s) = sel_scene_mut(&mut model) {
                    let kind = ShotKind::ALL[s.shots.len() % ShotKind::ALL.len()];
                    s.shots.push(ShotSpec { start: t, kind });
                    s.shots.sort_by(|a, b| a.start.total_cmp(&b.start));
                    model.status = format!("plano: {} @ {t:.1}s", kind.label());
                }
            }
            Msg::RemoveShot => {
                if let Some(s) = sel_scene_mut(&mut model) {
                    s.shots.pop();
                }
            }
            Msg::ExportVideo => {
                if !model.exporting {
                    if let Some(scene) = sel_scene(&model).cloned() {
                        model.exporting = true;
                        model.status = "exportando video… (puede tardar)".into();
                        let project = model.project.clone();
                        handle.spawn(move || Msg::ExportDone(render::export_scene(&project, &scene)));
                    }
                }
            }
            Msg::ExportDone(res) => {
                model.exporting = false;
                model.status = match res {
                    Ok(p) => format!("video listo: {p}"),
                    Err(e) => format!("export falló: {e}"),
                };
            }
            Msg::ToggleSim => {
                model.simulating = !model.simulating;
                model.gen += 1; // repone terreno fresco y reinicia la sim limpia
                model.status = match (model.level, model.simulating) {
                    (Level::Leyes, true) => "corriendo la ley…".into(),
                    (Level::Leyes, false) => "ley en pausa".into(),
                    (_, true) => "simulando agua…".into(),
                    (_, false) => "agua estática".into(),
                };
            }
            Msg::Orbit(dx, dy) => {
                model.yaw -= dx * 0.008;
                let lim = std::f32::consts::FRAC_PI_2 - 0.05;
                model.pitch = (model.pitch + dy * 0.008).clamp(-lim, lim);
            }
            Msg::Zoom(dy) => {
                let f = (1.0 + dy * 0.1).clamp(0.5, 1.5);
                let xz = PREVIEW_DIM_XZ as f32;
                model.dist = (model.dist * f).clamp(xz * 0.6, xz * 3.0);
            }
            Msg::AiFocus => {
                model.ai_focused = true;
                model.name_focused = false;
                model.seed_focused = false;
            }
            Msg::AiKey(ev) => {
                model.ai_input.apply_key(&ev);
            }
            Msg::AiGenerate => {
                let prompt = model.ai_input.text();
                if prompt.trim().is_empty() || model.ai_busy {
                    return model;
                }
                model.ai_busy = true;
                model.status = "generando con IA…".into();
                match model.level {
                    Level::Biomas | Level::Mundos => {
                        let refs = MatRefs::from_project(&model.project);
                        handle.spawn(move || Msg::AiBioma(ai::generate_bioma(&prompt, &refs)));
                    }
                    Level::Seres => {
                        handle.spawn(move || Msg::AiSere(ai::generate_character(&prompt)));
                    }
                    Level::Escenas => {
                        let mundo = model.project.mundos.first().map(|m| m.id).unwrap_or(0);
                        let dim = world_dim(PREVIEW_DIM_XZ);
                        handle.spawn(move || Msg::AiScene(ai::generate_scene(&prompt, mundo, dim)));
                    }
                    _ => {
                        model.ai_busy = false;
                        model.status = "la IA genera biomas, mundos, seres o escenas".into();
                    }
                }
            }
            Msg::AiBioma(mut b) => {
                b.id = model.project.alloc_id();
                let bid = b.id;
                let name = b.name.clone();
                model.project.biomas.push(b);
                // En modo Mundos, envolver el bioma en un mundo nuevo.
                if model.level == Level::Mundos {
                    let id = model.project.alloc_id();
                    model.project.mundos.push(Mundo { id, name: name.clone(), seed: 1337, biomas: vec![bid] });
                    model.set_selected(id);
                } else {
                    model.level = Level::Biomas;
                    model.set_selected(bid);
                }
                model.ai_busy = false;
                model.ai_input.set_text("");
                model.gen += 1;
                model.status = format!("generado: «{name}»");
                sync_inputs(&mut model);
            }
            Msg::AiSere(mut c) => {
                c.id = model.project.alloc_id();
                let id = c.id;
                let name = c.name.clone();
                model.project.seres.push(c);
                model.level = Level::Seres;
                model.set_selected(id);
                model.ai_busy = false;
                model.ai_input.set_text("");
                model.status = format!("ser generado: «{name}»");
                sync_inputs(&mut model);
            }
            Msg::AiScene(mut s) => {
                s.id = model.project.alloc_id();
                let id = s.id;
                model.project.escenas.push(s);
                model.level = Level::Escenas;
                model.set_selected(id);
                model.time = 0.0;
                model.playing = true;
                model.ai_busy = false;
                model.ai_input.set_text("");
                model.gen += 1;
                model.status = "escena generada".into();
                sync_inputs(&mut model);
            }
            Msg::Save => match save_project(&model.project) {
                Ok(_) => model.status = format!("guardado en {PROJECT_PATH}"),
                Err(e) => model.status = format!("error al guardar: {e}"),
            },
            Msg::Load => match load_project() {
                Ok(p) => {
                    model.project = p;
                    model.gen += 1;
                    reselect_all(&mut model);
                    model.status = format!("cargado de {PROJECT_PATH}");
                    sync_inputs(&mut model);
                }
                Err(e) => model.status = format!("error al cargar: {e}"),
            },
            Msg::Tick => {
                if model.level == Level::Escenas && model.playing {
                    if let Some(dur) = sel_scene(&model).map(|s| s.duration) {
                        model.time += DT;
                        if model.time >= dur {
                            model.time = 0.0;
                        }
                    }
                } else if model.level == Level::Seres {
                    model.time += DT; // turntable + respiración
                } else if model.level == Level::Leyes {
                    ensure_lab(&mut model);
                    // En modo grafo, reconstruir si el texto cambió afuera (no por edición del grafo).
                    if model.leyes_node_mode
                        && (model.eq_graph.is_none() || model.eq_graph_gen != model.lab_gen)
                    {
                        rebuild_eq_graph(&mut model);
                    }
                    if model.simulating {
                        let params = sel_ley(&model)
                            .map(|l| l.kind.ecuacion_param_values())
                            .unwrap_or_default();
                        if let Some(lab) = &mut model.lab {
                            lab.step(&params);
                        }
                    }
                }
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        // Los sidebars reservan su franja a los lados (el centro lleva un canvas GPU
        // que, a pantalla completa, taparía el chrome — overlay no es viable acá). La
        // **orientación** de los dientes sí sigue la convención global: sobresalen
        // hacia el centro (izq InnerLeft, der InnerRight). `dientes_outside` decide si
        // las franjas van al ras (FUERA) o flotando con margen (DENTRO).
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(model.theme.bg_app)
        .children(vec![left_sidebar(model), center(model), right_sidebar(model)])
    }
}

// =============================================================================
//  Sidebars
// =============================================================================

/// Sidebar izquierdo: rail de **niveles** (dientes coloridos) + lista del nivel con
/// CRUD + renombrar + IA + estado.
fn left_sidebar(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let btn = ButtonPalette::from_theme(theme);

    // Rail de niveles. El diente abre hacia el centro (a la derecha) → InnerLeft.
    let active = model.level.index();
    let items: Vec<DockRailItem> = (0..Level::ALL.len())
        .map(|i| DockRailItem { id: i as u64, active: i == active })
        .collect();
    let rail = dock_rail_view_side(
        &items,
        46.0,
        DockRailSide::InnerLeft,
        &DockRailPalette::from_theme(theme),
        |id, _size, _color| {
            let lvl = Level::from_index(id as usize);
            icon_view(lvl.icon(), lvl.color(), 1.8)
        },
        |id| Msg::SelectLevel(Level::from_index(id as usize)),
        |_| None,
    );

    // Contenido: título + lista + CRUD + renombrar.
    let mut rows: Vec<View<Msg>> = Vec::new();
    rows.push(section_title(model.level.label(), theme));
    for (id, name) in model.items() {
        rows.push(selectable_row(name, id == model.selected(), Msg::SelectItem(id), theme));
    }
    rows.push(spacer(8.0));
    // Botonera CRUD en fila.
    rows.push(crud_row(&btn));
    rows.push(spacer(8.0));
    rows.push(section_title("NOMBRE", theme));
    rows.push(text_input_view(
        &model.name_input,
        "nombre…",
        model.name_focused,
        &TextInputPalette::from_theme(theme),
        Msg::RenameFocus,
    ));

    // IA (según nivel).
    if matches!(model.level, Level::Biomas | Level::Mundos | Level::Seres | Level::Escenas) {
        let hint = match model.level {
            Level::Biomas | Level::Mundos => "p.ej. islas con ríos y nieve",
            Level::Seres => "p.ej. una niña de remera verde",
            _ => "p.ej. tres personajes que festejan",
        };
        rows.push(spacer(12.0));
        rows.push(section_title("IA — DESCRIBÍ", theme));
        rows.push(text_input_view(
            &model.ai_input,
            hint,
            model.ai_focused,
            &TextInputPalette::from_theme(theme),
            Msg::AiFocus,
        ));
        rows.push(spacer(6.0));
        rows.push(button_view(
            if model.ai_busy { "generando…" } else { "generar (IA)" },
            &btn,
            Msg::AiGenerate,
        ));
    }

    // Guardar/cargar + estado.
    rows.push(spacer(12.0));
    rows.push(button_view("guardar", &btn, Msg::Save));
    rows.push(spacer(4.0));
    rows.push(button_view("cargar", &btn, Msg::Load));
    rows.push(spacer(10.0));
    rows.push(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: Dimension::auto() },
            ..Default::default()
        })
        .text(model.status.clone(), 12.0, theme.fg_placeholder)
        .max_lines(3),
    );

    let content = View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
        padding: pad(12.0, 12.0),
        gap: gap_y(6.0),
        ..Default::default()
    })
    .children(rows);

    // Contenido a la izquierda (borde exterior), rail a la derecha (borde interno).
    sidebar_frame(theme, 256.0, !model.dientes_outside, vec![content, rail])
}

/// Sidebar derecho: rail de **herramientas** del nivel (dientes coloridos, espejados
/// hacia el centro) + el editor del set activo.
fn right_sidebar(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let tools = model.level.tools();
    let tab = model.tool_tab.min(tools.len().saturating_sub(1));
    let lvl_color = model.level.color();

    let items: Vec<DockRailItem> = (0..tools.len())
        .map(|i| DockRailItem { id: i as u64, active: i == tab })
        .collect();
    let icons: Vec<Icon> = tools.iter().map(|(_, ic)| *ic).collect();
    let rail = dock_rail_view_side(
        &items,
        46.0,
        DockRailSide::InnerRight,
        &DockRailPalette::from_theme(theme),
        move |id, _size, _color| icon_view(icons[id as usize], lvl_color, 1.8),
        |id| Msg::SelectTool(id as usize),
        |_| None,
    );

    let panel = View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
        padding: pad(14.0, 14.0),
        gap: gap_y(8.0),
        ..Default::default()
    })
    .children(editor(model, tab));

    // Rail a la izquierda (borde interno hacia el centro), editor a la derecha.
    sidebar_frame(theme, 300.0, !model.dientes_outside, vec![rail, panel])
}

/// Marco común de un sidebar (fondo de panel + ancho fijo + fila rail/contenido).
/// `floating` (dientes DENTRO) le da margen y esquinas redondeadas para que la franja
/// "flote"; al ras (FUERA) va pegada al borde.
fn sidebar_frame(theme: &Theme, width: f32, floating: bool, children: Vec<View<Msg>>) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::prelude::Rect;
    let m: f32 = if floating { 6.0 } else { 0.0 };
    let mut v = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: length(width), height: percent(1.0_f32) },
        margin: Rect { left: length(m), right: length(m), top: length(m), bottom: length(m) },
        padding: pad(0.0, 6.0),
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(children);
    if floating {
        v = v.radius(10.0);
    }
    v
}

/// Fila de botones CRUD: nuevo · duplicar · borrar.
fn crud_row(btn: &ButtonPalette) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        cell(button_view("+ nuevo", btn, Msg::NewItem)),
        cell(button_view("duplicar", btn, Msg::DupItem)),
        cell(button_view("borrar", btn, Msg::DelItem)),
    ])
}

/// Celda que crece para repartir el ancho de una fila.
fn cell(child: View<Msg>) -> View<Msg> {
    View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: Dimension::auto() },
        ..Default::default()
    })
    .children(vec![child])
}

// =============================================================================
//  Centro (preview)
// =============================================================================

fn center(model: &Model) -> View<Msg> {
    let inner = match model.level {
        Level::Leyes if model.leyes_node_mode => eq_graph_view(model),
        Level::Leyes => law_lab_view(model),
        Level::Materiales => material_swatch(model),
        _ => canvas_3d(model),
    };
    View::new(Style {
        flex_grow: 1.0,
        size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(Color::from_rgba8(12, 14, 18, 255))
    .children(vec![inner])
}

/// Mensaje centrado para niveles sin preview 3D.
fn placeholder_2d(text: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(llimphi_ui::llimphi_layout::taffy::prelude::JustifyContent::Center),
        padding: pad(40.0, 40.0),
        ..Default::default()
    })
    .children(vec![View::new(Style {
        size: Size { width: percent(0.7_f32), height: Dimension::auto() },
        ..Default::default()
    })
    .text(text.to_string(), 16.0, theme.fg_muted)
    .max_lines(4)])
}

/// Centro del nivel Leyes: el **laboratorio** — heatmap del campo visible de la ley
/// corriendo la ecuación, o el error de parseo, o una guía si no hay ley por ecuación.
fn law_lab_view(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let Some(lab) = &model.lab else {
        return placeholder_2d(
            "Elegí una ley por ecuación (botón «preset» a la derecha) y editá su fórmula. ▶ para simular.",
            theme,
        );
    };
    if let Err(err) = &lab.program {
        return placeholder_2d(&format!("La fórmula no compila:\n{err}"), theme);
    }
    let field_name = lab.engine.fields().get(lab.vis).map(|f| f.name.clone()).unwrap_or_default();
    let estado = if model.simulating { "▶ corriendo" } else { "▮ en pausa" };
    let overlay = View::new(Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        padding: pad(12.0, 10.0),
        ..Default::default()
    })
    .text(format!("campo «{field_name}»  ·  {LAB_DIM}²  ·  {estado}"), 13.0, theme.fg_muted);
    View::new(Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(llimphi_ui::llimphi_layout::taffy::prelude::JustifyContent::Center),
        padding: pad(24.0, 24.0),
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size { width: percent(0.82_f32), height: percent(0.82_f32) },
            ..Default::default()
        })
        .image(heatmap_image(lab)),
        overlay,
    ])
}

/// Centro del nivel Leyes en **modo grafo**: la ecuación como nodos (misma AST que la
/// fórmula). Arrastrar la barra de título reubica; arrastrar de un pin de salida a uno
/// de entrada reconecta (y reescribe la fórmula); right‑click borra el nodo.
fn eq_graph_view(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let (Some(g), Some(sym)) = (&model.eq_graph, sel_ley(model).and_then(|l| l.kind.ecuacion_symbols()))
    else {
        return placeholder_2d("Grafo no disponible (la ley no es por ecuación).", theme);
    };
    let specs = g.node_specs(&sym);
    let palette = NodegraphPalette::from_theme(theme);
    let metrics = NodegraphMetrics::default();
    let inner = nodegraph_view_ex(
        &specs,
        &g.wires,
        &palette,
        &metrics,
        |id, phase, dx, dy| match phase {
            DragPhase::Move => Some(Msg::GraphDrag(id, dx, dy)),
            _ => None,
        },
        |from, fp, to, tp| Some(Msg::GraphConnect(from, fp, to, tp)),
        Some(|id| Some(Msg::GraphDeleteNode(id))),
    );
    View::new(Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![inner])
}

/// Heatmap RGBA del campo visible del laboratorio (rango del campo → rampa magma).
fn heatmap_image(lab: &LawLab) -> Image {
    let f = lab.vis as u16;
    let (mn, mx) = lab.engine.fields().get(lab.vis).map(|d| (d.min, d.max)).unwrap_or((0.0, 1.0));
    let span = (mx - mn).max(1e-6);
    let mut rgba = vec![0u8; (LAB_DIM * LAB_DIM * 4) as usize];
    for z in 0..LAB_DIM {
        for x in 0..LAB_DIM {
            let t = ((lab.engine.get(f, x, 0, z) - mn) / span).clamp(0.0, 1.0);
            let [r, g, b] = ramp(t);
            let i = ((z * LAB_DIM + x) * 4) as usize;
            rgba[i] = r;
            rgba[i + 1] = g;
            rgba[i + 2] = b;
            rgba[i + 3] = 255;
        }
    }
    from_rgba8(rgba, LAB_DIM, LAB_DIM)
}

/// Rampa de color tipo *magma* (negro → púrpura → naranja → crema) para `t∈[0,1]`.
fn ramp(t: f32) -> [u8; 3] {
    const STOPS: [(f32, [f32; 3]); 4] = [
        (0.0, [8.0, 6.0, 30.0]),
        (0.4, [92.0, 22.0, 110.0]),
        (0.72, [222.0, 92.0, 58.0]),
        (1.0, [250.0, 232.0, 158.0]),
    ];
    let t = t.clamp(0.0, 1.0);
    let mut out = STOPS[STOPS.len() - 1].1;
    for w in STOPS.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t <= t1 {
            let k = ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0);
            out = [
                c0[0] + (c1[0] - c0[0]) * k,
                c0[1] + (c1[1] - c0[1]) * k,
                c0[2] + (c1[2] - c0[2]) * k,
            ];
            break;
        }
    }
    [out[0] as u8, out[1] as u8, out[2] as u8]
}

/// Swatch grande del color resuelto del material seleccionado.
fn material_swatch(model: &Model) -> View<Msg> {
    let c = model.project.resolve_material(model.sel[1]).color;
    let col = Color::from_rgba8(c[0], c[1], c[2], 255);
    View::new(Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(llimphi_ui::llimphi_layout::taffy::prelude::JustifyContent::Center),
        padding: pad(60.0, 60.0),
        ..Default::default()
    })
    .children(vec![View::new(Style {
        size: Size { width: percent(0.6_f32), height: percent(0.6_f32) },
        ..Default::default()
    })
    .fill(col)
    .radius(16.0)])
}

/// Canvas 3D: terreno del artefacto + (en escenas) los actores posados.
fn canvas_3d(model: &Model) -> View<Msg> {
    let (yaw, pitch, dist, gen) = (model.yaw, model.pitch, model.dist, model.gen);
    let preview = model.preview.clone();
    // Dirigiendo (Escenas + pestaña Reparto): el click coloca la clave seleccionada.
    let directing = model.level == Level::Escenas && model.tool_tab == 1 && sel_scene(model).is_some();
    let preview_click = model.preview.clone();
    let mr = model.preview_render();
    let simulating = model.simulating;
    let agua = mr.palette.agua;
    // El agua fluye SÓLO si su material tiene la ley Fluir (con sus params).
    let fluir = model.project.water_fluir();
    // Y las plantas (objetos) crecen si su material tiene la ley Crecer.
    let crecer = mr.bioma.objetos.iter().find_map(|o| {
        model
            .project
            .crecer_velocidad(o.material)
            .map(|v| (model.project.resolve_material(o.material).color, v))
    });
    // Pobladores del bioma: cada SereUso → unos cuantos habitantes (por probabilidad).
    let pobladores: Vec<(CharSpec, usize)> = mr
        .bioma
        .seres
        .iter()
        .filter_map(|su| {
            model
                .project
                .sere(su.sere)
                .map(|cs| (cs.clone(), ((su.probabilidad.clamp(0.0, 1.0) * 6.0).round() as usize).max(1)))
        })
        .collect();
    let absolute = Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    };

    let mut canvas = match model.level {
        Level::Escenas => {
            let scene = sel_scene(model).cloned();
            let script_cam = model.script_cam;
            // Recorrido a dibujar en 3D mientras se dirige (pestaña Reparto): los
            // waypoints del actor seleccionado + cuál está seleccionado.
            let path: Option<(Vec<(f32, f32)>, usize)> = if model.tool_tab == 1 {
                scene.as_ref().and_then(|s| s.actors.get(model.actor_sel)).map(|act| {
                    (act.keys.iter().map(|k| (k.gx, k.gz)).collect(), model.key_sel)
                })
            } else {
                None
            };
            let script_cam = if path.is_some() { false } else { script_cam }; // órbita para dirigir
            let scripts: Vec<ActorScript> = scene.as_ref().map(|s| s.scripts()).unwrap_or_default();
            let chars: Vec<CharSpec> = scene
                .as_ref()
                .map(|s| s.actors.iter().map(|a| model.project.character_or_default(a.character)).collect())
                .unwrap_or_default();
            let time = model.time;
            View::new(absolute).gpu_paint_with(move |device, queue, encoder, target, rect, vp| {
                let dim = world_dim(PREVIEW_DIM_XZ);
                let mut guard = preview.lock().unwrap();
                let p = guard.get_or_insert_with(|| WorldPreview::build(device, queue, &mr, dim, gen));
                // Al dirigir, ventana fija en el origen → la columna de grilla coincide
                // con la de mundo (el picking del click queda directo).
                let origin = if directing { [0, 0] } else { window_origin_for_cast(&scripts, time, dim) };
                p.ensure_window(device, queue, &mr, gen, origin);
                let half = Vec3::new(dim[0] as f32, dim[1] as f32, dim[2] as f32) * 0.5;
                let mut poses = Vec::with_capacity(scripts.len());
                let mut centroid = Vec3::ZERO;
                for (script, ch) in scripts.iter().zip(&chars) {
                    let at = script.quantize(time);
                    let s = script.sample(at);
                    let pos = p.ground_at_world(s.gx as i32, s.gz as i32) - half;
                    centroid += pos;
                    poses.push((pos, s, ch, at));
                }
                let look = if poses.is_empty() {
                    orbit_center(dim) - half
                } else {
                    centroid / poses.len() as f32 + Vec3::new(0.0, 1.0, 0.0)
                };
                let cast_d = 6.0 + poses.len() as f32 * 1.2;
                let scene_dist = (dist * 0.18).clamp(10.0, 70.0);
                let camera = match (script_cam, &scene) {
                    (true, Some(sc)) => sc.camera_at(look, cast_d, time),
                    _ => Camera3d::orbit(look, yaw, pitch, scene_dist),
                };
                // Guardar la cámara para resolver clicks → suelo (plano a la altura de pies).
                if directing && rect.h > 0.0 {
                    let inv_vp = camera.view_proj(rect.w / rect.h).inverse();
                    p.set_pick(inv_vp, camera.eye, look.y - 1.0);
                }
                let mut metas = Vec::with_capacity(poses.len());
                for (pos, s, ch, at) in &poses {
                    // Humanoide → Actor rico; rig → su andar (lo decide CharSpec).
                    metas.push(ch.to_meta(*pos, s.facing, s.clip, *at, Some(camera.eye)));
                }
                // Recorrido: un marcador (cubo) por waypoint del actor dirigido; el
                // seleccionado, más grande y dorado.
                if let Some((keys, sel)) = &path {
                    for (i, (gx, gz)) in keys.iter().enumerate() {
                        let pos = p.ground_at_world(*gx as i32, *gz as i32) - half + Vec3::new(0.0, 1.4, 0.0);
                        let (color, sz) = if i == *sel { ([0.96, 0.84, 0.20], 0.8) } else { ([0.55, 0.58, 0.66], 0.5) };
                        let (mut mv, mut mi) = (Vec::new(), Vec::new());
                        push_cube(&mut mv, &mut mi, Mat4::from_translation(pos) * Mat4::from_scale(Vec3::splat(sz)), color);
                        metas.push((Mat4::IDENTITY, mv, mi));
                    }
                }
                p.render_scene(device, queue, encoder, target, vp, (rect.x, rect.y, rect.w, rect.h), &camera, &metas);
            })
        }
        Level::Seres => {
            let sere = sel_sere(model).cloned();
            let time = model.time;
            let manada = model.manada;
            // Previsualizar el estado de andar que se está editando.
            let estado_clip = match model.andar_estado {
                0 => Clip::Idle,
                1 => Clip::Walk,
                _ => Clip::Run,
            };
            View::new(absolute).gpu_paint_with(move |device, queue, encoder, target, rect, vp| {
                let dim = world_dim(PREVIEW_DIM_XZ);
                let mut guard = preview.lock().unwrap();
                let p = guard.get_or_insert_with(|| WorldPreview::build(device, queue, &mr, dim, gen));
                p.rebuild_if(device, queue, &mr, dim, gen);
                let half = Vec3::new(dim[0] as f32, dim[1] as f32, dim[2] as f32) * 0.5;
                match (&sere, manada) {
                    // Manada viva: una bandada del ser deambula/se junta por su conducta.
                    (Some(cs), true) => {
                        p.ensure_manada(&[(cs.clone(), 9)]);
                        let metas = p.manada_metas(1.0 / 30.0, None);
                        let look = p.ground_at(dim[0] / 2, dim[2] / 2) - half + Vec3::new(0.0, 1.0, 0.0);
                        let camera = Camera3d::orbit(look, yaw, pitch, 32.0);
                        p.render_scene(device, queue, encoder, target, vp, (rect.x, rect.y, rect.w, rect.h), &camera, &metas);
                    }
                    // Turntable de un solo ser, animando el estado en edición.
                    other => {
                        p.clear_manada();
                        let pos = p.ground_at(dim[0] / 2, dim[2] / 2);
                        let look = pos + Vec3::new(0.0, 1.0, 0.0);
                        let cam_dist = (dist * 0.06).clamp(3.5, 14.0);
                        let camera = Camera3d::orbit(look, yaw, pitch, cam_dist);
                        let metas = match other.0 {
                            Some(cs) => vec![cs.to_meta(pos, time * 0.6, estado_clip, time, None)],
                            None => Vec::new(),
                        };
                        p.render_scene(device, queue, encoder, target, vp, (rect.x, rect.y, rect.w, rect.h), &camera, &metas);
                    }
                }
            })
        }
        // Mundos / Biomas: sólo el terreno, en órbita. Con «simular», corre la ley
        // Fluir: el agua cae/esparce/cae por cornisas, paso por cuadro.
        _ => View::new(absolute).gpu_paint_with(move |device, queue, encoder, target, rect, vp| {
            let dim = world_dim(PREVIEW_DIM_XZ);
            let mut guard = preview.lock().unwrap();
            let p = guard.get_or_insert_with(|| WorldPreview::build(device, queue, &mr, dim, gen));
            p.rebuild_if(device, queue, &mr, dim, gen);
            if simulating {
                if let Some((g, h)) = fluir {
                    p.ensure_sim(agua, g, h);
                    p.sim_step(queue, agua);
                }
                if let Some((col, vel)) = crecer {
                    p.ensure_growth(queue, col, vel);
                    p.growth_step(queue);
                }
                if !pobladores.is_empty() {
                    p.ensure_manada(&pobladores);
                }
            } else {
                p.clear_sim();
                p.clear_growth();
                p.clear_manada();
            }
            let camera = Camera3d::orbit(orbit_center(dim), yaw, pitch, dist);
            // Con bandada, el mundo se ve vivo: se componen los habitantes caminando.
            if simulating && p.tiene_manada() {
                let metas = p.manada_metas(1.0 / 30.0, None);
                p.render_scene(device, queue, encoder, target, vp, (rect.x, rect.y, rect.w, rect.h), &camera, &metas);
            } else {
                p.render(device, queue, encoder, target, vp, (rect.x, rect.y, rect.w, rect.h), &camera);
            }
        }),
    }
    .draggable(|phase, dx, dy| match phase {
        DragPhase::Move => Some(Msg::Orbit(dx, dy)),
        DragPhase::End => None,
    });

    // Al dirigir, un click (sin arrastrar) coloca la clave seleccionada sobre el suelo.
    if directing {
        canvas = canvas.on_click_at(move |lx, ly, rw, rh| {
            if rw <= 0.0 || rh <= 0.0 {
                return None;
            }
            let ndc_x = lx / rw * 2.0 - 1.0;
            let ndc_y = 1.0 - ly / rh * 2.0;
            let guard = preview_click.lock().ok()?;
            let (gx, gz) = guard.as_ref()?.pick_world(ndc_x, ndc_y)?;
            Some(Msg::PlaceKeyAt(gx, gz))
        });
    }
    canvas
}

/// Centro de órbita: el medio del mundo, algo por debajo del tope.
fn orbit_center(dim: [u32; 3]) -> Vec3 {
    Vec3::new(dim[0] as f32 * 0.5, dim[1] as f32 * 0.32, dim[2] as f32 * 0.5)
}

// =============================================================================
//  Editores por nivel (rail derecho)
// =============================================================================

fn editor(model: &Model, tab: usize) -> Vec<View<Msg>> {
    match model.level {
        Level::Leyes => ley_editor(model),
        Level::Materiales => material_editor(model, tab),
        Level::Seres => sere_editor(model, tab),
        Level::Biomas => bioma_editor(model, tab),
        Level::Mundos => mundo_editor(model, tab),
        Level::Escenas => scene_editor(model, tab),
    }
}

fn ley_editor(model: &Model) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let sp = SliderPalette::from_theme(theme);
    let btn = ButtonPalette::from_theme(theme);
    let Some(l) = model.project.ley(model.sel[0]) else {
        return vec![section_title("sin ley — creá una", theme)];
    };
    let mut v = vec![
        section_title("LEY", theme),
        button_view(format!("tipo: {}", l.kind.label()), &btn, Msg::CycleLeyKind),
    ];
    match &l.kind {
        // --- Ley autorable: editor de ecuaciones + laboratorio -----------------
        LeyKind::Ecuacion { campos, fuentes, params } => {
            v.push(spacer(4.0));
            v.push(button_view("↻ preset (catálogo)", &btn, Msg::LeyPreset));
            v.push(spacer(4.0));
            v.push(button_view(
                if model.leyes_node_mode { "▦ vista: grafo → laboratorio" } else { "▦ vista: laboratorio → grafo" },
                &btn,
                Msg::ToggleLeyesView,
            ));
            v.push(spacer(6.0));
            v.push(button_view(
                if model.simulating { "⏸ pausar" } else { "▶ simular" },
                &btn,
                Msg::ToggleSim,
            ));
            v.push(spacer(4.0));
            v.push(button_view("⟲ resembrar", &btn, Msg::ReseedLab));
            if campos.len() > 1 {
                let vis = model.lab.as_ref().map(|x| x.vis).unwrap_or(0);
                let vname = campos.get(vis).map(|c| c.name.as_str()).unwrap_or("?");
                v.push(spacer(4.0));
                v.push(button_view(format!("ver campo: {vname}"), &btn, Msg::CycleLabField));
            }
            // Error de compilación, si la fórmula no parsea.
            if let Some(Err(err)) = l.kind.compile_ecuacion() {
                v.push(spacer(6.0));
                v.push(body_text(format!("⚠ {err}"), Color::from_rgba8(232, 128, 96, 255), theme));
            }
            v.push(spacer(8.0));
            v.push(section_title("ECUACIONES  (Δcampo/dt)", theme));
            for (i, campo) in campos.iter().enumerate() {
                v.push(body_text(format!("Δ{}/dt =", campo.name), theme.fg_muted, theme));
                let editing = model.formula_focused && model.formula_target == i;
                if editing {
                    v.push(text_input_view(
                        &model.formula_input,
                        "fórmula…",
                        true,
                        &TextInputPalette::from_theme(theme),
                        Msg::FormulaFocus(i),
                    ));
                } else {
                    let src = fuentes.get(i).cloned().unwrap_or_default();
                    let label = if src.is_empty() { "(tocar para editar)".to_string() } else { src };
                    v.push(button_view(label, &btn, Msg::FormulaFocus(i)));
                }
                v.push(spacer(4.0));
            }
            v.push(button_view("+ campo", &btn, Msg::AddCampo));
            if campos.len() > 1 {
                v.push(spacer(4.0));
                v.push(button_view("− campo", &btn, Msg::RemoveCampo));
            }
            v.push(spacer(8.0));
            v.push(section_title("PARÁMETROS", theme));
            for (i, (name, value, min, max)) in l.kind.ecuacion_params().into_iter().enumerate() {
                v.push(slider_view(name, value, min, max, &sp, move |_p, dv| {
                    Some(Msg::SetEcuParam(i, value + dv))
                }));
            }
            v.push(spacer(4.0));
            v.push(button_view("+ parámetro", &btn, Msg::AddEcuParam));
            v.push(spacer(4.0));
            v.push(button_view("− parámetro", &btn, Msg::RemoveEcuParam));
            v.push(spacer(8.0));
            v.push(body_text(
                "términos: lap(c) · avg(c) · min6(c)/max6(c)/sum6(c) · abajo(c)… · clamp/min/max · < > · dt".into(),
                theme.fg_placeholder,
                theme,
            ));
            // Paleta de nodos (sólo en modo grafo): tocar = agregar un nodo suelto.
            if model.leyes_node_mode {
                v.push(spacer(8.0));
                v.push(section_title("NODOS  (tocar = agregar)", theme));
                v.push(button_view("const 1", &btn, Msg::GraphAddNode(NodeOp::Const(1.0))));
                v.push(button_view("dt", &btn, Msg::GraphAddNode(NodeOp::Dt)));
                for (f, c) in campos.iter().enumerate() {
                    let f = f as u16;
                    v.push(button_view(format!("campo {}", c.name), &btn, Msg::GraphAddNode(NodeOp::Field(f))));
                    v.push(button_view(format!("lap {}", c.name), &btn, Msg::GraphAddNode(NodeOp::Lap(f))));
                    v.push(button_view(format!("avg {}", c.name), &btn, Msg::GraphAddNode(NodeOp::Vecinos(Reduce::Promedio, f))));
                }
                for (p, pd) in params.iter().enumerate() {
                    v.push(button_view(format!("param {}", pd.name), &btn, Msg::GraphAddNode(NodeOp::Param(p as u16))));
                }
                for (lbl, op) in [
                    ("+", NodeOp::Bin(BinOp::Add)),
                    ("−", NodeOp::Bin(BinOp::Sub)),
                    ("×", NodeOp::Bin(BinOp::Mul)),
                    ("÷", NodeOp::Bin(BinOp::Div)),
                    ("min", NodeOp::Bin(BinOp::Min)),
                    ("max", NodeOp::Bin(BinOp::Max)),
                    ("<", NodeOp::Bin(BinOp::Lt)),
                    (">", NodeOp::Bin(BinOp::Gt)),
                    ("abs", NodeOp::Un(UnOp::Abs)),
                    ("−x", NodeOp::Un(UnOp::Neg)),
                    ("clamp", NodeOp::Clamp),
                ] {
                    v.push(button_view(lbl, &btn, Msg::GraphAddNode(op)));
                }
                v.push(spacer(4.0));
                v.push(body_text(
                    "arrastrá salida→entrada para conectar · right‑click borra un nodo".into(),
                    theme.fg_placeholder,
                    theme,
                ));
            }
        }
        // --- Leyes nativas (Fluir/Crecer): sliders cableados -------------------
        _ => {
            v.push(spacer(6.0));
            v.push(section_title("PARÁMETROS", theme));
            if l.kind.params().is_empty() {
                v.push(body_text("este tipo no tiene parámetros".into(), theme.fg_placeholder, theme));
            }
            for (i, (name, value, min, max)) in l.kind.params().into_iter().enumerate() {
                v.push(slider_view(name, value, min, max, &sp, move |_p, dv| {
                    Some(Msg::SetLeyParam(i, value + dv))
                }));
            }
        }
    }
    v
}

fn material_editor(model: &Model, tab: usize) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let sp = SliderPalette::from_theme(theme);
    let btn = ButtonPalette::from_theme(theme);
    let Some(m) = model.project.material(model.sel[1]) else {
        return vec![section_title("sin material — creá uno", theme)];
    };
    if tab == 0 {
        let resolved = model.project.resolve_material(m.id);
        let parent_name = m
            .parent
            .and_then(|pid| model.project.material(pid))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "— ninguno".into());
        let rgb = m.color.unwrap_or([
            resolved.color[0] as f32 / 255.0,
            resolved.color[1] as f32 / 255.0,
            resolved.color[2] as f32 / 255.0,
        ]);
        let grain = m.grain.unwrap_or(resolved.grain);
        let mut v = vec![
            section_title("ASPECTO", theme),
            button_view(format!("rol: {}", m.role.label()), &btn, Msg::CycleMatRole),
            spacer(4.0),
            button_view(format!("padre: {parent_name}"), &btn, Msg::CycleMatParent),
            spacer(6.0),
            section_title("COLOR", theme),
        ];
        for (ch, label) in [(0usize, "rojo"), (1, "verde"), (2, "azul")] {
            let val = rgb[ch];
            v.push(slider_view(label, val, 0.0, 1.0, &sp, move |_p, dv| Some(Msg::SetMatColor(ch, val + dv))));
        }
        v.push(slider_view("grano", grain, 0.0, 1.0, &sp, move |_p, dv| Some(Msg::SetMatGrain(grain + dv))));
        v
    } else {
        let mut v = vec![section_title("LEYES DEL MATERIAL", theme)];
        for u in &m.leyes {
            let name = model.project.ley(u.ley).map(|l| l.name.clone()).unwrap_or_else(|| "?".into());
            v.push(body_text(format!("· {name}"), theme.fg_text, theme));
        }
        if m.leyes.is_empty() {
            v.push(body_text("sin leyes aplicadas".into(), theme.fg_placeholder, theme));
        }
        v.push(spacer(6.0));
        v.push(button_view("+ agregar ley", &btn, Msg::AddMatLey));
        v.push(spacer(4.0));
        v.push(button_view("− quitar última", &btn, Msg::RemoveMatLey));
        v
    }
}

fn sere_editor(model: &Model, tab: usize) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let sp = SliderPalette::from_theme(theme);
    let btn = ButtonPalette::from_theme(theme);
    let Some(c) = sel_sere(model) else {
        return vec![section_title("sin ser — creá o generá uno", theme)];
    };
    match tab {
        0 => vec![
            section_title("CUERPO", theme),
            button_view(format!("forma: {}", c.cuerpo_label()), &btn, Msg::CycleSereCuerpo),
            spacer(4.0),
            button_view(format!("edad: {}", c.age.label()), &btn, Msg::CycleSereAge),
        ],
        1 => andar_tools(model, c),
        2 => conducta_tools(model, c),
        3 => color_tools("PIEL", Part::Skin, c.skin, &sp, theme),
        4 => color_tools("CAMISETA", Part::Shirt, c.shirt, &sp, theme),
        _ => color_tools("PANTALÓN", Part::Pants, c.pants, &sp, theme),
    }
}

/// Editor de la **conducta** (capa 3) de un ser: sliders de locomoción + toggle para
/// soltar una manada viva en el preview.
fn conducta_tools(model: &Model, c: &llimphi_voxel::CharSpec) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let sp = SliderPalette::from_theme(theme);
    let btn = ButtonPalette::from_theme(theme);
    let mut v = vec![section_title("CONDUCTA", theme)];
    for (i, (name, value, min, max)) in c.conducta.params().into_iter().enumerate() {
        v.push(slider_view(name, value, min, max, &sp, move |_p, dv| Some(Msg::SetConducta(i, value + dv))));
    }
    v.push(spacer(8.0));
    v.push(section_title("PREVIEW", theme));
    v.push(button_view(
        if model.manada { "⏸ detener manada" } else { "▶ soltar manada" },
        &btn,
        Msg::ToggleManada,
    ));
    v.push(spacer(4.0));
    v.push(body_text("una manada del ser deambula y se junta según su conducta".into(), theme.fg_placeholder, theme));
    v
}

/// Editor del **andar** (capa 2) de un ser-rig: estado a editar/previsualizar +
/// cadencia + cuánto balancea cada articulación. El humanoide usa animaciones fijas.
fn andar_tools<'a>(model: &Model, c: &'a llimphi_voxel::CharSpec) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let sp = SliderPalette::from_theme(theme);
    let btn = ButtonPalette::from_theme(theme);
    let Some(mov) = &c.cuerpo else {
        return vec![
            section_title("ANDAR", theme),
            body_text(
                "el humanoide usa animaciones fijas — cambiá la «forma» a un rig (cuadrúpedo, ave…) para editar su andar".into(),
                theme.fg_placeholder,
                theme,
            ),
        ];
    };
    let est = model.andar_estado.min(2);
    let andar = mov.andares.estado(est);
    let mut v = vec![
        section_title("ANDAR", theme),
        button_view(
            format!("estado: {}", llimphi_voxel::Andares::LABELS[est]),
            &btn,
            Msg::CycleAndarEstado,
        ),
        spacer(4.0),
        slider_view("cadencia (ritmo)", andar.cadencia, 0.0, 18.0, &sp, {
            let cad = andar.cadencia;
            move |_p, dv| Some(Msg::SetAndarCadencia(cad + dv))
        }),
        spacer(6.0),
        section_title("BALANCEO POR PARTE", theme),
    ];
    for (i, seg) in mov.rig.segmentos.iter().enumerate() {
        let amp = andar.osc.get(i).map(|o| o.amplitud).unwrap_or(0.0);
        v.push(slider_view(&seg.nombre, amp, 0.0, 1.5, &sp, move |_p, dv| {
            Some(Msg::SetAndarAmplitud(i, amp + dv))
        }));
    }
    v
}

fn bioma_editor(model: &Model, tab: usize) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let sp = SliderPalette::from_theme(theme);
    let btn = ButtonPalette::from_theme(theme);
    let Some(b) = model.project.bioma(model.sel[3]) else {
        return vec![section_title("sin bioma — creá uno", theme)];
    };
    let mat_name = |id: u64| model.project.material(id).map(|m| m.name.clone()).unwrap_or_else(|| "—".into());
    match tab {
        0 => vec![
            section_title("RELIEVE", theme),
            bslider(&sp, "base (llanura)", b.base, 0.0, 0.9, BiomaField::Base),
            bslider(&sp, "dunas", b.dune, 0.0, 0.4, BiomaField::Dune),
            bslider(&sp, "relieve (alto)", b.relief, 0.0, 1.0, BiomaField::Relief),
            bslider(&sp, "densidad montañas", b.mountains, 0.0, 1.0, BiomaField::Mountains),
            bslider(&sp, "nivel del agua", b.water_level, 0.0, 0.9, BiomaField::Water),
            bslider(&sp, "densidad ríos", b.rivers, 0.0, 1.0, BiomaField::Rivers),
            bslider(&sp, "altura cumbre", b.peak_at, 0.0, 1.0, BiomaField::PeakAt),
            spacer(8.0),
            section_title("SIMULAR LEYES", theme),
            button_view(if model.simulating { "⏸ detener" } else { "▶ simular (agua · plantas)" }, &btn, Msg::ToggleSim),
        ],
        1 => vec![
            section_title("MATERIALES", theme),
            button_view(format!("suelo: {}", mat_name(b.ground)), &btn, Msg::CycleBiomaGround),
            spacer(4.0),
            button_view(format!("acantilado: {}", mat_name(b.cliff)), &btn, Msg::CycleBiomaCliff),
            spacer(4.0),
            button_view(
                format!("cumbre: {}", b.peak.map(mat_name).unwrap_or_else(|| "ninguna".into())),
                &btn,
                Msg::CycleBiomaPeak,
            ),
        ],
        2 => {
            let mut v = vec![section_title("OBJETOS", theme)];
            for (i, o) in b.objetos.iter().enumerate() {
                let name = mat_name(o.material);
                v.push(body_text(format!("· {name}"), theme.fg_text, theme));
                let d = o.densidad;
                v.push(slider_view("densidad", d, 0.0, 0.05, &sp, move |_p, dv| Some(Msg::SetObjetoDensidad(i, d + dv))));
            }
            if b.objetos.is_empty() {
                v.push(body_text("sin objetos".into(), theme.fg_placeholder, theme));
            }
            v.push(spacer(6.0));
            v.push(button_view("+ objeto", &btn, Msg::AddBiomaObjeto));
            v.push(spacer(4.0));
            v.push(button_view("− quitar", &btn, Msg::RemoveBiomaObjeto));
            v
        }
        _ => {
            let mut v = vec![section_title("SERES (POBLACIÓN)", theme)];
            for (i, u) in b.seres.iter().enumerate() {
                let name = model.project.sere(u.sere).map(|s| s.name.clone()).unwrap_or_else(|| "—".into());
                v.push(button_view(format!("ser: {name}"), &btn, Msg::CycleBiomaSere(i)));
                let prob = u.probabilidad;
                v.push(slider_view("cantidad", prob, 0.0, 1.0, &sp, move |_p, dv| {
                    Some(Msg::SetBiomaSereProb(i, prob + dv))
                }));
                v.push(spacer(4.0));
            }
            if b.seres.is_empty() {
                v.push(body_text("sin seres — agregá para poblar el bioma".into(), theme.fg_placeholder, theme));
            }
            v.push(spacer(4.0));
            v.push(button_view("+ ser", &btn, Msg::AddBiomaSere));
            v.push(spacer(4.0));
            v.push(button_view("− quitar", &btn, Msg::RemoveBiomaSere));
            v.push(spacer(8.0));
            v.push(body_text("se ven con «▶ simular» en Mundos/Biomas".into(), theme.fg_placeholder, theme));
            v
        }
    }
}

fn mundo_editor(model: &Model, tab: usize) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let btn = ButtonPalette::from_theme(theme);
    let Some(m) = model.project.mundo(model.sel[4]) else {
        return vec![section_title("sin mundo — creá uno", theme)];
    };
    if tab == 0 {
        vec![
            section_title("SEMILLA", theme),
            text_input_view(
                &model.seed_input,
                "número…",
                model.seed_focused,
                &TextInputPalette::from_theme(theme),
                Msg::SeedFocus,
            ),
            spacer(6.0),
            button_view("🎲 random", &btn, Msg::SeedRandom),
            spacer(8.0),
            body_text(format!("semilla actual: {}", m.seed), theme.fg_muted, theme),
            spacer(10.0),
            section_title("SIMULAR LEYES", theme),
            button_view(if model.simulating { "⏸ detener" } else { "▶ simular (agua · plantas)" }, &btn, Msg::ToggleSim),
        ]
    } else {
        let bname = m
            .biomas
            .first()
            .and_then(|&id| model.project.bioma(id))
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "—".into());
        vec![
            section_title("BIOMAS", theme),
            button_view(format!("bioma: {bname}"), &btn, Msg::CycleMundoBioma),
            spacer(8.0),
            body_text("un mundo se compone de uno o más biomas".into(), theme.fg_placeholder, theme),
        ]
    }
}

fn scene_editor(model: &Model, tab: usize) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let sp = SliderPalette::from_theme(theme);
    let btn = ButtonPalette::from_theme(theme);
    let Some(s) = sel_scene(model) else {
        return vec![section_title("sin escena — generá una con IA", theme)];
    };
    match tab {
        0 => {
            let dur = s.duration;
            let mundo_name = model.project.mundo(s.mundo).map(|m| m.name.clone()).unwrap_or_else(|| "—".into());
            let t = model.time;
            vec![
                section_title("ESCENA", theme),
                button_view(format!("mundo: {mundo_name}"), &btn, Msg::CycleSceneMundo),
                spacer(6.0),
                slider_view("duración (s)", dur, 1.0, 20.0, &sp, move |_p, dv| Some(Msg::SetSceneDur(dur + dv))),
                spacer(6.0),
                button_view(if model.playing { "❚❚ pausa" } else { "▶ reproducir" }, &btn, Msg::TogglePlay),
                spacer(4.0),
                slider_view("tiempo", t.min(dur), 0.0, dur, &sp, move |_p, dv| Some(Msg::Scrub(t + dv))),
                spacer(6.0),
                body_text(format!("{} actores", s.actors.len()), theme.fg_muted, theme),
            ]
        }
        1 => reparto_tools(model, s),
        2 => {
            let cam_label = if model.script_cam { "cámara: guion" } else { "cámara: órbita" };
            vec![
                section_title("CÁMARA", theme),
                button_view(cam_label, &btn, Msg::ToggleSceneCam),
                spacer(4.0),
                body_text(format!("{} planos", s.shots.len()), theme.fg_muted, theme),
                spacer(4.0),
                button_view("+ plano acá", &btn, Msg::AddShot),
                spacer(4.0),
                button_view("− quitar plano", &btn, Msg::RemoveShot),
            ]
        }
        _ => vec![
            section_title("VIDEO", theme),
            button_view(if model.exporting { "exportando…" } else { "🎬 exportar video" }, &btn, Msg::ExportVideo),
            spacer(8.0),
            body_text("renderiza el guion a un .mkv (puede tardar)".into(), theme.fg_placeholder, theme),
        ],
    }
}

/// **Reparto** (filmografía): dirigir quién actúa y su recorrido (waypoints).
fn reparto_tools(model: &Model, s: &SceneSpec) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let sp = SliderPalette::from_theme(theme);
    let btn = ButtonPalette::from_theme(theme);
    let dimx = world_dim(PREVIEW_DIM_XZ)[0] as f32;
    let a = model.actor_sel.min(s.actors.len().saturating_sub(1));

    let mut v = vec![section_title("REPARTO", theme)];
    for (i, act) in s.actors.iter().enumerate() {
        let nombre = model.project.seres.get(act.character).map(|c| c.name.clone()).unwrap_or_else(|| "—".into());
        let label = format!("{nombre} · {} claves", act.keys.len());
        v.push(selectable_row(label, i == a, Msg::SelectActor(i), theme));
    }
    v.push(spacer(6.0));
    v.push(crud_pair("+ actor", Msg::AddActor, "− actor", Msg::RemoveActor, &btn));

    let Some(act) = s.actors.get(a) else { return v };
    let nombre = model.project.seres.get(act.character).map(|c| c.name.clone()).unwrap_or_else(|| "—".into());
    v.push(spacer(8.0));
    v.push(button_view(format!("actúa: {nombre}"), &btn, Msg::CycleActorSere));

    // Recorrido: las claves (waypoints) del actor seleccionado.
    v.push(spacer(8.0));
    v.push(section_title("RECORRIDO", theme));
    let k = model.key_sel.min(act.keys.len().saturating_sub(1));
    for (i, key) in act.keys.iter().enumerate() {
        let clip = key.clip.unwrap_or(Clip::Idle).label();
        v.push(selectable_row(format!("t={:.1}s · {clip}", key.t), i == k, Msg::SelectKey(i), theme));
    }
    v.push(spacer(4.0));
    v.push(crud_pair("+ clave", Msg::AddKey, "− clave", Msg::RemoveKey, &btn));

    if let Some(key) = act.keys.get(k) {
        v.push(spacer(8.0));
        v.push(section_title("CLAVE", theme));
        let (gx, gz, kt) = (key.gx, key.gz, key.t);
        v.push(slider_view("posición X", gx, 0.0, dimx, &sp, move |_p, dv| Some(Msg::SetKeyPos(true, gx + dv))));
        v.push(slider_view("posición Z", gz, 0.0, dimx, &sp, move |_p, dv| Some(Msg::SetKeyPos(false, gz + dv))));
        v.push(slider_view("tiempo (s)", kt, 0.0, s.duration, &sp, move |_p, dv| Some(Msg::SetKeyTime(kt + dv))));
        v.push(spacer(4.0));
        v.push(button_view(format!("acción: {}", key.clip.unwrap_or(Clip::Idle).label()), &btn, Msg::CycleKeyClip));
    }
    v.push(spacer(8.0));
    v.push(body_text("dale ▶ reproducir en «Escena» para ver el recorrido".into(), theme.fg_placeholder, theme));
    v
}

/// Par de botones en una fila (p.ej. agregar/quitar).
fn crud_pair(a: &str, ma: Msg, b: &str, mb: Msg, btn: &ButtonPalette) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![cell(button_view(a, btn, ma)), cell(button_view(b, btn, mb))])
}

/// Slider de un campo de relieve del bioma.
fn bslider(sp: &SliderPalette, label: &str, value: f32, min: f32, max: f32, field: BiomaField) -> View<Msg> {
    slider_view(label, value, min, max, sp, move |_p, dv| Some(Msg::SetBiomaField(field, value + dv)))
}

/// Set de sliders R/G/B de una parte coloreable de un ser.
fn color_tools(title: &str, part: Part, rgb: [f32; 3], sp: &SliderPalette, theme: &Theme) -> Vec<View<Msg>> {
    let mut v = vec![section_title(title, theme)];
    for (ch, label) in [(0usize, "rojo"), (1, "verde"), (2, "azul")] {
        let value = rgb[ch];
        v.push(slider_view(label, value, 0.0, 1.0, sp, move |_p, dv| Some(Msg::SetSereColor(part, ch, value + dv))));
    }
    v
}

// =============================================================================
//  CRUD: crear / duplicar / borrar / renombrar / reseleccionar
// =============================================================================

/// Crea un item nuevo en el nivel activo y devuelve su id.
fn new_item(model: &mut Model) -> u64 {
    let id = model.project.alloc_id();
    match model.level {
        Level::Leyes => model.project.leyes.push(llimphi_voxel::Ley {
            id,
            name: format!("ley {id}"),
            kind: LeyKind::Fluir { gravedad: 1.0, horizontal: 0.6 },
        }),
        Level::Materiales => model.project.materiales.push(MaterialDef {
            id,
            name: format!("material {id}"),
            parent: None,
            role: MatRole::Terreno,
            color: Some([0.6, 0.6, 0.6]),
            grain: Some(0.4),
            leyes: vec![],
            builtin: None,
        }),
        Level::Seres => {
            let mut c = CharSpec::new(format!("ser {id}"), Age::Adult);
            c.id = id;
            model.project.seres.push(c);
        }
        Level::Biomas => model.project.biomas.push(default_bioma(model, id)),
        Level::Mundos => {
            let bioma = model.project.biomas.first().map(|b| b.id).unwrap_or(0);
            model.project.mundos.push(Mundo { id, name: format!("mundo {id}"), seed: 1, biomas: vec![bioma] });
        }
        Level::Escenas => {
            let mundo = model.project.mundos.first().map(|m| m.id).unwrap_or(0);
            let dim = world_dim(PREVIEW_DIM_XZ);
            let mut s = SceneSpec::walk_and_emote(format!("escena {id}"), mundo, 3, Clip::Wave, dim);
            s.id = id;
            model.project.escenas.push(s);
        }
    }
    id
}

/// Un bioma por defecto que referencia los materiales del proyecto (o 0).
fn default_bioma(model: &Model, id: u64) -> Bioma {
    let ground = model.project.material_id_for(Material::Grass).unwrap_or(0);
    let cliff = model.project.material_id_for(Material::Rock).unwrap_or(0);
    Bioma {
        id,
        name: format!("bioma {id}"),
        base: 0.22,
        dune: 0.10,
        relief: 0.6,
        mountains: 0.4,
        water_level: 0.30,
        rivers: 0.2,
        peak_at: 0.8,
        ground,
        cliff,
        peak: None,
        objetos: vec![],
        seres: vec![],
    }
}

/// Duplica el item seleccionado (copia con id nuevo) y devuelve su id.
fn duplicate_item(model: &mut Model) -> Option<u64> {
    let sel = model.selected();
    let id = model.project.alloc_id();
    match model.level {
        Level::Leyes => {
            let mut x = model.project.ley(sel)?.clone();
            x.id = id;
            x.name = format!("{} copia", x.name);
            model.project.leyes.push(x);
        }
        Level::Materiales => {
            let mut x = model.project.material(sel)?.clone();
            x.id = id;
            x.name = format!("{} copia", x.name);
            x.builtin = None; // una copia editable, no la de fábrica
            model.project.materiales.push(x);
        }
        Level::Seres => {
            let mut x = sel_sere(model)?.clone();
            x.id = id;
            x.name = format!("{} copia", x.name);
            model.project.seres.push(x);
        }
        Level::Biomas => {
            let mut x = model.project.bioma(sel)?.clone();
            x.id = id;
            x.name = format!("{} copia", x.name);
            model.project.biomas.push(x);
        }
        Level::Mundos => {
            let mut x = model.project.mundo(sel)?.clone();
            x.id = id;
            x.name = format!("{} copia", x.name);
            model.project.mundos.push(x);
        }
        Level::Escenas => {
            let mut x = sel_scene(model)?.clone();
            x.id = id;
            x.name = format!("{} copia", x.name);
            model.project.escenas.push(x);
        }
    }
    Some(id)
}

/// Borra el item seleccionado del nivel activo.
fn delete_selected(model: &mut Model) {
    let sel = model.selected();
    match model.level {
        Level::Leyes => model.project.leyes.retain(|x| x.id != sel),
        Level::Materiales => model.project.materiales.retain(|x| x.id != sel),
        Level::Seres => model.project.seres.retain(|x| x.id != sel),
        Level::Biomas => model.project.biomas.retain(|x| x.id != sel),
        Level::Mundos => model.project.mundos.retain(|x| x.id != sel),
        Level::Escenas => model.project.escenas.retain(|x| x.id != sel),
    }
}

/// Fija el nombre del item seleccionado.
fn set_selected_name(model: &mut Model, name: String) {
    let sel = model.selected();
    match model.level {
        Level::Leyes => {
            if let Some(x) = model.project.leyes.iter_mut().find(|x| x.id == sel) {
                x.name = name;
            }
        }
        Level::Materiales => {
            if let Some(x) = model.project.materiales.iter_mut().find(|x| x.id == sel) {
                x.name = name;
            }
        }
        Level::Seres => {
            if let Some(x) = model.project.seres.iter_mut().find(|x| x.id == sel) {
                x.name = name;
            }
        }
        Level::Biomas => {
            if let Some(x) = model.project.biomas.iter_mut().find(|x| x.id == sel) {
                x.name = name;
            }
        }
        Level::Mundos => {
            if let Some(x) = model.project.mundos.iter_mut().find(|x| x.id == sel) {
                x.name = name;
            }
        }
        Level::Escenas => {
            if let Some(x) = model.project.escenas.iter_mut().find(|x| x.id == sel) {
                x.name = name;
            }
        }
    }
}

/// Recarga `name_input`/`seed_input` desde el item seleccionado.
fn sync_inputs(model: &mut Model) {
    model.name_input.set_text(&model.selected_name());
    if model.level == Level::Mundos {
        let s = model.project.mundo(model.selected()).map(|m| m.seed).unwrap_or(0);
        model.seed_input.set_text(&s.to_string());
    }
}

/// Reselecciona el primer item de cada nivel (tras cargar un proyecto).
fn reselect_all(model: &mut Model) {
    model.sel[0] = model.project.leyes.first().map(|x| x.id).unwrap_or(0);
    model.sel[1] = model.project.materiales.first().map(|x| x.id).unwrap_or(0);
    model.sel[2] = model.project.seres.first().map(|x| x.id).unwrap_or(0);
    model.sel[3] = model.project.biomas.first().map(|x| x.id).unwrap_or(0);
    model.sel[4] = model.project.mundos.first().map(|x| x.id).unwrap_or(0);
    model.sel[5] = model.project.escenas.first().map(|x| x.id).unwrap_or(0);
}

// --- Accesores al item seleccionado (mut e inmut) ---
fn sel_material_mut(model: &mut Model) -> Option<&mut MaterialDef> {
    let id = model.sel[1];
    model.project.materiales.iter_mut().find(|x| x.id == id)
}
fn sel_ley(model: &Model) -> Option<&llimphi_voxel::Ley> {
    model.project.ley(model.sel[0])
}
fn sel_ley_mut(model: &mut Model) -> Option<&mut llimphi_voxel::Ley> {
    let id = model.sel[0];
    model.project.leyes.iter_mut().find(|x| x.id == id)
}

/// Catálogo de **leyes de fábrica** por ecuación (autoradas, editables). Ciclar
/// reemplaza la `Ecuacion` actual por la siguiente del catálogo. Todas verificadas
/// como comportamientos vivos por los tests de `llimphi-voxel`.
fn law_presets() -> Vec<(&'static str, LeyKind)> {
    vec![
        ("difusión", LeyKind::ecuacion_default()),
        (
            "reacción-difusión",
            LeyKind::Ecuacion {
                campos: vec![
                    FieldDef::new("u", 1.0, 0.0, 1.0),
                    FieldDef::new("v", 0.0, 0.0, 1.0),
                ],
                params: vec![
                    ParamDef::new("Du", 0.16, 0.0, 1.0),
                    ParamDef::new("Dv", 0.08, 0.0, 1.0),
                    ParamDef::new("F", 0.06, 0.0, 0.1),
                    ParamDef::new("k", 0.062, 0.0, 0.1),
                ],
                fuentes: vec![
                    "Du * lap(u) - u * v * v + F * (1 - u)".into(),
                    "Dv * lap(v) + u * v * v - (F + k) * v".into(),
                ],
            },
        ),
        (
            "calor",
            LeyKind::Ecuacion {
                campos: vec![FieldDef::new("t", 0.0, 0.0, 1.0)],
                params: vec![ParamDef::new("difus", 0.2, 0.0, 0.25)],
                fuentes: vec!["difus * lap(t)".into()],
            },
        ),
        (
            "crecer",
            LeyKind::Ecuacion {
                campos: vec![FieldDef::new("h", 0.0, 0.0, 1.0)],
                params: vec![ParamDef::new("vel", 0.05, 0.0, 0.3), ParamDef::new("tope", 0.8, 0.0, 1.0)],
                fuentes: vec!["vel * (h < tope)".into()],
            },
        ),
    ]
}

/// La siguiente ley del catálogo tras la actual (por label del preset actual).
fn next_preset(actual: &LeyKind) -> LeyKind {
    let presets = law_presets();
    // Encuentra el preset cuya estructura de campos coincide con la actual; si no,
    // arranca del primero. Ciclo simple por índice.
    let idx = presets
        .iter()
        .position(|(_, k)| leyes_misma_forma(k, actual))
        .map(|i| (i + 1) % presets.len())
        .unwrap_or(0);
    presets[idx].1.clone()
}

/// Dos leyes `Ecuacion` "tienen la misma forma" si coinciden los nombres de campos
/// (heurística para ubicar el preset actual al ciclar).
fn leyes_misma_forma(a: &LeyKind, b: &LeyKind) -> bool {
    match (a, b) {
        (LeyKind::Ecuacion { campos: ca, .. }, LeyKind::Ecuacion { campos: cb, .. }) => {
            ca.len() == cb.len() && ca.iter().zip(cb).all(|(x, y)| x.name == y.name)
        }
        _ => false,
    }
}
fn sel_sere<'a>(model: &'a Model) -> Option<&'a CharSpec> {
    let id = model.sel[2];
    model.project.seres.iter().find(|x| x.id == id)
}
fn sel_sere_mut(model: &mut Model) -> Option<&mut CharSpec> {
    let id = model.sel[2];
    model.project.seres.iter_mut().find(|x| x.id == id)
}
fn sel_bioma_mut(model: &mut Model) -> Option<&mut Bioma> {
    let id = model.sel[3];
    model.project.biomas.iter_mut().find(|x| x.id == id)
}
fn sel_mundo_mut(model: &mut Model) -> Option<&mut Mundo> {
    let id = model.sel[4];
    model.project.mundos.iter_mut().find(|x| x.id == id)
}
fn sel_scene<'a>(model: &'a Model) -> Option<&'a SceneSpec> {
    let id = model.sel[5];
    model.project.escenas.iter().find(|x| x.id == id)
}
fn sel_scene_mut(model: &mut Model) -> Option<&mut SceneSpec> {
    let id = model.sel[5];
    model.project.escenas.iter_mut().find(|x| x.id == id)
}

// --- Listas de ids para los botones de ciclo ---
fn terreno_ids(model: &Model) -> Vec<u64> {
    model.project.materiales.iter().map(|m| m.id).collect()
}
fn objeto_ids(model: &Model) -> Vec<u64> {
    let objs: Vec<u64> = model
        .project
        .materiales
        .iter()
        .filter(|m| matches!(m.role, MatRole::Objeto(_)))
        .map(|m| m.id)
        .collect();
    if objs.is_empty() {
        terreno_ids(model)
    } else {
        objs
    }
}
fn bioma_ids(model: &Model) -> Vec<u64> {
    model.project.biomas.iter().map(|b| b.id).collect()
}
fn sere_ids(model: &Model) -> Vec<u64> {
    model.project.seres.iter().map(|s| s.id).collect()
}

/// Reparte los tiempos de las claves de un actor uniformemente en `[0, dur]`, así el
/// recorrido se recorre a lo largo de toda la escena (el director interpola entre ellas).
fn respread(actor: &mut ActorSpec, dur: f32) {
    let n = actor.keys.len();
    for (i, k) in actor.keys.iter_mut().enumerate() {
        k.t = if n <= 1 { 0.0 } else { i as f32 / (n - 1) as f32 * dur };
    }
}
fn mundo_ids(model: &Model) -> Vec<u64> {
    model.project.mundos.iter().map(|m| m.id).collect()
}

/// El siguiente id de `ids` después de `cur` (cicla); `cur` si la lista está vacía.
fn next_in(ids: &[u64], cur: u64) -> u64 {
    if ids.is_empty() {
        return cur;
    }
    let i = ids.iter().position(|&x| x == cur).unwrap_or(usize::MAX);
    ids[(i.wrapping_add(1)) % ids.len()]
}

/// Cicla un `Option<u64>` por `None → ids[0] → … → None`.
fn next_option(cur: Option<u64>, ids: &[u64]) -> Option<u64> {
    match cur {
        None => ids.first().copied(),
        Some(c) => {
            let i = ids.iter().position(|&x| x == c);
            match i {
                Some(i) if i + 1 < ids.len() => Some(ids[i + 1]),
                _ => None,
            }
        }
    }
}

// =============================================================================
//  Helpers de layout
// =============================================================================

fn selectable_row(label: String, selected: bool, msg: Msg, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        padding: pad(10.0, 0.0),
        ..Default::default()
    })
    .fill(if selected { theme.bg_selected } else { theme.bg_panel })
    .radius(6.0)
    .text(label, 15.0, if selected { theme.fg_text } else { theme.fg_muted })
    .on_click(msg)
}

fn section_title(text: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        ..Default::default()
    })
    .text(text.to_string(), 12.0, theme.accent)
    .bold()
}

fn body_text(s: String, color: Color, _theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        ..Default::default()
    })
    .text(s, 13.0, color)
    .max_lines(2)
}

fn spacer(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(h) },
        ..Default::default()
    })
}

fn pad(
    x: f32,
    y: f32,
) -> llimphi_ui::llimphi_layout::taffy::prelude::Rect<llimphi_ui::llimphi_layout::taffy::prelude::LengthPercentage> {
    use llimphi_ui::llimphi_layout::taffy::prelude::Rect;
    Rect { left: length(x), right: length(x), top: length(y), bottom: length(y) }
}

fn gap_y(h: f32) -> Size<llimphi_ui::llimphi_layout::taffy::prelude::LengthPercentage> {
    Size { width: length(0.0_f32), height: length(h) }
}

// =============================================================================
//  Persistencia RON
// =============================================================================

fn save_project(project: &Project) -> Result<(), String> {
    let s = ron::ser::to_string_pretty(project, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())?;
    std::fs::write(PROJECT_PATH, s).map_err(|e| e.to_string())
}

fn load_project() -> Result<Project, String> {
    let s = std::fs::read_to_string(PROJECT_PATH).map_err(|e| e.to_string())?;
    ron::from_str(&s).map_err(|e| e.to_string())
}

/// Estado de arranque (sin `Handle`) — reusado por `init` y por el pantallazo.
pub(crate) fn demo_model() -> Model {
    let project = Project::starter();
    let mut model = Model {
        theme: Theme::dark(),
        level: Level::Mundos,
        sel: [0; 6],
        tool_tab: 0,
        gen: 1,
        yaw: 35_f32.to_radians(),
        pitch: 26_f32.to_radians(),
        dist: default_dist(),
        preview: Arc::new(Mutex::new(None)),
        status: "creador de mundos por niveles".into(),
        name_input: TextInputState::new(),
        name_focused: false,
        seed_input: TextInputState::new(),
        seed_focused: false,
        ai_input: TextInputState::new(),
        ai_focused: false,
        ai_busy: false,
        time: 0.0,
        playing: false,
        script_cam: true,
        exporting: false,
        simulating: false,
        andar_estado: 1, // caminar
        manada: false,
        actor_sel: 0,
        key_sel: 0,
        rng: 0x1234_5678,
        dientes_outside: wawa_config::WawaConfig::load().dientes_outside,
        lab: None,
        lab_gen: 1,
        formula_input: TextInputState::new(),
        formula_focused: false,
        formula_target: 0,
        leyes_node_mode: false,
        eq_graph: None,
        eq_graph_gen: 0,
        project,
    };
    reselect_all(&mut model);
    sync_inputs(&mut model);
    model
}

fn main() {
    if std::env::args().any(|a| a == "--shot") {
        shot::shot();
        return;
    }
    if std::env::args().any(|a| a == "--turntable") {
        let p = Project::starter();
        let mr = p
            .mundos
            .first()
            .and_then(|m| p.render_mundo(m.id))
            .unwrap_or_else(fallback_render);
        match render::turntable(&mr) {
            Ok(out) => eprintln!("turntable ok: {out}"),
            Err(e) => eprintln!("turntable error: {e}"),
        }
        return;
    }
    if std::env::args().any(|a| a == "--flythrough") {
        let p = Project::starter();
        let mr = p
            .mundos
            .first()
            .and_then(|m| p.render_mundo(m.id))
            .unwrap_or_else(fallback_render);
        match render::flythrough(&mr) {
            Ok(out) => eprintln!("flythrough ok: {out}"),
            Err(e) => eprintln!("flythrough error: {e}"),
        }
        return;
    }
    if std::env::args().any(|a| a == "--export") {
        let p = Project::starter();
        match p.escenas.first() {
            Some(s) => match render::export_scene(&p, s) {
                Ok(out) => eprintln!("export ok: {out}"),
                Err(e) => eprintln!("export error: {e}"),
            },
            None => eprintln!("export error: el proyecto no tiene escenas"),
        }
        return;
    }
    llimphi_ui::run::<Studio>();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El laboratorio corre una ley autorada del catálogo y forma estructura: el campo
    /// visible desarrolla rango espacial (certifica símbolos + params + programa + step,
    /// la cadena de app completa, sin GPU ni screenshot).
    #[test]
    fn lab_corre_ley_del_catalogo_y_forma_patron() {
        let ley = law_presets().into_iter().find(|(n, _)| *n == "reacción-difusión").unwrap().1;
        let LeyKind::Ecuacion { campos, .. } = &ley else { panic!("es Ecuacion") };
        let prog = ley.compile_ecuacion().unwrap().expect("compila");
        let mut lab = LawLab::build(1, 1, campos, Ok(prog), 1);
        let params = ley.ecuacion_param_values();
        for _ in 0..80 {
            lab.step(&params); // 80 · LAB_SUBSTEPS pasos
        }
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for z in 0..LAB_DIM {
            for x in 0..LAB_DIM {
                let v = lab.engine.get(1, x, 0, z);
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        assert!(hi - lo > 0.05, "la ley formó patrón en el lab (rango={})", hi - lo);
        // El heatmap se construye a la resolución del lab sin panics.
        let _img = heatmap_image(&lab);
    }

    /// Ciclar el catálogo desde la difusión por defecto entrega la reacción‑difusión.
    #[test]
    fn preset_cicla_al_siguiente() {
        let n = next_preset(&LeyKind::ecuacion_default());
        match &n {
            LeyKind::Ecuacion { campos, .. } => assert_eq!(campos.len(), 2),
            _ => panic!("sigue siendo Ecuacion"),
        }
    }

    /// La rampa cubre de oscuro a claro en los extremos (heatmap legible).
    #[test]
    fn rampa_va_de_oscuro_a_claro() {
        let bajo = ramp(0.0);
        let alto = ramp(1.0);
        assert!(bajo.iter().map(|&c| c as u32).sum::<u32>() < 100, "extremo bajo oscuro");
        assert!(alto[0] > 200 && alto[1] > 200, "extremo alto claro");
    }
}
