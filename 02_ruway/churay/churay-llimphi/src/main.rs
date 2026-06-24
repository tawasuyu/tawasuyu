//! churay — el instalador/actualizador gráfico de la suite tawasuyu, estilo
//! Office: elegís apps de un catálogo por cuadrante, modo **sistema** (root) o
//! **local** (`~/.local`), clic en Instalar y barra de progreso por app.
//!
//! Toda la lógica vive en `churay-core` (frontend-agnóstico). Acá sólo el
//! bucle Elm de Llimphi: estado + vista + worker de instalación.

use churay_core::install::Step;
use churay_core::{
    install_unit, pending_updates, InstallConfig, InstallMode, InstalledState, Manifest, Unit,
    UpdateKind,
};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_progress::linear_progress_view;
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};
use llimphi_widget_switch::{switch_view, SwitchPalette};

const ROW_H: f32 = 58.0;
const VIEWPORT: f32 = 392.0;

/// Estado de instalación de una unidad concreta.
#[derive(Clone, PartialEq)]
enum UnitStatus {
    Idle,
    Working(f32),
    Done,
    Failed(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Catalogo,
    Actualizaciones,
}

struct Model {
    theme: Theme,
    mode: InstallMode,
    cfg: InstallConfig,
    units: Vec<Unit>,
    selected: Vec<bool>,
    status: Vec<UnitStatus>,
    state: InstalledState,
    tab: Tab,
    scroll: f32,
    installing: bool,
}

#[derive(Clone)]
enum Msg {
    SetMode(InstallMode),
    Toggle(usize),
    SeleccionarTodo(bool),
    Scroll(f32),
    Tab(Tab),
    Instalar,
    Progress(String, Step, f32),
    UnitDone(String),
    UnitFailed(String, String),
    AllDone,
    ReexecRoot,
}

struct Churay;

impl Model {
    /// Índices de unidades visibles en el modo actual (local oculta `System`).
    fn visibles(&self) -> Vec<usize> {
        self.units
            .iter()
            .enumerate()
            .filter(|(_, u)| self.mode.admits(u.scope))
            .map(|(i, _)| i)
            .collect()
    }

    fn content_len(&self) -> f32 {
        self.visibles().len() as f32 * ROW_H
    }

    fn reload_state(&mut self) {
        self.state = InstalledState::load(&self.cfg.prefix);
    }
}

impl App for Churay {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "churay · instalar tawasuyu"
    }

    fn initial_size() -> (u32, u32) {
        (780, 640)
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        // El modo puede venir forzado por el re-exec con root.
        let mode = match std::env::var("CHURAY_MODE").as_deref() {
            Ok("system") => InstallMode::System,
            _ => InstallMode::Local,
        };
        let cfg = InstallConfig::detect(mode);
        let units = churay_core::suite_catalog();
        let n = units.len();
        let state = InstalledState::load(&cfg.prefix);
        Model {
            theme: Theme::dark(),
            mode,
            cfg,
            units,
            selected: vec![false; n],
            status: vec![UnitStatus::Idle; n],
            state,
            tab: Tab::Catalogo,
            scroll: 0.0,
            installing: false,
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::SetMode(m) => {
                model.mode = m;
                model.cfg = InstallConfig::detect(m);
                model.reload_state();
                model.scroll = 0.0;
            }
            Msg::Toggle(i) => {
                if let Some(s) = model.selected.get_mut(i) {
                    *s = !*s;
                }
            }
            Msg::SeleccionarTodo(v) => {
                for i in model.visibles() {
                    model.selected[i] = v;
                }
            }
            Msg::Scroll(d) => {
                model.scroll = clamp_offset(model.scroll + d, model.content_len(), VIEWPORT);
            }
            Msg::Tab(t) => model.tab = t,
            Msg::Instalar => {
                if model.installing {
                    return model;
                }
                let unidades: Vec<Unit> = model
                    .visibles()
                    .into_iter()
                    .filter(|&i| model.selected[i])
                    .map(|i| model.units[i].clone())
                    .collect();
                if unidades.is_empty() {
                    return model;
                }
                for i in model.visibles() {
                    if model.selected[i] {
                        model.status[i] = UnitStatus::Working(0.0);
                    }
                }
                model.installing = true;
                let cfg = model.cfg.clone();
                let h = handle.clone();
                handle.spawn(move || {
                    let mut state = InstalledState::load(&cfg.prefix);
                    for u in &unidades {
                        let id = u.id.clone();
                        let hp = h.clone();
                        let res = install_unit(&cfg, u, &mut state, &mut |step, r| {
                            hp.dispatch(Msg::Progress(id.clone(), step, r));
                        });
                        match res {
                            Ok(()) => h.dispatch(Msg::UnitDone(u.id.clone())),
                            Err(e) => h.dispatch(Msg::UnitFailed(u.id.clone(), e.to_string())),
                        }
                    }
                    Msg::AllDone
                });
            }
            Msg::Progress(id, _step, r) => {
                if let Some(i) = model.units.iter().position(|u| u.id == id) {
                    if model.status[i] != UnitStatus::Done {
                        model.status[i] = UnitStatus::Working(r);
                    }
                }
            }
            Msg::UnitDone(id) => {
                if let Some(i) = model.units.iter().position(|u| u.id == id) {
                    model.status[i] = UnitStatus::Done;
                }
            }
            Msg::UnitFailed(id, err) => {
                if let Some(i) = model.units.iter().position(|u| u.id == id) {
                    model.status[i] = UnitStatus::Failed(err);
                }
            }
            Msg::AllDone => {
                model.installing = false;
                model.reload_state();
            }
            Msg::ReexecRoot => {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new("pkexec")
                        .arg(exe)
                        .env("CHURAY_MODE", "system")
                        .spawn();
                    handle.quit();
                }
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let t = &model.theme;
        let body = match model.tab {
            Tab::Catalogo => catalogo(model),
            Tab::Actualizaciones => actualizaciones(model),
        };
        col(percent(1.0), percent(1.0))
            .fill(t.bg_app)
            .children(vec![header(model), body, footer(model)])
    }
}

// ---------- secciones ----------

fn header(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let titulo = View::new(Style {
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned("Instalar tawasuyu", 24.0, t.fg_text, Alignment::Start);

    let mut hijos = vec![
        titulo,
        chip("Sistema", model.mode == InstallMode::System, Msg::SetMode(InstallMode::System), t),
        chip("Sólo para mí", model.mode == InstallMode::Local, Msg::SetMode(InstallMode::Local), t),
    ];

    // Tabs.
    let tabs = row(percent(1.0), length(34.0)).gap(8.0).children(vec![
        tabchip("Catálogo", model.tab == Tab::Catalogo, Msg::Tab(Tab::Catalogo), t),
        tabchip(
            "Actualizaciones",
            model.tab == Tab::Actualizaciones,
            Msg::Tab(Tab::Actualizaciones),
            t,
        ),
    ]);

    let barra = row(percent(1.0), length(44.0))
        .gap(10.0)
        .children(std::mem::take(&mut hijos));

    let mut secciones = vec![barra, tabs];

    // Aviso de root cuando el modo Sistema no es escribible.
    if model.mode == InstallMode::System && model.cfg.needs_root() {
        secciones.push(banner_root(t));
    }

    col(percent(1.0), auto())
        .gap(10.0)
        .pad(20.0)
        .fill(t.bg_panel)
        .children(secciones)
}

fn banner_root(t: &Theme) -> View<Msg> {
    let txt = View::new(Style {
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        "El modo Sistema escribe en /usr/local — hace falta root.",
        14.0,
        t.fg_text,
        Alignment::Start,
    );
    let boton = View::new(Style {
        size: Size { width: length(170.0), height: length(30.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(t.accent)
    .radius(8.0)
    .text("Reabrir como root", 13.0, t.bg_app)
    .on_click(Msg::ReexecRoot);
    row(percent(1.0), length(40.0))
        .gap(10.0)
        .pad(8.0)
        .fill(t.bg_input)
        .radius(8.0)
        .children(vec![txt, boton])
}

fn catalogo(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let vis = model.visibles();
    let filas: Vec<View<Msg>> = vis.iter().map(|&i| fila(model, i)).collect();
    let lista = col(percent(1.0), auto())
        .children(filas);

    let scroller = scroll_y(
        model.scroll,
        model.content_len(),
        VIEWPORT,
        lista,
        Msg::Scroll,
        &ScrollPalette::from_theme(t),
    );

    let wrap = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0), height: length(VIEWPORT) },
        ..Default::default()
    })
    .clip(true)
    .children(vec![scroller]);

    col(percent(1.0), length(VIEWPORT))
        .pad(16.0)
        .children(vec![wrap])
}

fn fila(model: &Model, i: usize) -> View<Msg> {
    let t = &model.theme;
    let u = &model.units[i];
    let sel = model.selected[i];
    let instalada = model.state.is_installed(&u.id);

    let sw = switch_view(
        if sel { 1.0 } else { 0.0 },
        Msg::Toggle(i),
        &SwitchPalette::from_theme(t),
    );
    let sw_wrap = View::new(Style {
        size: Size { width: length(54.0), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![sw]);

    let icono = View::new(Style {
        size: Size { width: length(40.0), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(u.icon.clone(), 22.0, t.fg_text);

    let titulo = View::new(Style {
        size: Size { width: percent(1.0), height: length(22.0) },
        ..Default::default()
    })
    .text_aligned(u.label.as_str(), 16.0, t.fg_text, Alignment::Start);
    let desc = View::new(Style {
        size: Size { width: percent(1.0), height: length(18.0) },
        ..Default::default()
    })
    .text_aligned(u.description.as_str(), 12.0, t.fg_muted, Alignment::Start);
    let medio = col(percent(1.0), auto())
        .gap(2.0)
        .grow()
        .children(vec![titulo, desc]);

    let estado = estado_view(model, i, instalada, t);

    row(percent(1.0), length(ROW_H))
        .gap(8.0)
        .pad_x(6.0)
        .children(vec![sw_wrap, icono, medio, estado])
}

fn estado_view(model: &Model, i: usize, instalada: bool, t: &Theme) -> View<Msg> {
    let (txt, color) = match &model.status[i] {
        UnitStatus::Working(r) => (format!("{}%", (r * 100.0) as u32), t.accent),
        UnitStatus::Done => ("✓ instalada".to_string(), t.accent),
        UnitStatus::Failed(_) => ("✗ falló".to_string(), t.fg_destructive),
        UnitStatus::Idle => {
            if instalada {
                ("instalada".to_string(), t.fg_muted)
            } else {
                (model.units[i].version.clone(), t.fg_muted)
            }
        }
    };
    View::new(Style {
        size: Size { width: length(110.0), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(txt, 13.0, color)
}

fn actualizaciones(model: &Model) -> View<Msg> {
    let t = &model.theme;
    // Sin manifiesto remoto todavía: diagnóstico contra el catálogo local.
    let manifest = Manifest::new(churay_core::SUITE_VERSION, model.units.clone());
    let pend = pending_updates(&model.state, &manifest);
    let instaladas: Vec<_> = pend.iter().filter(|u| u.kind != UpdateKind::Nueva).collect();

    let mut hijos: Vec<View<Msg>> = Vec::new();
    if model.state.units.is_empty() {
        hijos.push(linea("Todavía no instalaste nada.", t.fg_muted, t));
    } else {
        for (id, inst) in model.state.units.iter() {
            let label = model.units.iter().find(|u| &u.id == id).map(|u| u.label.clone()).unwrap_or_else(|| id.clone());
            let disponible = instaladas.iter().any(|u| &u.id == id);
            let txt = if disponible {
                format!("{label} — {}  ·  hay actualización", inst.version)
            } else {
                format!("{label} — {}  ·  al día", inst.version)
            };
            let color = if disponible { t.accent } else { t.fg_text };
            hijos.push(linea(&txt, color, t));
        }
    }
    hijos.push(linea(
        "El chequeo contra un repositorio remoto firmado llega con el bundle de release.",
        t.fg_placeholder,
        t,
    ));

    col(percent(1.0), length(VIEWPORT))
        .pad(20.0)
        .gap(8.0)
        .children(hijos)
}

fn footer(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let n_sel = model
        .visibles()
        .into_iter()
        .filter(|&i| model.selected[i])
        .count();

    let resumen = View::new(Style { flex_grow: 1.0, ..Default::default() }).text_aligned(
        format!("{n_sel} seleccionada(s) · destino: {}", model.cfg.prefix.display()),
        13.0,
        t.fg_muted,
        Alignment::Start,
    );

    let acciones = row(auto(), length(34.0))
        .gap(8.0)
        .children(vec![
            View::new(Style {
                size: Size { width: length(90.0), height: length(34.0) },
                ..Default::default()
            })
            .children(vec![button_view("Todo", &ButtonPalette::from_theme(t), Msg::SeleccionarTodo(true))]),
            View::new(Style {
                size: Size { width: length(90.0), height: length(34.0) },
                ..Default::default()
            })
            .children(vec![button_view("Nada", &ButtonPalette::from_theme(t), Msg::SeleccionarTodo(false))]),
            instalar_boton(model, n_sel, t),
        ]);

    let mut hijos = vec![row(percent(1.0), length(34.0)).gap(10.0).children(vec![resumen, acciones])];

    if model.installing {
        let total = model.units.len().max(1) as f32;
        let hechas = model
            .status
            .iter()
            .filter(|s| matches!(s, UnitStatus::Done | UnitStatus::Failed(_)))
            .count() as f32;
        hijos.push(linear_progress_view(hechas / total, t.bg_input, t.accent, 6.0));
    }

    col(percent(1.0), auto())
        .gap(10.0)
        .pad(20.0)
        .fill(t.bg_panel)
        .children(hijos)
}

fn instalar_boton(model: &Model, n_sel: usize, t: &Theme) -> View<Msg> {
    let activo = n_sel > 0 && !model.installing;
    let label = if model.installing { "Instalando…" } else { "Instalar" };
    let bg = if activo { t.accent } else { t.bg_button };
    let fg = if activo { t.bg_app } else { t.fg_muted };
    let mut v = View::new(Style {
        size: Size { width: length(150.0), height: length(34.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(8.0)
    .text(label, 15.0, fg);
    if activo {
        v = v.on_click(Msg::Instalar);
    }
    v
}

// ---------- helpers de layout / chips ----------

fn col(w: Dimension, h: Dimension) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: w, height: h },
        ..Default::default()
    })
}

fn row(w: Dimension, h: Dimension) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: w, height: h },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
}

fn linea(txt: &str, color: Color, _t: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0), height: length(24.0) },
        ..Default::default()
    })
    .text_aligned(txt, 14.0, color, Alignment::Start)
}

fn chip(label: &str, active: bool, msg: Msg, t: &Theme) -> View<Msg> {
    let bg = if active { t.accent } else { t.bg_button };
    let fg = if active { t.bg_app } else { t.fg_text };
    View::new(Style {
        size: Size { width: length(120.0), height: length(34.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(8.0)
    .text(label, 14.0, fg)
    .on_click(msg)
}

fn tabchip(label: &str, active: bool, msg: Msg, t: &Theme) -> View<Msg> {
    let fg = if active { t.fg_text } else { t.fg_muted };
    let bg = if active { t.bg_input } else { t.bg_panel };
    View::new(Style {
        size: Size { width: length(160.0), height: length(30.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(6.0)
    .text(label, 13.0, fg)
    .on_click(msg)
}

// pequeñas comodidades de builder sobre View
trait ViewExt {
    fn gap(self, g: f32) -> Self;
    fn pad(self, p: f32) -> Self;
    fn pad_x(self, p: f32) -> Self;
    fn grow(self) -> Self;
}
impl ViewExt for View<Msg> {
    fn gap(mut self, g: f32) -> Self {
        self.style.gap = Size { width: length(g), height: length(g) };
        self
    }
    fn pad(mut self, p: f32) -> Self {
        self.style.padding = Rect { left: length(p), right: length(p), top: length(p), bottom: length(p) };
        self
    }
    fn pad_x(mut self, p: f32) -> Self {
        self.style.padding = Rect { left: length(p), right: length(p), top: length(0.0), bottom: length(0.0) };
        self
    }
    fn grow(mut self) -> Self {
        self.style.flex_grow = 1.0;
        self
    }
}

fn main() {
    llimphi_ui::run::<Churay>();
}
