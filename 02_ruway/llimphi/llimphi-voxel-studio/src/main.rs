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

use llimphi_3d::glam::Vec3;
use llimphi_3d::Camera3d;
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
    window_origin_for_cast, world_dim, ActorScript, Age, Bioma, BiomaPalette, CharSpec, Clip,
    Forma, LeyKind, LeyUso, MatRole, Material, MaterialDef, Mundo, MundoRender, Project, SceneSpec,
    ShotKind, ShotSpec, PREVIEW_DIM_XZ,
};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_dock_rail::{dock_rail_view_side, DockRailItem, DockRailPalette, DockRailSide};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

mod ai;
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
                ("Piel", Icon::Image),
                ("Camiseta", Icon::Image),
                ("Pantalón", Icon::Image),
            ],
            Level::Biomas => vec![
                ("Relieve", Icon::Mountain),
                ("Materiales", Icon::Leaf),
                ("Objetos", Icon::Grid),
            ],
            Level::Mundos => vec![("Semilla", Icon::Globe), ("Biomas", Icon::Mountain)],
            Level::Escenas => vec![("Escena", Icon::Film), ("Cámara", Icon::Camera), ("Video", Icon::Play)],
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
    // Materiales.
    CycleMatRole,
    SetMatColor(usize, f32),
    SetMatGrain(f32),
    CycleMatParent,
    AddMatLey,
    RemoveMatLey,
    // Seres.
    CycleSereAge,
    SetSereColor(Part, usize, f32),
    // Biomas.
    SetBiomaField(BiomaField, f32),
    CycleBiomaGround,
    CycleBiomaCliff,
    CycleBiomaPeak,
    AddBiomaObjeto,
    RemoveBiomaObjeto,
    SetObjetoDensidad(usize, f32),
    // Mundos.
    SeedFocus,
    SeedKey(KeyEvent),
    SeedRandom,
    CycleMundoBioma,
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
    /// Semilla del random de mundos (LCG; no hay `Math.random`).
    rng: u32,
    /// Decisión global: dientes DENTRO (overlay) o FUERA (franja reservada).
    dientes_outside: bool,
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
                model.status = if model.simulating { "simulando agua…".into() } else { "agua estática".into() };
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
            size: Size { width: percent(1.0), height: percent(1.0) },
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
            size: Size { width: percent(1.0), height: Dimension::auto() },
            ..Default::default()
        })
        .text(model.status.clone(), 12.0, theme.fg_placeholder)
        .max_lines(3),
    );

    let content = View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(0.0), height: percent(1.0) },
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
        size: Size { width: percent(0.0), height: percent(1.0) },
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
    let m = if floating { 6.0 } else { 0.0 };
    let mut v = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: length(width), height: percent(1.0) },
        margin: Rect { left: length(m), right: length(m), top: length(m), bottom: length(m) },
        padding: pad(0.0, 6.0),
        gap: Size { width: length(4.0), height: length(0.0) },
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
        size: Size { width: percent(1.0), height: Dimension::auto() },
        gap: Size { width: length(4.0), height: length(0.0) },
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
        size: Size { width: percent(0.0), height: Dimension::auto() },
        ..Default::default()
    })
    .children(vec![child])
}

// =============================================================================
//  Centro (preview)
// =============================================================================

fn center(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let inner = match model.level {
        Level::Leyes => placeholder_2d(
            "Las Leyes son comportamientos (sin simular aún). Editá sus parámetros a la derecha.",
            theme,
        ),
        Level::Materiales => material_swatch(model),
        _ => canvas_3d(model),
    };
    View::new(Style {
        flex_grow: 1.0,
        size: Size { width: Dimension::auto(), height: percent(1.0) },
        ..Default::default()
    })
    .fill(Color::from_rgba8(12, 14, 18, 255))
    .children(vec![inner])
}

/// Mensaje centrado para niveles sin preview 3D.
fn placeholder_2d(text: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0), height: percent(1.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(llimphi_ui::llimphi_layout::taffy::prelude::JustifyContent::Center),
        padding: pad(40.0, 40.0),
        ..Default::default()
    })
    .children(vec![View::new(Style {
        size: Size { width: percent(0.7), height: Dimension::auto() },
        ..Default::default()
    })
    .text(text.to_string(), 16.0, theme.fg_muted)
    .max_lines(4)])
}

/// Swatch grande del color resuelto del material seleccionado.
fn material_swatch(model: &Model) -> View<Msg> {
    let c = model.project.resolve_material(model.sel[1]).color;
    let col = Color::from_rgba8(c[0], c[1], c[2], 255);
    View::new(Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0), height: percent(1.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(llimphi_ui::llimphi_layout::taffy::prelude::JustifyContent::Center),
        padding: pad(60.0, 60.0),
        ..Default::default()
    })
    .children(vec![View::new(Style {
        size: Size { width: percent(0.6), height: percent(0.6) },
        ..Default::default()
    })
    .fill(col)
    .radius(16.0)])
}

/// Canvas 3D: terreno del artefacto + (en escenas) los actores posados.
fn canvas_3d(model: &Model) -> View<Msg> {
    let (yaw, pitch, dist, gen) = (model.yaw, model.pitch, model.dist, model.gen);
    let preview = model.preview.clone();
    let mr = model.preview_render();
    let simulating = model.simulating;
    let agua = mr.palette.agua;
    // El agua fluye SÓLO si su material tiene la ley Fluir (con sus params).
    let fluir = model.project.water_fluir();
    let absolute = Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0), height: percent(1.0) },
        ..Default::default()
    };

    let canvas = match model.level {
        Level::Escenas => {
            let scene = sel_scene(model).cloned();
            let script_cam = model.script_cam;
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
                let origin = window_origin_for_cast(&scripts, time, dim);
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
                let mut metas = Vec::with_capacity(poses.len());
                for (pos, s, ch, at) in &poses {
                    let mut a = ch.to_actor(*pos, s.facing);
                    a.set_clip(s.clip);
                    a.advance(*at);
                    a.look_at(Some(camera.eye));
                    let (v, i) = a.mesh();
                    metas.push((a.model(), v, i));
                }
                p.render_scene(device, queue, encoder, target, vp, (rect.x, rect.y, rect.w, rect.h), &camera, &metas);
            })
        }
        Level::Seres => {
            let sere = sel_sere(model).cloned();
            let time = model.time;
            View::new(absolute).gpu_paint_with(move |device, queue, encoder, target, rect, vp| {
                let dim = world_dim(PREVIEW_DIM_XZ);
                let mut guard = preview.lock().unwrap();
                let p = guard.get_or_insert_with(|| WorldPreview::build(device, queue, &mr, dim, gen));
                p.rebuild_if(device, queue, &mr, dim, gen);
                let pos = p.ground_at(dim[0] / 2, dim[2] / 2);
                let look = pos + Vec3::new(0.0, 1.0, 0.0);
                let cam_dist = (dist * 0.06).clamp(3.5, 14.0);
                let camera = Camera3d::orbit(look, yaw, pitch, cam_dist);
                let metas = match &sere {
                    Some(cs) => {
                        let mut a = cs.to_actor(pos, time * 0.6);
                        a.advance(time);
                        let (v, i) = a.mesh();
                        vec![(a.model(), v, i)]
                    }
                    None => Vec::new(),
                };
                p.render_scene(device, queue, encoder, target, vp, (rect.x, rect.y, rect.w, rect.h), &camera, &metas);
            })
        }
        // Mundos / Biomas: sólo el terreno, en órbita. Con «simular», corre la ley
        // Fluir: el agua cae/esparce/cae por cornisas, paso por cuadro.
        _ => View::new(absolute).gpu_paint_with(move |device, queue, encoder, target, rect, vp| {
            let dim = world_dim(PREVIEW_DIM_XZ);
            let mut guard = preview.lock().unwrap();
            let p = guard.get_or_insert_with(|| WorldPreview::build(device, queue, &mr, dim, gen));
            p.rebuild_if(device, queue, &mr, dim, gen);
            match (simulating, fluir) {
                (true, Some((g, h))) => {
                    p.ensure_sim(agua, g, h);
                    p.sim_step(queue, agua);
                }
                _ => p.clear_sim(),
            }
            let camera = Camera3d::orbit(orbit_center(dim), yaw, pitch, dist);
            p.render(device, queue, encoder, target, vp, (rect.x, rect.y, rect.w, rect.h), &camera);
        }),
    }
    .draggable(|phase, dx, dy| match phase {
        DragPhase::Move => Some(Msg::Orbit(dx, dy)),
        DragPhase::End => None,
    });

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
        spacer(6.0),
        section_title("PARÁMETROS", theme),
    ];
    if l.kind.params().is_empty() {
        v.push(body_text("este tipo no tiene parámetros".into(), theme.fg_placeholder, theme));
    }
    for (i, (name, value, min, max)) in l.kind.params().into_iter().enumerate() {
        v.push(slider_view(name, value, min, max, &sp, move |_p, dv| Some(Msg::SetLeyParam(i, value + dv))));
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
            button_view(format!("edad: {}", c.age.label()), &btn, Msg::CycleSereAge),
        ],
        1 => color_tools("PIEL", Part::Skin, c.skin, &sp, theme),
        2 => color_tools("CAMISETA", Part::Shirt, c.shirt, &sp, theme),
        _ => color_tools("PANTALÓN", Part::Pants, c.pants, &sp, theme),
    }
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
            section_title("LEY FLUIR (AGUA)", theme),
            button_view(if model.simulating { "⏸ detener agua" } else { "💧 simular agua" }, &btn, Msg::ToggleSim),
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
        _ => {
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
            section_title("LEY FLUIR (AGUA)", theme),
            button_view(if model.simulating { "⏸ detener agua" } else { "💧 simular agua" }, &btn, Msg::ToggleSim),
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
        1 => {
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
        size: Size { width: percent(1.0), height: length(30.0) },
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
        size: Size { width: percent(1.0), height: length(20.0) },
        ..Default::default()
    })
    .text(text.to_string(), 12.0, theme.accent)
    .bold()
}

fn body_text(s: String, color: Color, _theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0), height: Dimension::auto() },
        ..Default::default()
    })
    .text(s, 13.0, color)
    .max_lines(2)
}

fn spacer(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0), height: length(h) },
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
    Size { width: length(0.0), height: length(h) }
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
        rng: 0x1234_5678,
        dientes_outside: wawa_config::WawaConfig::load().dientes_outside,
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
