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
use llimphi_voxel::{world_dim, Project, WorldRecipe, PREVIEW_DIM_XZ};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

mod ai;
mod preview;
mod shot;
use preview::WorldPreview;

/// Dónde se guarda/carga el proyecto (relativo al cwd).
const PROJECT_PATH: &str = "voxel-studio.ron";

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
}

impl Model {
    /// Receta del mundo seleccionado (si hay).
    fn recipe(&self) -> Option<WorldRecipe> {
        self.project.worlds.get(self.sel).map(|w| w.recipe)
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

    fn init(_handle: &Handle<Msg>) -> Model {
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

/// Panel izquierdo: lista de mundos + acciones (nuevo/guardar/cargar) + estado.
fn left_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let btn = ButtonPalette::from_theme(theme);

    let mut rows: Vec<View<Msg>> = Vec::new();
    rows.push(section_title("MUNDOS", theme));
    for (i, w) in model.project.worlds.iter().enumerate() {
        let selected = i == model.sel;
        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0), height: length(30.0) },
                align_items: Some(AlignItems::Center),
                padding: pad(10.0, 0.0),
                ..Default::default()
            })
            .fill(if selected { theme.bg_selected } else { theme.bg_panel })
            .radius(6.0)
            .text(
                w.name.clone(),
                15.0,
                if selected { theme.fg_text } else { theme.fg_muted },
            )
            .on_click(Msg::Select(i)),
        );
    }

    rows.push(spacer(10.0));
    rows.push(button_view("nuevo mundo", &btn, Msg::NewWorld));
    rows.push(spacer(6.0));
    rows.push(button_view("guardar", &btn, Msg::Save));
    rows.push(spacer(6.0));
    rows.push(button_view("cargar", &btn, Msg::Load));

    // Sección IA: describir un mundo en prosa y generarlo.
    rows.push(spacer(14.0));
    rows.push(section_title("IA — DESCRIBÍ UN MUNDO", theme));
    rows.push(text_input_view(
        &model.ai_input,
        "p.ej. islas con ríos y nieve",
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

    rows.push(spacer(12.0));
    rows.push(
        View::new(Style {
            size: Size { width: percent(1.0), height: Dimension::auto() },
            ..Default::default()
        })
        .text(model.status.clone(), 12.0, theme.fg_placeholder)
        .max_lines(3),
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(210.0), height: percent(1.0) },
        padding: pad(14.0, 14.0),
        gap: gap_y(6.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(rows)
}

/// Centro: el canvas 3D del preview en vivo, draggable para orbitar.
fn center_canvas(model: &Model) -> View<Msg> {
    let (yaw, pitch, dist, gen) = (model.yaw, model.pitch, model.dist, model.gen);
    let recipe = model.recipe().unwrap_or_else(|| WorldRecipe::grassland(1));
    let preview = model.preview.clone();

    let canvas = View::new(Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0), height: percent(1.0) },
        ..Default::default()
    })
    .gpu_paint_with(move |device, queue, encoder, target, rect, vp| {
        let dim = world_dim(PREVIEW_DIM_XZ);
        let mut guard = preview.lock().unwrap();
        let p = guard.get_or_insert_with(|| WorldPreview::build(device, queue, &recipe, dim, gen));
        p.rebuild_if(device, queue, &recipe, dim, gen);
        // Cámara de órbita centrada en el mundo (grilla [0,dim]).
        let center = Vec3::new(dim[0] as f32 * 0.5, dim[1] as f32 * 0.32, dim[2] as f32 * 0.5);
        let camera = Camera3d::orbit(center, yaw, pitch, dist);
        p.render(device, queue, encoder, target, vp, (rect.x, rect.y, rect.w, rect.h), &camera);
    })
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

/// Panel derecho: los sliders + botones de ciclo de la receta seleccionada.
fn right_panel(model: &Model) -> View<Msg> {
    let theme = &model.theme;
    let sp = SliderPalette::from_theme(theme);
    let btn = ButtonPalette::from_theme(theme);

    let Some(r) = model.recipe() else {
        return View::new(Style {
            size: Size { width: length(280.0), height: percent(1.0) },
            padding: pad(16.0, 16.0),
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .text("sin mundo seleccionado", 14.0, theme.fg_muted);
    };

    let slider = |label: &str, value: f32, min: f32, max: f32, field: Field| -> View<Msg> {
        slider_view(label, value, min, max, &sp, move |phase, dv| {
            let _ = phase;
            Some(Msg::Set(field, value + dv))
        })
    };

    let children = vec![
        section_title("RELIEVE", theme),
        slider("semilla", r.seed as f32, 0.0, 64.0, Field::Seed),
        slider("base (llanura)", r.base, 0.0, 0.9, Field::Base),
        slider("dunas", r.dune, 0.0, 0.4, Field::Dune),
        slider("relieve (alto montañas)", r.relief, 0.0, 1.0, Field::Relief),
        slider("densidad montañas", r.mountains, 0.0, 1.0, Field::Mountains),
        spacer(8.0),
        section_title("AGUA", theme),
        slider("nivel del agua", r.water_level, 0.0, 0.9, Field::Water),
        slider("densidad ríos", r.rivers, 0.0, 1.0, Field::Rivers),
        spacer(8.0),
        section_title("MATERIALES", theme),
        button_view(format!("suelo: {}", r.ground.label()), &btn, Msg::CycleGround),
        spacer(4.0),
        button_view(format!("acantilado: {}", r.cliff.label()), &btn, Msg::CycleCliff),
        spacer(4.0),
        button_view(format!("cumbre: {}", r.peak.label()), &btn, Msg::CyclePeak),
        slider("altura cumbre", r.peak_at, 0.0, 1.0, Field::PeakAt),
        spacer(8.0),
        section_title("FLORA", theme),
        button_view(format!("tipo: {}", r.flora.label()), &btn, Msg::CycleFlora),
        slider("densidad flora", r.flora_density, 0.0, 0.05, Field::Flora),
    ];

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(280.0), height: percent(1.0) },
        padding: pad(16.0, 16.0),
        gap: gap_y(8.0),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(children)
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
    }
}

fn main() {
    if std::env::args().any(|a| a == "--shot") {
        shot::shot();
        return;
    }
    llimphi_ui::run::<Studio>();
}
