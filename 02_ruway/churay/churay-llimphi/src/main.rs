// En release sobre Windows: subsistema GUI (sin consola negra detrás).
// No-op en Linux/otros targets — preserva `cargo check --workspace`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! churay — el instalador/actualizador gráfico de la suite tawasuyu, estilo
//! Office: elegís apps de un catálogo por cuadrante, modo **sistema** (root) o
//! **local** (`~/.local`), clic en Instalar y barra de progreso por app.
//!
//! Toda la lógica vive en `churay-core` (frontend-agnóstico). Acá sólo el
//! bucle Elm de Llimphi: estado + vista + worker de instalación.

use churay_core::install::Step;
use churay_core::{
    install_unit, pending_updates, source_kind, InstallConfig, InstallMode, InstalledState,
    Manifest, SourceKind, Unit, UpdateKind,
};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_image::Image;
use llimphi_theme::motion;
use llimphi_ui::llimphi_raster::kurbo::Affine;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use marca::Brand;
use llimphi_icons::Icon;
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_empty::{empty_view, EmptyPalette};
use llimphi_widget_progress::linear_progress_view;
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};
use llimphi_widget_switch::{switch_view, SwitchPalette};

use std::hash::{Hash, Hasher};

const ROW_H: f32 = 58.0;
const VIEWPORT: f32 = 392.0;

/// Hash estable de una cadena → `key` para animaciones implícitas (la misma
/// escena/ítem produce siempre la misma key entre rebuilds).
fn key_of(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// `key` estable de la escena actual (pantalla + pestaña). Cambia sólo al pasar
/// de una vista a otra → dispara la transición de entrada del contenido.
fn scene_key(model: &Model) -> u64 {
    let s = match model.screen {
        Screen::Bienvenida => "bienvenida",
        Screen::Resultado => "resultado",
        Screen::SistemaBase => "sistema-base",
        Screen::Catalogo => match model.tab {
            Tab::Catalogo => "catalogo",
            Tab::Actualizaciones => "actualizaciones",
        },
    };
    key_of(s)
}

/// Estado de instalación de una unidad concreta.
#[derive(Clone, PartialEq)]
enum UnitStatus {
    Idle,
    Working(Step, f32),
    Done,
    Failed(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Catalogo,
    Actualizaciones,
}

/// Pantalla actual: bienvenida (branding) → catálogo → resultado.
#[derive(Clone, Copy, PartialEq)]
enum Screen {
    Bienvenida,
    Catalogo,
    Resultado,
    SistemaBase,
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
    /// Manifiesto del repo remoto, una vez bajado y verificado.
    remote_manifest: Option<Manifest>,
    /// Mensaje de estado del repo remoto (resultado del último chequeo).
    repo_msg: String,
    buscando: bool,
    /// Pantalla actual.
    screen: Screen,
    /// Logo de la suite (de `marca`), decodificado una vez.
    logo: Option<Image>,
    /// Checkbox de la portada: no volver a mostrarla.
    skip_welcome: bool,
    /// Componentes del sistema base (compositor/DM mirada) — no son apps.
    base: Vec<churay_core::Component>,
    base_sel: Vec<bool>,
    base_status: Vec<UnitStatus>,
    base_installing: bool,
    base_done: bool,
    /// Contraseña de sudo tecleada in-app (se borra al usarla).
    password: String,
    /// Error de la instalación elevada (contraseña, etc.).
    base_error: Option<String>,
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
    BuscarRemoto,
    RemotoListo(Result<Manifest, String>),
    Comenzar,
    AgregarSugeridas,
    ToggleSkipWelcome,
    Lanzar(String),
    VolverCatalogo,
    InstalarSugeridasResultado,
    Cerrar,
    AbrirSistemaBase,
    ToggleBaseComp(usize),
    InstalarBase,
    BaseProgress(String, f32),
    BaseCompDone(String),
    BaseAllDone,
    BaseError(String),
    PassType(String),
    PassBackspace,
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

    /// Índices de unidades **sugeridas** por las seleccionadas que todavía no
    /// están marcadas (y son visibles en el modo). Para el banner "Agregar".
    fn sugeridas_faltantes(&self) -> Vec<usize> {
        let vis = self.visibles();
        let seleccionadas: Vec<&str> = vis
            .iter()
            .filter(|&&i| self.selected[i])
            .map(|&i| self.units[i].id.as_str())
            .collect();
        let mut faltan = Vec::new();
        for &i in &vis {
            if self.selected[i] {
                continue;
            }
            let id = &self.units[i].id;
            let sugerida = vis.iter().any(|&j| {
                self.selected[j] && self.units[j].suggests.iter().any(|s| s == id)
            });
            // Evitá repetir las ya seleccionadas.
            if sugerida && !seleccionadas.contains(&id.as_str()) {
                faltan.push(i);
            }
        }
        faltan
    }

    /// `true` si instalar el sistema base hace falta root (prefix no escribible).
    fn base_needs_root(&self) -> bool {
        InstallConfig::detect(InstallMode::System).needs_root()
    }

    /// Índices de unidades **sugeridas por las ya instaladas** que todavía no se
    /// instalaron — para la pantalla de resultado ("te sugerimos también…").
    fn sugeridas_de_instaladas(&self) -> Vec<usize> {
        let sugeridas: Vec<String> = self
            .units
            .iter()
            .filter(|u| self.state.is_installed(&u.id))
            .flat_map(|u| u.suggests.iter().cloned())
            .collect();
        self.units
            .iter()
            .enumerate()
            .filter(|(_, u)| {
                self.mode.admits(u.scope)
                    && !self.state.is_installed(&u.id)
                    && sugeridas.contains(&u.id)
            })
            .map(|(i, _)| i)
            .collect()
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

    /// En la pantalla del sistema base, cuando hace falta root, tecleás la
    /// contraseña de sudo directamente (campo enmascarado in-app).
    fn on_key(model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if model.screen != Screen::SistemaBase || !model.base_needs_root() || model.base_installing {
            return None;
        }
        if !matches!(event.state, KeyState::Pressed) {
            return None;
        }
        match &event.key {
            Key::Named(NamedKey::Backspace) => Some(Msg::PassBackspace),
            Key::Named(NamedKey::Enter) => Some(Msg::InstalarBase),
            _ => match &event.text {
                Some(s) if !s.is_empty() && !s.chars().any(|c| c.is_control()) => {
                    Some(Msg::PassType(s.clone()))
                }
                _ => None,
            },
        }
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
        let prefs = churay_core::Prefs::load();
        // La portada se salta si el usuario ya lo pidió (es también actualizador).
        let screen = if prefs.skip_welcome { Screen::Catalogo } else { Screen::Bienvenida };
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
            remote_manifest: None,
            repo_msg: String::new(),
            buscando: false,
            screen,
            logo: llimphi_image::decode_bytes(&Brand::Suite.image()).ok(),
            // El checkbox de la portada arranca activado ("no mostrar de nuevo").
            skip_welcome: true,
            base: {
                let b = churay_core::base_system();
                b
            },
            base_sel: vec![true; churay_core::base_system().len()],
            base_status: vec![UnitStatus::Idle; churay_core::base_system().len()],
            base_installing: false,
            base_done: false,
            password: String::new(),
            base_error: None,
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
                        model.status[i] = UnitStatus::Working(Step::Resolviendo, 0.0);
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
            Msg::Progress(id, step, r) => {
                if let Some(i) = model.units.iter().position(|u| u.id == id) {
                    if model.status[i] != UnitStatus::Done {
                        model.status[i] = UnitStatus::Working(step, r);
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
                // Mostrá el resultado: qué quedó instalado y cómo abrirlo.
                model.screen = Screen::Resultado;
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
            Msg::BuscarRemoto => {
                let Some(url) = model.cfg.remote_base_url.clone() else {
                    model.repo_msg = "No hay repo remoto configurado (CHURAY_REPO).".into();
                    return model;
                };
                model.buscando = true;
                model.repo_msg = format!("Consultando {url}…");
                handle.spawn(move || {
                    let res = churay_core::fetch_signed_manifest(
                        &url,
                        &churay_core::CurlFetcher,
                        None,
                    )
                    .map_err(|e| e.to_string());
                    Msg::RemotoListo(res)
                });
            }
            Msg::RemotoListo(res) => {
                model.buscando = false;
                match res {
                    Ok(m) => {
                        let pend = pending_updates(&model.state, &m)
                            .into_iter()
                            .filter(|u| u.kind != churay_core::UpdateKind::Nueva)
                            .count();
                        model.repo_msg = format!(
                            "Repo {} · {} unidad(es) · {} con actualización",
                            m.suite_version,
                            m.units.len(),
                            pend
                        );
                        model.remote_manifest = Some(m);
                    }
                    Err(e) => model.repo_msg = format!("Falló el chequeo: {e}"),
                }
            }
            Msg::Comenzar => {
                if model.skip_welcome {
                    let _ = (churay_core::Prefs { skip_welcome: true }).save();
                }
                model.screen = Screen::Catalogo;
            }
            Msg::ToggleSkipWelcome => model.skip_welcome = !model.skip_welcome,
            Msg::AgregarSugeridas => {
                for i in model.sugeridas_faltantes() {
                    model.selected[i] = true;
                }
            }
            Msg::Lanzar(program) => {
                let bin = model.cfg.prefix.join("bin").join(&program);
                let _ = std::process::Command::new(bin).spawn();
            }
            Msg::VolverCatalogo => {
                for s in model.status.iter_mut() {
                    *s = UnitStatus::Idle;
                }
                model.screen = Screen::Catalogo;
            }
            Msg::Cerrar => handle.quit(),
            Msg::AbrirSistemaBase => model.screen = Screen::SistemaBase,
            Msg::ToggleBaseComp(i) => {
                if let Some(s) = model.base_sel.get_mut(i) {
                    *s = !*s;
                }
            }
            Msg::InstalarBase => {
                if model.base_installing {
                    return model;
                }
                let sel_ids: Vec<String> = model
                    .base
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| model.base_sel[*i])
                    .map(|(_, c)| c.id.to_string())
                    .collect();
                if sel_ids.is_empty() {
                    return model;
                }
                let needs_root = model.base_needs_root();
                // Si hace falta root y todavía no hay contraseña, esperá a que la
                // tecleen (el campo está en pantalla). No cierres la app.
                if needs_root && model.password.is_empty() {
                    model.base_error = Some("Escribí tu contraseña de sudo abajo.".into());
                    return model;
                }
                for i in 0..model.base.len() {
                    if model.base_sel[i] {
                        model.base_status[i] = UnitStatus::Working(Step::Resolviendo, 0.0);
                    }
                }
                model.base_installing = true;
                model.base_done = false;
                model.base_error = None;
                let cfg = InstallConfig::detect(InstallMode::System);
                let h = handle.clone();
                if needs_root {
                    // Elevación in-app: corremos el modo headless del propio binario
                    // bajo `sudo -S`, pasándole la contraseña por stdin. La app NO
                    // se cierra; leemos el progreso por stdout.
                    let pass = std::mem::take(&mut model.password);
                    let ids = sel_ids.join(",");
                    handle.spawn(move || {
                        elevated_install(&cfg, &ids, &pass, &h);
                        Msg::BaseAllDone
                    });
                } else {
                    handle.spawn(move || {
                        let comps = churay_core::base_system();
                        let refs: Vec<&churay_core::Component> =
                            comps.iter().filter(|c| sel_ids.iter().any(|id| id == c.id)).collect();
                        let _ = churay_core::install_base(
                            std::path::Path::new("/"),
                            &cfg,
                            None,
                            &refs,
                            &mut |id, step, ratio| {
                                if step == Step::Hecho {
                                    h.dispatch(Msg::BaseCompDone(id.to_string()));
                                } else {
                                    h.dispatch(Msg::BaseProgress(id.to_string(), ratio));
                                }
                            },
                        );
                        Msg::BaseAllDone
                    });
                }
            }
            Msg::PassType(s) => model.password.push_str(&s),
            Msg::PassBackspace => {
                model.password.pop();
            }
            Msg::BaseError(e) => {
                model.base_installing = false;
                model.base_error = Some(e);
                for s in model.base_status.iter_mut() {
                    if matches!(s, UnitStatus::Working(_, _)) {
                        *s = UnitStatus::Idle;
                    }
                }
            }
            Msg::BaseProgress(id, ratio) => {
                if let Some(i) = model.base.iter().position(|c| c.id == id) {
                    if model.base_status[i] != UnitStatus::Done {
                        model.base_status[i] = UnitStatus::Working(Step::Copiando, ratio);
                    }
                }
            }
            Msg::BaseCompDone(id) => {
                if let Some(i) = model.base.iter().position(|c| c.id == id) {
                    model.base_status[i] = UnitStatus::Done;
                }
            }
            Msg::BaseAllDone => {
                model.base_installing = false;
                // Si hubo un error elevado (contraseña, etc.), no marques "listo".
                if model.base_error.is_some() {
                    return model;
                }
                model.base_done = true;
                // Lo que quedó a medias = falló (install_base corta en el 1er error).
                for s in model.base_status.iter_mut() {
                    if matches!(s, UnitStatus::Working(_, _)) {
                        *s = UnitStatus::Failed("no se pudo instalar".into());
                    }
                }
            }
            Msg::InstalarSugeridasResultado => {
                for i in model.sugeridas_de_instaladas() {
                    model.selected[i] = true;
                }
                for s in model.status.iter_mut() {
                    *s = UnitStatus::Idle;
                }
                model.screen = Screen::Catalogo;
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let t = &model.theme;
        let contenido = match model.screen {
            Screen::Bienvenida => bienvenida(model),
            Screen::Resultado => resultado(model),
            Screen::SistemaBase => sistema_base(model),
            Screen::Catalogo => {
                let body = match model.tab {
                    Tab::Catalogo => catalogo(model),
                    Tab::Actualizaciones => actualizaciones(model),
                };
                col(percent(1.0), percent(1.0))
                    .fill(t.bg_app)
                    .children(vec![header(model), body, footer(model)])
            }
        };
        // Transición de escena: al cambiar de pantalla o de pestaña el contenido
        // entra con un fade + leve slide-up en vez de saltar de golpe.
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(vec![contenido])
        .animated_enter_from(scene_key(model), motion::SLOW, Affine::translate((0.0, 24.0)))
    }
}

/// Pantalla introductoria: logo de la suite (de `marca`), nombre, tagline y un
/// botón para entrar al catálogo.
fn bienvenida(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let meta = Brand::Suite.meta();
    let accent = Color::from_rgba8(meta.accent[0], meta.accent[1], meta.accent[2], meta.accent[3]);

    let logo = match &model.logo {
        Some(img) => View::new(Style {
            size: Size { width: length(168.0), height: length(168.0) },
            ..Default::default()
        })
        .image(img.clone()),
        None => View::new(Style {
            size: Size { width: length(168.0), height: length(168.0) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(accent)
        .radius(24.0)
        .text("◆", 72.0, t.bg_app),
    };

    let nombre = View::new(Style {
        size: Size { width: percent(1.0), height: length(48.0) },
        ..Default::default()
    })
    .text_aligned(meta.name, 40.0, t.fg_text, Alignment::Center);
    let tagline = View::new(Style {
        size: Size { width: percent(1.0), height: length(28.0) },
        ..Default::default()
    })
    .text_aligned(meta.tagline, 16.0, t.fg_muted, Alignment::Center);

    let comenzar = View::new(Style {
        size: Size { width: length(220.0), height: length(46.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(accent)
    .radius(10.0)
    .text("Comenzar", 17.0, t.bg_app)
    .on_click(Msg::Comenzar);

    // "No mostrar de nuevo" — activado por defecto (es también actualizador).
    let sw = View::new(Style {
        size: Size { width: length(54.0), height: length(28.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![switch_view(
        if model.skip_welcome { 1.0 } else { 0.0 },
        Msg::ToggleSkipWelcome,
        &SwitchPalette::from_theme(t),
    )]);
    let chk_label = View::new(Style {
        size: Size { width: length(180.0), height: length(28.0) },
        ..Default::default()
    })
    .text_aligned("No mostrar de nuevo", 13.0, t.fg_muted, Alignment::Start)
    .on_click(Msg::ToggleSkipWelcome);
    let chk = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0), height: length(30.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(8.0), height: length(0.0) },
        ..Default::default()
    })
    .children(vec![sw, chk_label]);

    col(percent(1.0), percent(1.0))
        .fill(t.bg_app)
        .gap(18.0)
        .pad(40.0)
        .children(vec![
            View::new(Style { flex_grow: 1.0, ..Default::default() }),
            wrap_center(logo),
            nombre,
            tagline,
            wrap_center(comenzar),
            chk,
            View::new(Style { flex_grow: 1.0, ..Default::default() }),
        ])
}

/// Pantalla de resultado: qué se instaló, cómo abrirlo, qué falló, y
/// sugerencias de lo que combina con lo instalado.
fn resultado(model: &Model) -> View<Msg> {
    let t = &model.theme;

    let titulo = View::new(Style {
        size: Size { width: percent(1.0), height: length(40.0) },
        ..Default::default()
    })
    .text_aligned("Listo", 28.0, t.fg_text, Alignment::Start);

    let mut filas: Vec<View<Msg>> = vec![titulo];
    filas.push(linea(&instruccion_modo(model), t.fg_muted, t));

    // Unidades tocadas en esta corrida (Done / Failed).
    for (i, u) in model.units.iter().enumerate() {
        match &model.status[i] {
            UnitStatus::Done => filas.push(fila_resultado_ok(model, u, t)),
            UnitStatus::Failed(err) => filas.push(linea(
                &format!("✗ {} — {}", u.label, err),
                t.fg_destructive,
                t,
            )),
            _ => {}
        }
    }

    // Sugerencias de lo instalado.
    let sug = model.sugeridas_de_instaladas();
    if !sug.is_empty() {
        let nombres: Vec<&str> = sug.iter().map(|&i| model.units[i].label.as_str()).collect();
        let txt = View::new(Style { flex_grow: 1.0, ..Default::default() }).text_aligned(
            format!("Combinan con lo que instalaste: {}", nombres.join(", ")),
            14.0,
            t.fg_text,
            Alignment::Start,
        );
        let add = boton("Instalar sugeridas", t.accent, t.bg_app, 170.0, Msg::InstalarSugeridasResultado);
        filas.push(
            row(percent(1.0), length(40.0))
                .gap(10.0)
                .pad(8.0)
                .fill(t.bg_input)
                .radius(8.0)
                .children(vec![txt, add]),
        );
    }

    // Acciones finales.
    let volver = boton("Volver al catálogo", t.bg_button, t.fg_text, 180.0, Msg::VolverCatalogo);
    let cerrar = boton("Cerrar", t.bg_button, t.fg_text, 110.0, Msg::Cerrar);
    let acciones = row(percent(1.0), length(40.0))
        .gap(10.0)
        .justify(JustifyContent::End)
        .children(vec![volver, cerrar]);

    col(percent(1.0), percent(1.0))
        .fill(t.bg_app)
        .gap(10.0)
        .pad(28.0)
        .children(vec![
            col(percent(1.0), llimphi_ui::llimphi_layout::taffy::prelude::auto())
                .gap(8.0)
                .grow()
                .children(filas),
            acciones,
        ])
}

/// Fila de una unidad instalada OK: ✓, dónde quedó, instrucción si la necesita,
/// y botón Abrir sólo si es algo que se abre suelto.
fn fila_resultado_ok(model: &Model, u: &Unit, t: &Theme) -> View<Msg> {
    let bin = model.cfg.prefix.join("bin").join(&u.program);
    let mut info_hijos = vec![
        View::new(Style { size: Size { width: percent(1.0), height: length(20.0) }, ..Default::default() })
            .text_aligned(format!("✓ {}", u.label), 16.0, t.accent, Alignment::Start),
        View::new(Style { size: Size { width: percent(1.0), height: length(16.0) }, ..Default::default() })
            .text_aligned(
                format!("En el menú de apps · {}", bin.display()),
                11.0,
                t.fg_muted,
                Alignment::Start,
            ),
    ];
    // Instrucción exacta si la pieza la necesita (p.ej. la barra no se abre sola).
    if let Some(nota) = &u.post_install {
        info_hijos.push(
            View::new(Style { size: Size { width: percent(1.0), height: length(16.0) }, ..Default::default() })
                .text_aligned(format!("ⓘ {nota}"), 11.0, t.accent, Alignment::Start),
        );
    }
    let alto = if u.post_install.is_some() { 60.0 } else { 46.0 };
    let info = View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        ..Default::default()
    })
    .gap(2.0)
    .children(info_hijos);

    // Sólo lo "abrible" (apps reales) lleva botón Abrir. Las piezas complicadas
    // (barra, compositor, init) corren en contexto de sesión: sin Abrir.
    let mut hijos = vec![info];
    if u.launchable && !u.requires_root() {
        hijos.push(boton("Abrir", t.accent, t.bg_app, 90.0, Msg::Lanzar(u.program.clone())));
    }
    row(percent(1.0), length(alto)).gap(10.0).pad_x(6.0).children(hijos)
}

/// Instrucción según el modo de instalación (local vs sistema).
fn instruccion_modo(model: &Model) -> String {
    match model.mode {
        InstallMode::Local => format!(
            "Instalado en {}/bin. Si no aparecen en el menú, asegurate de que esa carpeta esté en tu PATH.",
            model.cfg.prefix.display()
        ),
        InstallMode::System => format!(
            "Instalado en {}/bin (en el PATH del sistema).",
            model.cfg.prefix.display()
        ),
    }
}

/// Botón compacto reutilizable.
fn boton(label: &str, bg: Color, fg: Color, w: f32, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(w), height: length(34.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(8.0)
    .text(label, 14.0, fg)
    .on_click(msg)
}

/// Centra horizontalmente un hijo en una fila de ancho completo.
fn wrap_center(child: View<Msg>) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0), height: llimphi_ui::llimphi_layout::taffy::prelude::auto() },
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![child])
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
        size: Size { width: percent(1.0), height: length(VIEWPORT - 64.0) },
        ..Default::default()
    })
    .clip(true)
    .children(vec![scroller]);

    col(percent(1.0), length(VIEWPORT))
        .pad(16.0)
        .gap(10.0)
        .children(vec![base_card(t), wrap])
}

/// La opción principal — destacada (no un subtítulo perdido): badge, borde de
/// acento, título y subtítulo.
fn base_card(t: &Theme) -> View<Msg> {
    let accent = t.accent;
    let badge = View::new(Style {
        size: Size { width: length(48.0), height: length(48.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(110, 140, 220, 46))
    .radius(12.0)
    .text("🖥", 26.0, accent);

    let textos = col(percent(1.0), llimphi_ui::llimphi_layout::taffy::prelude::auto())
        .gap(3.0)
        .grow()
        .children(vec![
            View::new(Style { size: Size { width: percent(1.0), height: length(22.0) }, ..Default::default() })
                .text_aligned("Instalar el escritorio completo", 17.0, t.fg_text, Alignment::Start)
                .text_weight(700.0),
            View::new(Style { size: Size { width: percent(1.0), height: length(18.0) }, ..Default::default() })
                .text_aligned(
                    "Sistema base mirada: compositor · display manager · barra · sesión",
                    12.0,
                    t.fg_muted,
                    Alignment::Start,
                ),
        ]);

    let chevron = View::new(Style { size: Size { width: length(34.0), height: length(34.0) }, align_items: Some(AlignItems::Center), justify_content: Some(JustifyContent::Center), ..Default::default() })
        .fill(accent)
        .radius(17.0)
        .text("›", 22.0, t.bg_app);

    row(percent(1.0), length(72.0))
        .gap(14.0)
        .pad(12.0)
        .fill(t.bg_panel)
        .radius(14.0)
        .border(1.5, accent)
        .children(vec![badge, textos, chevron])
        .on_click(Msg::AbrirSistemaBase)
}

/// Pantalla del **sistema base**: los componentes del escritorio mirada
/// (compositor, DM, barra…), todos activos por defecto, instalación real con
/// root en cualquier Linux.
fn sistema_base(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let needs_root = InstallConfig::detect(InstallMode::System).needs_root();

    let titulo = View::new(Style { size: Size { width: percent(1.0), height: length(30.0) }, ..Default::default() })
        .text_aligned("Sistema base — Escritorio mirada", 22.0, t.fg_text, Alignment::Start);
    let sub = View::new(Style { size: Size { width: percent(1.0), height: length(20.0) }, ..Default::default() })
        .text_aligned(
            "El display manager + compositor + sesión. Se instala en el sistema (/usr, /etc).",
            12.0,
            t.fg_muted,
            Alignment::Start,
        );
    let mut head = vec![titulo, sub];
    if model.base_done {
        head.push(linea(
            "Listo. Reiniciá la sesión y elegí «mirada» en el greeter (TTY: sudo mirada-dm). Si instalaste un init aparte, elegilo en GRUB.",
            t.accent,
            t,
        ));
    }

    // Filas de componentes (todos marcados por defecto).
    let mut filas: Vec<View<Msg>> = Vec::new();
    for (i, c) in model.base.iter().enumerate() {
        let sw = View::new(Style { size: Size { width: length(54.0), height: length(40.0) }, align_items: Some(AlignItems::Center), justify_content: Some(JustifyContent::Center), ..Default::default() })
            .children(vec![switch_view(
                if model.base_sel[i] { 1.0 } else { 0.0 },
                Msg::ToggleBaseComp(i),
                &SwitchPalette::from_theme(t),
            )]);
        let textos = col(percent(1.0), llimphi_ui::llimphi_layout::taffy::prelude::auto())
            .gap(2.0)
            .grow()
            .children(vec![
                View::new(Style { size: Size { width: percent(1.0), height: length(18.0) }, ..Default::default() })
                    .text_aligned(c.label, 14.0, t.fg_text, Alignment::Start),
                View::new(Style { size: Size { width: percent(1.0), height: length(16.0) }, ..Default::default() })
                    .text_aligned(c.description, 11.0, t.fg_muted, Alignment::Start),
            ]);
        let estado = {
            let (txt, color) = match &model.base_status[i] {
                UnitStatus::Working(_, _) => ("instalando…".to_string(), t.accent),
                UnitStatus::Done => ("✓".to_string(), t.accent),
                UnitStatus::Failed(_) => ("✗".to_string(), t.fg_destructive),
                UnitStatus::Idle => (String::new(), t.fg_muted),
            };
            View::new(Style { size: Size { width: length(90.0), height: length(40.0) }, align_items: Some(AlignItems::Center), justify_content: Some(JustifyContent::Center), ..Default::default() })
                .text(txt, 12.0, color)
        };
        filas.push(row(percent(1.0), length(44.0)).gap(8.0).pad_x(6.0).children(vec![sw, textos, estado]));
    }

    // Acciones.
    let n_sel = model.base_sel.iter().filter(|x| **x).count();
    let label_btn = if model.base_installing {
        "Instalando…"
    } else if needs_root {
        "Instalar (sudo)"
    } else {
        "Instalar"
    };
    let activo = n_sel > 0 && !model.base_installing && (!needs_root || !model.password.is_empty());
    let bg = if activo { t.accent } else { t.bg_button };
    let fg = if activo { t.bg_app } else { t.fg_muted };
    let mut instalar = View::new(Style { size: Size { width: length(170.0), height: length(36.0) }, align_items: Some(AlignItems::Center), justify_content: Some(JustifyContent::Center), ..Default::default() })
        .fill(bg).radius(8.0).text(label_btn, 15.0, fg);
    if activo {
        instalar = instalar.on_click(Msg::InstalarBase);
    }
    let acciones = row(percent(1.0), length(40.0))
        .gap(10.0)
        .children(vec![
            View::new(Style { flex_grow: 1.0, ..Default::default() }).text_aligned(
                format!("{n_sel} de {} componentes", model.base.len()),
                12.0,
                t.fg_muted,
                Alignment::Start,
            ),
            boton("Volver", t.bg_button, t.fg_text, 110.0, Msg::VolverCatalogo),
            instalar,
        ]);

    let cuerpo = col(percent(1.0), llimphi_ui::llimphi_layout::taffy::prelude::auto())
        .gap(4.0)
        .grow()
        .children(filas);

    let mut hijos = vec![
        col(percent(1.0), llimphi_ui::llimphi_layout::taffy::prelude::auto()).gap(8.0).children(head),
        cuerpo,
    ];
    // Campo de contraseña sudo in-app (sólo si hace falta root y no terminó).
    if needs_root && !model.base_done {
        hijos.push(campo_password(model, t));
    }
    if let Some(err) = &model.base_error {
        hijos.push(linea(err, t.fg_destructive, t));
    }
    hijos.push(acciones);

    col(percent(1.0), percent(1.0)).fill(t.bg_app).gap(10.0).pad(24.0).children(hijos)
}

/// El campo de contraseña de sudo, enmascarado, que se llena tecleando (lo
/// captura `on_key`). No abre un proceso externo ni cierra la app.
fn campo_password(model: &Model, t: &Theme) -> View<Msg> {
    let dots: String = "•".repeat(model.password.chars().count());
    let contenido = if dots.is_empty() {
        ("🔒  Escribí tu contraseña de sudo y presioná Enter".to_string(), t.fg_placeholder)
    } else {
        (format!("🔒  {dots}"), t.fg_text)
    };
    View::new(Style {
        size: Size { width: percent(1.0), height: length(40.0) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(t.bg_input)
    .radius(8.0)
    .border(1.0, t.border_focus)
    .pad_x(12.0)
    .text_aligned(contenido.0, 14.0, contenido.1, Alignment::Start)
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

    // Ícono en una pastilla tintada por cuadrante.
    let icono = View::new(Style {
        size: Size { width: length(38.0), height: length(38.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(icon_tint(&u.category))
    .radius(9.0)
    .text(u.icon.clone(), 20.0, icon_color(&u.category));
    let icono = View::new(Style {
        size: Size { width: length(46.0), height: length(ROW_H) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![icono]);

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

    // Pop-in: cada fila entra con un fade la primera vez que aparece su key
    // (estable por id de unidad).
    row(percent(1.0), length(ROW_H))
        .gap(8.0)
        .pad_x(6.0)
        .children(vec![sw_wrap, icono, medio, estado])
        .animated_enter(key_of(&u.id), motion::NORMAL)
}

fn estado_view(model: &Model, i: usize, instalada: bool, t: &Theme) -> View<Msg> {
    let (txt, color) = match &model.status[i] {
        UnitStatus::Working(step, r) => (paso_label(*step, *r), t.accent),
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
    // Si bajamos un manifiesto remoto firmado, comparamos contra él; si no,
    // contra el catálogo local.
    let (manifest, fuente) = match &model.remote_manifest {
        Some(m) => (m.clone(), "repo remoto"),
        None => (Manifest::new(churay_core::SUITE_VERSION, model.units.clone()), "catálogo local"),
    };
    let pend = pending_updates(&model.state, &manifest);
    let con_update: Vec<_> = pend.iter().filter(|u| u.kind != UpdateKind::Nueva).collect();

    // Encabezado: botón de chequeo remoto + estado.
    let label_btn = if model.buscando { "Buscando…" } else { "Buscar actualizaciones" };
    let tiene_repo = model.cfg.remote_base_url.is_some();
    let mut btn = View::new(Style {
        size: Size { width: length(220.0), height: length(32.0) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(if tiene_repo { t.accent } else { t.bg_button })
    .radius(8.0)
    .text(label_btn, 14.0, if tiene_repo { t.bg_app } else { t.fg_muted });
    if tiene_repo && !model.buscando {
        btn = btn.on_click(Msg::BuscarRemoto);
    }
    let estado_repo = View::new(Style { flex_grow: 1.0, ..Default::default() }).text_aligned(
        if model.repo_msg.is_empty() {
            if tiene_repo {
                format!("Repo: {}", model.cfg.remote_base_url.as_deref().unwrap_or(""))
            } else {
                "Sin repo remoto (definí CHURAY_REPO para actualizar online).".to_string()
            }
        } else {
            model.repo_msg.clone()
        },
        13.0,
        t.fg_muted,
        Alignment::Start,
    );
    let cabecera = row(percent(1.0), length(34.0)).gap(10.0).children(vec![estado_repo, btn]);

    let mut hijos: Vec<View<Msg>> = vec![cabecera];
    hijos.push(linea(&format!("Comparando contra: {fuente}"), t.fg_placeholder, t));

    if model.state.units.is_empty() {
        hijos.push(
            View::new(Style {
                flex_grow: 1.0,
                size: Size { width: percent(1.0_f32), height: length(220.0_f32) },
                ..Default::default()
            })
            .children(vec![empty_view(
                Icon::Archive,
                "Todavía no instalaste nada",
                Some("Elegí apps en el catálogo e instalalas; sus actualizaciones aparecen acá."),
                &EmptyPalette::from_theme(t),
            )]),
        );
    } else {
        for (id, inst) in model.state.units.iter() {
            let label = model
                .units
                .iter()
                .find(|u| &u.id == id)
                .map(|u| u.label.clone())
                .unwrap_or_else(|| id.clone());
            let nueva = con_update
                .iter()
                .find(|u| &u.id == id)
                .map(|u| u.available_version.clone());
            let (txt, color) = match nueva {
                Some(v) => (format!("{label} — {} → {}  ·  actualizar", inst.version, v), t.accent),
                None => (format!("{label} — {}  ·  al día", inst.version), t.fg_text),
            };
            hijos.push(linea(&txt, color, t));
        }
    }

    col(percent(1.0), length(VIEWPORT)).pad(20.0).gap(8.0).children(hijos)
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

    let mut hijos = Vec::new();

    // Banner de sugerencias: si lo elegido sugiere unidades sin marcar.
    let sugeridas = model.sugeridas_faltantes();
    if !sugeridas.is_empty() && !model.installing {
        let nombres: Vec<&str> = sugeridas.iter().map(|&i| model.units[i].label.as_str()).collect();
        let txt = View::new(Style { flex_grow: 1.0, ..Default::default() }).text_aligned(
            format!("Se complementan con: {}", nombres.join(", ")),
            13.0,
            t.fg_text,
            Alignment::Start,
        );
        let add = View::new(Style {
            size: Size { width: length(150.0), height: length(30.0) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(t.accent)
        .radius(8.0)
        .text("Agregar sugeridas", 12.0, t.bg_app)
        .on_click(Msg::AgregarSugeridas);
        hijos.push(
            row(percent(1.0), length(38.0))
                .gap(10.0)
                .pad(8.0)
                .fill(t.bg_input)
                .radius(8.0)
                .children(vec![txt, add]),
        );
    }

    // Aviso de fuente: si lo elegido se va a compilar (lento) o no hay con qué
    // (sistema sin bundle/repo/cargo). Anticipa la sorpresa del "compilando".
    let kinds: Vec<SourceKind> = model
        .visibles()
        .into_iter()
        .filter(|&i| model.selected[i])
        .map(|i| source_kind(&model.cfg, &model.units[i]))
        .collect();
    if kinds.iter().any(|k| *k == SourceKind::None) {
        hijos.push(linea(
            "⚠ Sin fuente para algo de lo elegido: hace falta un bundle o CHURAY_REPO (este sistema no puede compilar).",
            t.fg_destructive,
            t,
        ));
    } else if kinds.iter().any(|k| *k == SourceKind::Build) {
        hijos.push(linea(
            "⚙ Algo se compilará desde fuente (lento; modo dev, requiere cargo).",
            t.fg_muted,
            t,
        ));
    }

    hijos.push(row(percent(1.0), length(34.0)).gap(10.0).children(vec![resumen, acciones]));

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

/// Color del ícono (RGB) según el cuadrante — para que el catálogo no sea gris.
fn icon_rgb(category: &str) -> [u8; 3] {
    match category {
        "ruway" => [232, 156, 74],     // HACER — ámbar
        "yachay" => [120, 168, 240],   // CONOCER — azul
        "unanchay" => [118, 214, 150], // PERCIBIR — verde
        "ukupacha" => [186, 146, 236], // RAÍZ — violeta
        "sistema" => [228, 132, 132],  // sistema — rojo suave
        _ => [160, 180, 210],
    }
}
fn icon_color(category: &str) -> Color {
    let c = icon_rgb(category);
    Color::from_rgba8(c[0], c[1], c[2], 255)
}
fn icon_tint(category: &str) -> Color {
    let c = icon_rgb(category);
    Color::from_rgba8(c[0], c[1], c[2], 40)
}

fn paso_label(step: Step, r: f32) -> String {
    let pct = (r * 100.0) as u32;
    match step {
        Step::Resolviendo => "resolviendo…".into(),
        Step::Descargando => "bajando…".into(),
        Step::Compilando => format!("compilando {pct}%"),
        Step::Copiando => "copiando…".into(),
        Step::Desktop => "instalando…".into(),
        Step::Hecho => "✓".into(),
    }
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
    fn justify(self, j: JustifyContent) -> Self;
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
    fn justify(mut self, j: JustifyContent) -> Self {
        self.style.justify_content = Some(j);
        self
    }
}

/// Corre el modo headless del propio binario bajo `sudo -S`, pasándole la
/// contraseña por stdin. Lee `DONE <id>` por stdout y los redespacha como
/// componentes terminados; si falla, dispara `BaseError`. La app no se cierra.
fn elevated_install(cfg: &InstallConfig, ids: &str, pass: &str, h: &Handle<Msg>) {
    use std::io::{BufRead, BufReader, Read, Write};
    let Ok(exe) = std::env::current_exe() else {
        h.dispatch(Msg::BaseError("no encontré el ejecutable de churay".into()));
        return;
    };
    let mut cmd = std::process::Command::new("sudo");
    cmd.arg("-S").arg("-p").arg(""); // sin prompt (la clave va por stdin)
    cmd.arg(&exe).arg(format!("--install-base={ids}"));
    if let Some(b) = &cfg.bundle_dir {
        cmd.arg(format!("--bundle={}", b.display()));
    }
    if let Some(r) = &cfg.remote_base_url {
        cmd.arg(format!("--repo={r}"));
    }
    if let Some(w) = &cfg.workspace_root {
        cmd.arg(format!("--workspace={}", w.display()));
    }
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            h.dispatch(Msg::BaseError(format!("no se pudo lanzar sudo: {e}")));
            return;
        }
    };
    if let Some(mut sin) = child.stdin.take() {
        let _ = writeln!(sin, "{pass}");
    }
    let mut any = false;
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(id) = line.strip_prefix("DONE ") {
                any = true;
                h.dispatch(Msg::BaseCompDone(id.to_string()));
            }
        }
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    if !ok && !any {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        let low = err.to_lowercase();
        let msg = if low.contains("incorrect") || low.contains("contraseña") || low.contains("sorry") {
            "Contraseña incorrecta.".to_string()
        } else {
            format!("No se pudo instalar. {}", err.lines().last().unwrap_or("").trim())
        };
        h.dispatch(Msg::BaseError(msg));
    }
}

/// Modo headless: `churay --install-base=<ids> [--bundle=…] [--repo=…]
/// [--workspace=…]`. Corre como root (bajo sudo/pkexec), instala el sistema base
/// y emite `DONE <id>` por componente. No abre ventana.
fn run_install_base_headless(args: &[String]) -> i32 {
    use std::io::Write;
    let getarg = |k: &str| args.iter().find_map(|a| a.strip_prefix(k).map(|s| s.to_string()));
    let ids: Vec<String> = getarg("--install-base=")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    let mut cfg = InstallConfig::detect(InstallMode::System);
    if let Some(b) = getarg("--bundle=") {
        cfg.bundle_dir = Some(b.into());
    }
    if let Some(r) = getarg("--repo=") {
        cfg.remote_base_url = Some(r);
    }
    if let Some(w) = getarg("--workspace=") {
        cfg.workspace_root = Some(w.into());
    }
    let comps = churay_core::base_system();
    let refs: Vec<&churay_core::Component> = comps
        .iter()
        .filter(|c| ids.is_empty() || ids.iter().any(|id| id == c.id))
        .collect();
    // `$CHURAY_SYSROOT` (default `/`) — para probar sin tocar el sistema.
    let sysroot = std::env::var_os("CHURAY_SYSROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/"));
    match churay_core::install_base(&sysroot, &cfg, None, &refs, &mut |id, step, _| {
        if step == Step::Hecho {
            println!("DONE {id}");
            let _ = std::io::stdout().flush();
        }
    }) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ERR {e}");
            1
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a.starts_with("--install-base")) {
        std::process::exit(run_install_base_headless(&args));
    }
    llimphi_ui::run::<Churay>();
}
