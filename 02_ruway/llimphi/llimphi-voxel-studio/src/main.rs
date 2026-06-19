//! # llimphi-voxel-studio — el creador de mundos, con interfaz
//!
//! La **interfaz propia** para crear/editar los artefactos de `llimphi-voxel`.
//! Fase 1: el **editor de mundos** — lista de mundos del [`Project`], sliders
//! in-situ de la [`WorldRecipe`] seleccionada, **preview 3D en vivo** que
//! regenera el terreno al mover cualquier parámetro, y persistencia RON.
//!
//! Próximas fases (ver el plan): asistencia por IA ("describí un mundo" → receta),
//! editor de escenas (director) y de personajes.
//!
//! ```bash
//! cargo run -p llimphi-voxel-studio --release            # ventana interactiva
//! cargo run -p llimphi-voxel-studio --release -- --shot  # PNG headless a /tmp
//! ```
//! - Lista izquierda: elegí un mundo / creá uno nuevo / guardá / cargá.
//! - Centro: **arrastrar** orbita, **rueda** hace zoom.
//! - Derecha: sliders de relieve/montañas/ríos/agua + ciclo de materiales y flora.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use llimphi_3d::glam::Vec3;
use llimphi_3d::Camera3d;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    length, percent, AlignItems, Dimension, FlexDirection, Position, Size, Style,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{
    App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta,
};
use llimphi_voxel::{
    world_dim, ActorScript, Age, CharSpec, Project, SceneSpec, ShotKind, ShotSpec, WorldRecipe,
    PREVIEW_DIM_XZ,
};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

mod ai;
mod preview;
mod render;
mod shot;
mod soundtrack;
use preview::WorldPreview;

/// Dónde se guarda/carga el proyecto (relativo al cwd).
const PROJECT_PATH: &str = "voxel-studio.ron";

/// Paso de tiempo de la reproducción de escenas (~30 fps).
const DT: f32 = 1.0 / 30.0;

/// Qué artefacto edita la UI ahora.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Editor de mundos (receta + preview).
    Worlds,
    /// Editor/reproductor de escenas (director).
    Scenes,
    /// Editor de personajes (constitución + colores).
    Characters,
}

/// Parte coloreable de un personaje.
#[derive(Clone, Copy)]
enum Part {
    Skin,
    Shirt,
    Pants,
}

/// Los parámetros `f32` de la receta que un slider edita.
#[derive(Clone, Copy)]
enum Field {
    Seed,
    Base,
    Dune,
    Relief,
    Mountains,
    Water,
    Rivers,
    PeakAt,
    Flora,
}

#[derive(Clone)]
enum Msg {
    /// Elegir el mundo `i` de la lista.
    Select(usize),
    /// Fijar un parámetro de la receta del mundo actual.
    Set(Field, f32),
    /// Ciclar el material del suelo / acantilado / cumbre, o la flora.
    CycleGround,
    CycleCliff,
    CyclePeak,
    CycleFlora,
    /// Crear un mundo nuevo (pradera por defecto).
    NewWorld,
    /// Guardar / cargar el proyecto en RON.
    Save,
    Load,
    /// Cámara de órbita.
    Orbit(f32, f32),
    Zoom(f32),
    /// IA "poor": foco/tecla del campo de descripción, disparo y resultado.
    AiFocus,
    AiKey(KeyEvent),
    AiGenerate,
    AiResult(WorldRecipe, String),
    /// Escenas (director).
    SwitchMode(Mode),
    SelectScene(usize),
    CycleSceneWorld,
    SetSceneDur(f32),
    TogglePlay,
    Scrub(f32),
    AiSceneGenerate,
    AiSceneResult(SceneSpec),
    /// Cámara de escena: alterna órbita libre ↔ guion (planos); agrega/quita plano
    /// en el instante actual.
    ToggleSceneCam,
    AddShot,
    RemoveShot,
    /// Exportar la escena a video (headless, en un worker).
    ExportVideo,
    ExportDone(Result<String, String>),
    /// Elegir el diente (set de herramientas) del sidebar derecho.
    SelectTool(usize),
    /// Personajes (constitución + colores).
    SelectChar(usize),
    CycleCharAge,
    SetColor(Part, usize, f32),
    NewChar,
    AiCharGenerate,
    AiCharResult(CharSpec),
    /// Tick periódico de reproducción.
    Tick,
}

struct Model {
    theme: Theme,
    project: Project,
    /// Índice del mundo seleccionado.
    sel: usize,
    /// Generación de la receta: se incrementa en cada edición para que el preview
    /// sepa que tiene que regenerar el grid.
    gen: u64,
    /// Cámara de órbita.
    yaw: f32,
    pitch: f32,
    dist: f32,
    /// Preview perezoso: se construye en la 1ª pintada GPU (ahí hay device/queue).
    preview: Arc<Mutex<Option<WorldPreview>>>,
    /// Mensaje de estado (guardado/cargado/errores).
    status: String,
    /// Campo de descripción para la IA + foco + flag "generando".
    ai_input: TextInputState,
    ai_focused: bool,
    ai_busy: bool,
    /// Modo de edición + estado de escenas/personajes.
    mode: Mode,
    scene_sel: usize,
    char_sel: usize,
    /// Tiempo de reproducción (seg) y si está corriendo.
    time: f32,
    playing: bool,
    /// Cámara de escena guionada (planos) vs órbita libre.
    script_cam: bool,
    /// Exportación de video en curso.
    exporting: bool,
    /// Diente activo del sidebar derecho (qué set de herramientas se muestra).
    tool_tab: usize,
}

impl Model {
    /// Receta del mundo seleccionado (si hay).
    fn recipe(&self) -> Option<WorldRecipe> {
        self.project.worlds.get(self.sel).map(|w| w.recipe)
    }

    /// La escena seleccionada (si hay).
    fn scene(&self) -> Option<&SceneSpec> {
        self.project.scenes.get(self.scene_sel)
    }

    /// La receta del mundo de fondo de la escena seleccionada.
    fn scene_recipe(&self) -> Option<WorldRecipe> {
        let s = self.scene()?;
        self.project.worlds.get(s.world).map(|w| w.recipe)
    }

    /// El personaje seleccionado (si hay).
    fn char_spec(&self) -> Option<&CharSpec> {
        self.project.characters.get(self.char_sel)
    }
}

/// Distancia de órbita inicial/clamp en función del lado del mundo.
fn default_dist() -> f32 {
    PREVIEW_DIM_XZ as f32 * 1.6
}

/// Nombre de mundo a partir de la descripción de la IA: las primeras ~3 palabras.
fn world_name_from(prompt: &str) -> String {
    let s: String = prompt.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        "mundo IA".into()
    } else {
        format!("IA: {s}")
    }
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
        // Reloj de reproducción de escenas (avanza `time` cuando `playing`).
        handle.spawn_periodic(Duration::from_millis(33), || Msg::Tick);
        demo_model()
    }

    fn on_wheel(_m: &Model, delta: WheelDelta, _c: (f32, f32), _mods: Modifiers) -> Option<Msg> {
        Some(Msg::Zoom(delta.y))
    }

    fn on_key(model: &Model, ev: &KeyEvent) -> Option<Msg> {
        // Con el campo de la IA enfocado, el teclado lo alimenta: Enter genera,
        // Escape lo suelta, el resto va al buffer de texto.
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
            Msg::Select(i) => {
                if i < model.project.worlds.len() {
                    model.sel = i;
                    model.gen += 1; // forzar regeneración del preview
                }
            }
            Msg::Set(field, v) => {
                if let Some(w) = model.project.worlds.get_mut(model.sel) {
                    let r = &mut w.recipe;
                    match field {
                        Field::Seed => r.seed = v.max(0.0) as u32,
                        Field::Base => r.base = v.clamp(0.0, 0.9),
                        Field::Dune => r.dune = v.clamp(0.0, 0.4),
                        Field::Relief => r.relief = v.clamp(0.0, 1.0),
                        Field::Mountains => r.mountains = v.clamp(0.0, 1.0),
                        Field::Water => r.water_level = v.clamp(0.0, 0.9),
                        Field::Rivers => r.rivers = v.clamp(0.0, 1.0),
                        Field::PeakAt => r.peak_at = v.clamp(0.0, 1.0),
                        Field::Flora => r.flora_density = v.clamp(0.0, 0.05),
                    }
                    model.gen += 1;
                }
            }
            Msg::CycleGround => {
                if let Some(w) = model.project.worlds.get_mut(model.sel) {
                    w.recipe.ground = w.recipe.ground.next();
                    model.gen += 1;
                }
            }
            Msg::CycleCliff => {
                if let Some(w) = model.project.worlds.get_mut(model.sel) {
                    w.recipe.cliff = w.recipe.cliff.next();
                    model.gen += 1;
                }
            }
            Msg::CyclePeak => {
                if let Some(w) = model.project.worlds.get_mut(model.sel) {
                    w.recipe.peak = w.recipe.peak.next();
                    model.gen += 1;
                }
            }
            Msg::CycleFlora => {
                if let Some(w) = model.project.worlds.get_mut(model.sel) {
                    w.recipe.flora = w.recipe.flora.next();
                    model.gen += 1;
                }
            }
            Msg::NewWorld => {
                let n = model.project.worlds.len() + 1;
                let idx = model
                    .project
                    .add_world(llimphi_voxel::NamedWorld::new(
                        format!("mundo {n}"),
                        WorldRecipe::grassland(1337 + n as u32),
                    ));
                model.sel = idx;
                model.gen += 1;
                model.status = format!("mundo nuevo: «{}»", model.project.worlds[idx].name);
            }
            Msg::Save => match save_project(&model.project) {
                Ok(_) => model.status = format!("guardado en {PROJECT_PATH}"),
                Err(e) => model.status = format!("error al guardar: {e}"),
            },
            Msg::Load => match load_project() {
                Ok(p) => {
                    let n = p.worlds.len();
                    model.project = p;
                    model.sel = model.sel.min(n.saturating_sub(1));
                    model.gen += 1;
                    model.status = format!("cargado de {PROJECT_PATH} ({n} mundos)");
                }
                Err(e) => model.status = format!("error al cargar: {e}"),
            },
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
            Msg::AiFocus => model.ai_focused = true,
            Msg::AiKey(ev) => {
                model.ai_input.apply_key(&ev);
            }
            Msg::AiGenerate => {
                let prompt = model.ai_input.text();
                if !prompt.trim().is_empty() && !model.ai_busy {
                    model.ai_busy = true;
                    model.status = "generando mundo con IA…".into();
                    let name = world_name_from(&prompt);
                    handle.spawn(move || Msg::AiResult(ai::generate(&prompt), name));
                }
            }
            Msg::AiResult(recipe, name) => {
                let idx = model
                    .project
                    .add_world(llimphi_voxel::NamedWorld::new(name, recipe));
                model.sel = idx;
                model.gen += 1;
                model.ai_busy = false;
                model.ai_input.set_text("");
                model.status = format!("mundo generado: «{}»", model.project.worlds[idx].name);
            }
            Msg::SwitchMode(m) => {
                model.mode = m;
                model.tool_tab = 0; // cada modo tiene su propio juego de dientes
                model.gen += 1; // el preview pasa a mostrar otro artefacto
            }
            Msg::SelectTool(i) => model.tool_tab = i,
            Msg::SelectScene(i) => {
                if i < model.project.scenes.len() {
                    model.scene_sel = i;
                    model.time = 0.0;
                    model.gen += 1;
                }
            }
            Msg::CycleSceneWorld => {
                let nw = model.project.worlds.len().max(1);
                if let Some(s) = model.project.scenes.get_mut(model.scene_sel) {
                    s.world = (s.world + 1) % nw;
                    model.gen += 1;
                }
            }
            Msg::SetSceneDur(v) => {
                if let Some(s) = model.project.scenes.get_mut(model.scene_sel) {
                    s.duration = v.clamp(1.0, 20.0);
                }
            }
            Msg::TogglePlay => model.playing = !model.playing,
            Msg::Scrub(v) => {
                model.playing = false;
                let dur = model.scene().map(|s| s.duration).unwrap_or(1.0);
                model.time = v.clamp(0.0, dur);
            }
            Msg::AiSceneGenerate => {
                let prompt = model.ai_input.text();
                if !prompt.trim().is_empty() && !model.ai_busy {
                    model.ai_busy = true;
                    model.status = "generando escena con IA…".into();
                    let world = model.scene().map(|s| s.world).unwrap_or(model.sel);
                    let dim = world_dim(PREVIEW_DIM_XZ);
                    handle.spawn(move || Msg::AiSceneResult(ai::generate_scene(&prompt, world, dim)));
                }
            }
            Msg::AiSceneResult(scene) => {
                let idx = model.project.add_scene(scene);
                model.scene_sel = idx;
                model.time = 0.0;
                model.playing = true;
                model.gen += 1;
                model.ai_busy = false;
                model.ai_input.set_text("");
                model.status = format!("escena generada: «{}»", model.project.scenes[idx].name);
            }
            Msg::ToggleSceneCam => model.script_cam = !model.script_cam,
            Msg::AddShot => {
                let t = model.time;
                if let Some(s) = model.project.scenes.get_mut(model.scene_sel) {
                    // El tipo del plano nuevo cicla por la cantidad ya puesta.
                    let kind = ShotKind::ALL[s.shots.len() % ShotKind::ALL.len()];
                    s.shots.push(ShotSpec { start: t, kind });
                    s.shots.sort_by(|a, b| a.start.total_cmp(&b.start));
                    model.status = format!("plano agregado: {} @ {t:.1}s", kind.label());
                }
            }
            Msg::RemoveShot => {
                if let Some(s) = model.project.scenes.get_mut(model.scene_sel) {
                    s.shots.pop();
                    model.status = "último plano quitado".into();
                }
            }
            Msg::ExportVideo => {
                if !model.exporting {
                    if let Some(scene) = model.scene().cloned() {
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
            Msg::SelectChar(i) => {
                if i < model.project.characters.len() {
                    model.char_sel = i;
                }
            }
            Msg::CycleCharAge => {
                if let Some(c) = model.project.characters.get_mut(model.char_sel) {
                    c.age = c.age.next();
                }
            }
            Msg::SetColor(part, ch, v) => {
                if let Some(c) = model.project.characters.get_mut(model.char_sel) {
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
            Msg::NewChar => {
                let n = model.project.characters.len() + 1;
                model.project.characters.push(CharSpec::new(format!("personaje {n}"), Age::Adult));
                model.char_sel = model.project.characters.len() - 1;
                model.status = "personaje nuevo".into();
            }
            Msg::AiCharGenerate => {
                let prompt = model.ai_input.text();
                if !prompt.trim().is_empty() && !model.ai_busy {
                    model.ai_busy = true;
                    model.status = "generando personaje con IA…".into();
                    handle.spawn(move || Msg::AiCharResult(ai::generate_character(&prompt)));
                }
            }
            Msg::AiCharResult(cs) => {
                model.project.characters.push(cs);
                model.char_sel = model.project.characters.len() - 1;
                model.ai_busy = false;
                model.ai_input.set_text("");
                model.status =
                    format!("personaje generado: «{}»", model.project.characters[model.char_sel].name);
            }
            Msg::Tick => {
                if model.mode == Mode::Scenes && model.playing {
                    if let Some(dur) = model.scene().map(|s| s.duration) {
                        model.time += DT;
                        if model.time >= dur {
                            model.time = 0.0; // loop
                        }
                    }
                } else if model.mode == Mode::Characters {
                    // Turntable + respiración del personaje en exhibición.
                    model.time += DT;
                }
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0), height: percent(1.0) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![
            left_panel(model),
            center_canvas(model),
            right_panel(model),
        ])
    }
}

// =============================================================================
//  Paneles
// =============================================================================

/// Panel izquierdo: **rail de modos** (dientes Mundos·Escenas·Gente) al borde +
/// el contenido del modo (navegación + IA + estado) al costado. Mismo widget de
/// dientes que el sidebar derecho (`dock_rail_view`).
fn left_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let btn = ButtonPalette::from_theme(theme);

    // Rail de modos: un diente por modo, el activo sobresale.
    let active = mode_index(model.mode);
    let mode_items: Vec<DockRailItem> = (0..3)
        .map(|i| DockRailItem { id: i as u64, active: i == active })
        .collect();
    let rail = dock_rail_view(
        &mode_items,
        46.0,
        &DockRailPalette::from_theme(theme),
        |id, size, color| {
            View::new(Style::default())
                .text(["Mu", "Es", "Ge"][id as usize].to_string(), size * 0.7, color)
        },
        |id| Msg::SwitchMode(mode_from_index(id as usize)),
        |_| None,
    );

    // Contenido del modo: navegación propia…
    let mut rows: Vec<View<Msg>> = Vec::new();
    match model.mode {
        Mode::Worlds => worlds_left(model, &btn, &mut rows),
        Mode::Scenes => scenes_left(model, &btn, &mut rows),
        Mode::Characters => chars_left(model, &btn, &mut rows),
    }

    // …IA compartida (rótulo/acción según modo)…
    let (ai_title, ai_hint, ai_msg): (&str, &str, Msg) = match model.mode {
        Mode::Worlds => (
            "IA — DESCRIBÍ UN MUNDO",
            "p.ej. islas con ríos y nieve",
            Msg::AiGenerate,
        ),
        Mode::Scenes => (
            "IA — DESCRIBÍ UNA ESCENA",
            "p.ej. tres personajes que festejan",
            Msg::AiSceneGenerate,
        ),
        Mode::Characters => (
            "IA — DESCRIBÍ UN PERSONAJE",
            "p.ej. una niña de remera verde",
            Msg::AiCharGenerate,
        ),
    };
    rows.push(spacer(14.0));
    rows.push(section_title(ai_title, theme));
    rows.push(text_input_view(
        &model.ai_input,
        ai_hint,
        model.ai_focused,
        &TextInputPalette::from_theme(theme),
        Msg::AiFocus,
    ));
    rows.push(spacer(6.0));
    rows.push(button_view(
        if model.ai_busy { "generando…" } else { "generar (IA)" },
        &btn,
        ai_msg,
    ));

    // …y el estado al pie.
    rows.push(spacer(12.0));
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

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: length(256.0), height: percent(1.0) },
        padding: pad(0.0, 6.0),
        gap: Size { width: length(4.0), height: length(0.0) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![rail, content])
}

/// Índice del modo (para mapearlo a un diente del rail) y su inversa.
fn mode_index(m: Mode) -> usize {
    match m {
        Mode::Worlds => 0,
        Mode::Scenes => 1,
        Mode::Characters => 2,
    }
}
fn mode_from_index(i: usize) -> Mode {
    match i {
        0 => Mode::Worlds,
        1 => Mode::Scenes,
        _ => Mode::Characters,
    }
}

/// Fila seleccionable (mundo o escena) de la lista izquierda.
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

/// Contenido izquierdo del modo Mundos.
fn worlds_left(model: &Model, btn: &ButtonPalette, rows: &mut Vec<View<Msg>>) {
    let theme = &model.theme;
    rows.push(section_title("MUNDOS", theme));
    for (i, w) in model.project.worlds.iter().enumerate() {
        rows.push(selectable_row(w.name.clone(), i == model.sel, Msg::Select(i), theme));
    }
    rows.push(spacer(10.0));
    rows.push(button_view("nuevo mundo", btn, Msg::NewWorld));
    rows.push(spacer(6.0));
    rows.push(button_view("guardar", btn, Msg::Save));
    rows.push(spacer(6.0));
    rows.push(button_view("cargar", btn, Msg::Load));
}

/// Contenido izquierdo del modo Escenas: lista + reproducción.
fn scenes_left(model: &Model, btn: &ButtonPalette, rows: &mut Vec<View<Msg>>) {
    let theme = &model.theme;
    rows.push(section_title("ESCENAS", theme));
    for (i, s) in model.project.scenes.iter().enumerate() {
        rows.push(selectable_row(s.name.clone(), i == model.scene_sel, Msg::SelectScene(i), theme));
    }
    rows.push(spacer(10.0));
    rows.push(button_view("guardar", btn, Msg::Save));
    rows.push(spacer(6.0));
    rows.push(button_view("cargar", btn, Msg::Load));

    rows.push(spacer(12.0));
    rows.push(section_title("REPRODUCCIÓN", theme));
    rows.push(button_view(
        if model.playing { "❚❚ pausa" } else { "▶ reproducir" },
        btn,
        Msg::TogglePlay,
    ));
    rows.push(spacer(6.0));
    let dur = model.scene().map(|s| s.duration).unwrap_or(1.0);
    let t = model.time;
    rows.push(slider_view(
        "tiempo",
        t.min(dur),
        0.0,
        dur,
        &SliderPalette::from_theme(theme),
        move |_phase, dv| Some(Msg::Scrub(t + dv)),
    ));
}

/// Contenido izquierdo del modo Personajes: lista + nuevo.
fn chars_left(model: &Model, btn: &ButtonPalette, rows: &mut Vec<View<Msg>>) {
    let theme = &model.theme;
    rows.push(section_title("GENTE", theme));
    for (i, c) in model.project.characters.iter().enumerate() {
        let label = format!("{} · {}", c.name, c.age.label());
        rows.push(selectable_row(label, i == model.char_sel, Msg::SelectChar(i), theme));
    }
    rows.push(spacer(10.0));
    rows.push(button_view("nuevo personaje", btn, Msg::NewChar));
    rows.push(spacer(6.0));
    rows.push(button_view("guardar", btn, Msg::Save));
    rows.push(spacer(6.0));
    rows.push(button_view("cargar", btn, Msg::Load));
}

/// Centro: el canvas 3D del preview en vivo, draggable para orbitar. En modo
/// Escenas, además posa y anima los actores del guion en el instante `time`; en
/// modo Personajes, exhibe al personaje seleccionado en turntable.
fn center_canvas(model: &Model) -> View<Msg> {
    let (yaw, pitch, dist, gen) = (model.yaw, model.pitch, model.dist, model.gen);
    let preview = model.preview.clone();

    let absolute = Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0), height: percent(1.0) },
        ..Default::default()
    };

    let canvas = match model.mode {
        Mode::Worlds => {
            let recipe = model.recipe().unwrap_or_else(|| WorldRecipe::grassland(1));
            View::new(absolute).gpu_paint_with(move |device, queue, encoder, target, rect, vp| {
                let dim = world_dim(PREVIEW_DIM_XZ);
                let mut guard = preview.lock().unwrap();
                let p =
                    guard.get_or_insert_with(|| WorldPreview::build(device, queue, &recipe, dim, gen));
                p.rebuild_if(device, queue, &recipe, dim, gen);
                let camera = Camera3d::orbit(orbit_center(dim), yaw, pitch, dist);
                p.render(device, queue, encoder, target, vp, (rect.x, rect.y, rect.w, rect.h), &camera);
            })
        }
        Mode::Scenes => {
            let recipe = model.scene_recipe().unwrap_or_else(|| WorldRecipe::grassland(1));
            let scene = model.scene().cloned();
            let script_cam = model.script_cam;
            let scripts: Vec<ActorScript> =
                scene.as_ref().map(|s| s.scripts()).unwrap_or_default();
            let chars: Vec<CharSpec> = scene
                .as_ref()
                .map(|s| {
                    s.actors
                        .iter()
                        .map(|a| model.project.character_or_default(a.character))
                        .collect()
                })
                .unwrap_or_default();
            let time = model.time;
            View::new(absolute).gpu_paint_with(move |device, queue, encoder, target, rect, vp| {
                let dim = world_dim(PREVIEW_DIM_XZ);
                let mut guard = preview.lock().unwrap();
                let p =
                    guard.get_or_insert_with(|| WorldPreview::build(device, queue, &recipe, dim, gen));
                p.rebuild_if(device, queue, &recipe, dim, gen);
                // Primero ubicar a los actores sobre el relieve para encuadrar al
                // reparto (su centroide), no el mundo entero — si no, salen diminutos.
                // El shader voxel espera coords CENTRADAS (grilla centrada en el
                // origen: world = grilla − dim/2). Sin esto el reparto del guion
                // "flota" en cielo vacío (el terreno queda corrido fuera de cuadro).
                let half = Vec3::new(dim[0] as f32, dim[1] as f32, dim[2] as f32) * 0.5;
                let mut poses = Vec::with_capacity(scripts.len());
                let mut centroid = Vec3::ZERO;
                for (script, ch) in scripts.iter().zip(&chars) {
                    // Cuantización por actor (Héroe en doses) — igual que el export.
                    let at = script.quantize(time);
                    let s = script.sample(at);
                    let pos = p.ground_at(s.gx.max(0.0) as u32, s.gz.max(0.0) as u32) - half;
                    centroid += pos;
                    poses.push((pos, s, ch, at));
                }
                let look = if poses.is_empty() {
                    orbit_center(dim) - half
                } else {
                    centroid / poses.len() as f32 + Vec3::new(0.0, 1.0, 0.0)
                };
                // Cámara: en modo guion, el plano vigente; si no, órbita libre
                // (la rueda, que mueve `dist`, acerca/aleja).
                let cast_d = 6.0 + poses.len() as f32 * 1.2;
                let scene_dist = (dist * 0.18).clamp(10.0, 70.0);
                let camera = match (script_cam, &scene) {
                    (true, Some(sc)) => sc.camera_at(look, cast_d, time),
                    _ => Camera3d::orbit(look, yaw, pitch, scene_dist),
                };
                // Posar cada actor (mirando a la cámara) y mallar.
                let mut metas = Vec::with_capacity(poses.len());
                for (pos, s, ch, at) in &poses {
                    let mut a = ch.to_actor(*pos, s.facing);
                    a.set_clip(s.clip);
                    a.advance(*at);
                    a.look_at(Some(camera.eye));
                    let (v, i) = a.mesh();
                    metas.push((a.model(), v, i));
                }
                p.render_scene(
                    device, queue, encoder, target, vp, (rect.x, rect.y, rect.w, rect.h), &camera,
                    &metas,
                );
            })
        }
        Mode::Characters => {
            let recipe = model.recipe().unwrap_or_else(|| WorldRecipe::grassland(1));
            let charspec = model.char_spec().cloned();
            let time = model.time;
            View::new(absolute).gpu_paint_with(move |device, queue, encoder, target, rect, vp| {
                let dim = world_dim(PREVIEW_DIM_XZ);
                let mut guard = preview.lock().unwrap();
                let p =
                    guard.get_or_insert_with(|| WorldPreview::build(device, queue, &recipe, dim, gen));
                p.rebuild_if(device, queue, &recipe, dim, gen);
                // El personaje en el centro del mundo, en turntable; cámara cerca.
                let pos = p.ground_at(dim[0] / 2, dim[2] / 2);
                let look = pos + Vec3::new(0.0, 1.0, 0.0);
                let cam_dist = (dist * 0.06).clamp(3.5, 14.0);
                let camera = Camera3d::orbit(look, yaw, pitch, cam_dist);
                let metas = match &charspec {
                    Some(cs) => {
                        let mut a = cs.to_actor(pos, time * 0.6); // gira despacio
                        a.advance(time); // respira (Idle)
                        let (v, i) = a.mesh();
                        vec![(a.model(), v, i)]
                    }
                    None => Vec::new(),
                };
                p.render_scene(
                    device, queue, encoder, target, vp, (rect.x, rect.y, rect.w, rect.h), &camera,
                    &metas,
                );
            })
        }
    }
    .draggable(|phase, dx, dy| match phase {
        DragPhase::Move => Some(Msg::Orbit(dx, dy)),
        DragPhase::End => None,
    });

    View::new(Style {
        flex_grow: 1.0,
        size: Size { width: Dimension::auto(), height: percent(1.0) },
        ..Default::default()
    })
    .fill(Color::from_rgba8(12, 14, 18, 255))
    .children(vec![canvas])
}

/// Centro de órbita: el medio del mundo (grilla `[0,dim]`), algo por debajo del
/// tope para encuadrar el relieve.
fn orbit_center(dim: [u32; 3]) -> Vec3 {
    Vec3::new(dim[0] as f32 * 0.5, dim[1] as f32 * 0.32, dim[2] as f32 * 0.5)
}

/// Panel derecho: **rail de dientes** (un set de herramientas por diente) pegado al
/// borde interno + el panel del set activo al costado. Sigue el patrón canónico de
/// `llimphi-widget-dock-rail` (items→ids, `make_icon`, `on_activate`), como cosmos.
fn right_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let icons = tool_icons(model.mode);
    let tab = model.tool_tab.min(icons.len().saturating_sub(1));

    // Dientes: uno por set de herramientas del modo. El diente activo "sobresale".
    let rail_items: Vec<DockRailItem> = (0..icons.len())
        .map(|i| DockRailItem { id: i as u64, active: i == tab })
        .collect();
    let labels = icons.clone();
    let rail = dock_rail_view(
        &rail_items,
        46.0,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| {
            View::new(Style::default()).text(labels[id as usize].to_string(), size * 0.7, color)
        },
        |id| Msg::SelectTool(id as usize),
        |_payload| None,
    );

    // Panel del set activo.
    let panel = View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(0.0), height: percent(1.0) },
        padding: pad(14.0, 14.0),
        gap: gap_y(8.0),
        ..Default::default()
    })
    .children(tool_content(model, tab));

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: length(300.0), height: percent(1.0) },
        padding: pad(0.0, 6.0),
        gap: Size { width: length(4.0), height: length(0.0) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![rail, panel])
}

/// Etiqueta corta de cada diente (set de herramientas) por modo.
fn tool_icons(mode: Mode) -> Vec<&'static str> {
    match mode {
        Mode::Worlds => vec!["Re", "Ag", "Mt", "Fl"],
        Mode::Scenes => vec!["Es", "Cá", "Vi"],
        Mode::Characters => vec!["Cu", "Pi", "Cm", "Pa"],
    }
}

/// Contenido del set de herramientas activo (`tab`) según el modo.
fn tool_content(model: &Model, tab: usize) -> Vec<View<Msg>> {
    match model.mode {
        Mode::Worlds => world_tools(model, tab),
        Mode::Scenes => scene_tools(model, tab),
        Mode::Characters => char_tools(model, tab),
    }
}

/// Slider de un campo de la receta (helper de los sets de mundo).
fn wslider(
    sp: &SliderPalette,
    label: &str,
    value: f32,
    min: f32,
    max: f32,
    field: Field,
) -> View<Msg> {
    slider_view(label, value, min, max, sp, move |_phase, dv| Some(Msg::Set(field, value + dv)))
}

/// Sets de herramientas del modo Mundos: Relieve · Agua · Materiales · Flora.
fn world_tools(model: &Model, tab: usize) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let sp = SliderPalette::from_theme(theme);
    let btn = ButtonPalette::from_theme(theme);
    let Some(r) = model.recipe() else {
        return vec![section_title("sin mundo seleccionado", theme)];
    };
    match tab {
        0 => vec![
            section_title("RELIEVE", theme),
            wslider(&sp, "semilla", r.seed as f32, 0.0, 64.0, Field::Seed),
            wslider(&sp, "base (llanura)", r.base, 0.0, 0.9, Field::Base),
            wslider(&sp, "dunas", r.dune, 0.0, 0.4, Field::Dune),
            wslider(&sp, "relieve (alto)", r.relief, 0.0, 1.0, Field::Relief),
            wslider(&sp, "densidad montañas", r.mountains, 0.0, 1.0, Field::Mountains),
        ],
        1 => vec![
            section_title("AGUA", theme),
            wslider(&sp, "nivel del agua", r.water_level, 0.0, 0.9, Field::Water),
            wslider(&sp, "densidad ríos", r.rivers, 0.0, 1.0, Field::Rivers),
        ],
        2 => vec![
            section_title("MATERIALES", theme),
            button_view(format!("suelo: {}", r.ground.label()), &btn, Msg::CycleGround),
            spacer(4.0),
            button_view(format!("acantilado: {}", r.cliff.label()), &btn, Msg::CycleCliff),
            spacer(4.0),
            button_view(format!("cumbre: {}", r.peak.label()), &btn, Msg::CyclePeak),
            wslider(&sp, "altura cumbre", r.peak_at, 0.0, 1.0, Field::PeakAt),
        ],
        _ => vec![
            section_title("FLORA", theme),
            button_view(format!("tipo: {}", r.flora.label()), &btn, Msg::CycleFlora),
            wslider(&sp, "densidad flora", r.flora_density, 0.0, 0.05, Field::Flora),
        ],
    }
}

/// Sets de herramientas del modo Escenas: Escena · Cámara · Video.
fn scene_tools(model: &Model, tab: usize) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let sp = SliderPalette::from_theme(theme);
    let btn = ButtonPalette::from_theme(theme);
    let Some(s) = model.scene() else {
        return vec![section_title("sin escena — generá una con IA", theme)];
    };
    match tab {
        0 => {
            let dur = s.duration;
            let world_name = model
                .project
                .worlds
                .get(s.world)
                .map(|w| w.name.clone())
                .unwrap_or_else(|| "—".into());
            vec![
                section_title("ESCENA", theme),
                body_text(format!("«{}»", s.name), theme.fg_text, theme),
                spacer(6.0),
                section_title("MUNDO DE FONDO", theme),
                button_view(format!("mundo: {world_name}"), &btn, Msg::CycleSceneWorld),
                spacer(6.0),
                section_title("TIEMPO", theme),
                slider_view("duración (s)", dur, 1.0, 20.0, &sp, move |_p, dv| {
                    Some(Msg::SetSceneDur(dur + dv))
                }),
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
                spacer(8.0),
                body_text(
                    "agregá cortes mientras scrubeás el tiempo".into(),
                    theme.fg_placeholder,
                    theme,
                ),
            ]
        }
        _ => vec![
            section_title("VIDEO", theme),
            button_view(
                if model.exporting { "exportando…" } else { "🎬 exportar video" },
                &btn,
                Msg::ExportVideo,
            ),
            spacer(8.0),
            body_text(
                "renderiza el guion a un .mkv (puede tardar)".into(),
                theme.fg_placeholder,
                theme,
            ),
        ],
    }
}

/// Sets de herramientas del modo Gente: Cuerpo · Piel · Camiseta · Pantalón.
fn char_tools(model: &Model, tab: usize) -> Vec<View<Msg>> {
    let theme = &model.theme;
    let sp = SliderPalette::from_theme(theme);
    let btn = ButtonPalette::from_theme(theme);
    let Some(c) = model.char_spec() else {
        return vec![section_title("sin personaje — creá o generá uno", theme)];
    };
    match tab {
        0 => vec![
            section_title("CUERPO", theme),
            body_text(format!("«{}»", c.name), theme.fg_text, theme),
            spacer(6.0),
            button_view(format!("edad: {}", c.age.label()), &btn, Msg::CycleCharAge),
        ],
        1 => color_tools("PIEL", Part::Skin, c.skin, &sp, theme),
        2 => color_tools("CAMISETA", Part::Shirt, c.shirt, &sp, theme),
        _ => color_tools("PANTALÓN", Part::Pants, c.pants, &sp, theme),
    }
}

/// Set de sliders R/G/B de una parte coloreable del personaje.
fn color_tools(
    title: &str,
    part: Part,
    rgb: [f32; 3],
    sp: &SliderPalette,
    theme: &Theme,
) -> Vec<View<Msg>> {
    let mut v = vec![section_title(title, theme)];
    for (ch, label) in [(0usize, "rojo"), (1, "verde"), (2, "azul")] {
        let value = rgb[ch];
        v.push(slider_view(label, value, 0.0, 1.0, sp, move |_p, dv| {
            Some(Msg::SetColor(part, ch, value + dv))
        }));
    }
    v
}

/// Línea de texto de cuerpo (multi-línea) para los paneles.
fn body_text(s: String, color: Color, _theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0), height: Dimension::auto() },
        ..Default::default()
    })
    .text(s, 13.0, color)
    .max_lines(2)
}

// =============================================================================
//  Helpers de layout
// =============================================================================

fn section_title(text: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0), height: length(20.0) },
        ..Default::default()
    })
    .text(text.to_string(), 12.0, theme.accent)
    .bold()
}

fn spacer(h: f32) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0), height: length(h) },
        ..Default::default()
    })
}

fn pad(x: f32, y: f32) -> llimphi_ui::llimphi_layout::taffy::prelude::Rect<
    llimphi_ui::llimphi_layout::taffy::prelude::LengthPercentage,
> {
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
    let s = ron::ser::to_string_pretty(project, ron::ser::PrettyConfig::default())
        .map_err(|e| e.to_string())?;
    std::fs::write(PROJECT_PATH, s).map_err(|e| e.to_string())
}

fn load_project() -> Result<Project, String> {
    let s = std::fs::read_to_string(PROJECT_PATH).map_err(|e| e.to_string())?;
    ron::from_str(&s).map_err(|e| e.to_string())
}

/// Estado de arranque (sin `Handle`) — reusado por `init` y por el pantallazo.
pub(crate) fn demo_model() -> Model {
    Model {
        theme: Theme::dark(),
        project: Project::starter(),
        sel: 0,
        gen: 1,
        yaw: 35_f32.to_radians(),
        pitch: 26_f32.to_radians(),
        dist: default_dist(),
        preview: Arc::new(Mutex::new(None)),
        status: "proyecto de arranque · desierto + pradera".into(),
        ai_input: TextInputState::new(),
        ai_focused: false,
        ai_busy: false,
        mode: Mode::Worlds,
        scene_sel: 0,
        char_sel: 0,
        time: 0.0,
        playing: false,
        script_cam: true,
        exporting: false,
        tool_tab: 0,
    }
}

fn main() {
    if std::env::args().any(|a| a == "--shot") {
        shot::shot();
        return;
    }
    if std::env::args().any(|a| a == "--flythrough") {
        // Vuelo sobre un mundo infinito (para el GIF del README): relieve dramático
        // (montañas + cumbres nevadas) para que el vuelo tenga de qué — no el
        // desierto llano.
        // Dunas grandes en TODO el ancho (sin gating de montañas que deja valles
        // planos que se ven cielo) → relieve uniforme que llena el cuadro al volar.
        let recipe = WorldRecipe {
            base: 0.40,
            dune: 0.30,
            relief: 0.35,
            mountains: 0.3,
            water_level: 0.05,
            rivers: 0.1,
            peak_at: 0.92, // poca nieve
            ..WorldRecipe::grassland(11)
        };
        match render::flythrough(&recipe) {
            Ok(out) => eprintln!("flythrough ok: {out}"),
            Err(e) => eprintln!("flythrough error: {e}"),
        }
        return;
    }
    if std::env::args().any(|a| a == "--turntable") {
        // Vitrina del motor voxel: orbita un mundo (para el GIF del README).
        let p = Project::starter();
        let recipe = p.worlds.first().map(|w| w.recipe).unwrap_or_else(|| WorldRecipe::desert(1));
        match render::turntable(&recipe) {
            Ok(out) => eprintln!("turntable ok: {out}"),
            Err(e) => eprintln!("turntable error: {e}"),
        }
        return;
    }
    if std::env::args().any(|a| a == "--export") {
        // Certificación headless del pipeline de video: exporta la escena demo.
        let p = Project::starter();
        match p.scenes.first() {
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
