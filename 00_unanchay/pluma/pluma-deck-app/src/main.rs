//! `pluma-deck` — app de **presentaciones espaciales** (modo Recorrido, tipo Prezi).
//!
//! Unifica en un solo binario lo que antes vivía repartido en demos: abrir un
//! documento, **presentarlo** (la cámara vuela por la ruta) y **autorearlo**
//! (mover/crear/borrar/rotar marcos), con guardar/cargar en el formato nativo.
//! Toda la lógica vive en `pluma-deck-core` (cámara, ruta, autoría, persistencia,
//! adaptador pluma); aquí sólo se cablean eventos y se elige la vista por modo.
//!
//! Uso:
//!   `cargo run -p pluma-deck-app -- [archivo.deck | archivo.md]`
//!   - `*.deck`  → carga el recorrido binario (postcard) y guarda sobre él.
//!   - `*.md`    → importa markdown como recorrido (guarda en `recorrido.deck`).
//!   - sin arg   → recorrido de bienvenida (guarda en `recorrido.deck`).
//!
//! Controles comunes: **flechas / Espacio / Enter** vuela por la ruta ·
//! **Home/Esc** vista general · **p** modo presentador (autoplay) · **rueda**
//! zoom-a-cursor · **Tab** alterna presentar/editar · **Ctrl+S / Ctrl+O**
//! guarda / carga · **Ctrl+Z / Ctrl+Shift+Z** deshace / rehace autoría. En
//! **editar**: arrastrar mueve/selecciona un marco (o panea el vacío), **n**
//! crea, **Supr** elimina, **[ ]** rota. En **presentar**: arrastrar panea libre.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, FlexDirection, Size, Style};
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_motion::{animate, motion, Tween};
use pluma_deck_core::adaptador::recorrido_desde_atomos;
use pluma_deck_core::{Autoplay, ContenidoMarco, Marco, Recorrido, RecorridoState, Rect, RejillaOpts};
use pluma_deck_recorrido_llimphi::{dentro, panel_actual, recorrido_view, recorrido_view_editor, ZOOM_BASE};

use app_bus::{AppMenu, Menu, MenuItem};

const PANEL_INICIAL: Rect = Rect { x: 0.0, y: 0.0, w: 1200.0, h: 760.0 };

#[derive(Clone, Copy, PartialEq)]
enum Modo {
    Presentar,
    Editar,
}

#[derive(Clone)]
enum Msg {
    Zoom { mult: f64, cursor: (f32, f32) },
    /// Pan libre (modo presentar): delta de pantalla.
    Pan { dx: f32, dy: f32 },
    /// Arrastre de autoría (modo editar): delta + posición del press.
    Arrastre { dx: f32, dy: f32, lx: f32, ly: f32 },
    FinArrastre,
    NuevoMarco,
    Eliminar,
    Rotar(f64),
    Deshacer,
    Rehacer,
    Guardar,
    Cargar,
    ToggleModo,
    VistaGeneral,
    ToggleAutoplay,
    Siguiente,
    Anterior,
    Tick,
    /// Cicla el tema del chrome (barra de menú / overlays).
    CambiarTema,
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cierra).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Navegación de teclado en el dropdown del menú principal (±1 fila).
    MenuNav(i32),
    /// Enter sobre la fila activa del menú principal.
    MenuActivate,
    /// Tick de re-render para la animación de aparición del dropdown.
    MenuTick,
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click en el lienzo → menú contextual anclado en `(x, y)` de
    /// ventana. En modo editar selecciona el marco bajo el cursor (si lo hay).
    ContextMenuOpen(f32, f32),
}

struct Model {
    rec: Recorrido,
    state: RecorridoState,
    autoplay: Autoplay,
    modo: Modo,
    seleccionado: Option<u64>,
    /// `None` = sin arrastre. `Some(None)` = paneando. `Some(Some(id))` = moviendo ese marco.
    arrastrando: Option<Option<u64>>,
    /// Destino de Ctrl+S (postcard).
    guardar_en: PathBuf,
    /// Undo/redo de autoría (snapshots del recorrido).
    historial: Historial<Recorrido>,
    /// Tema del chrome (barra de menú / overlays). El lienzo usa su paleta propia.
    theme: Theme,
    /// Barra de menú principal: índice del menú raíz abierto (`None` cerrado).
    menu_open: Option<usize>,
    /// Fila activa (resaltada por teclado) del dropdown del menú principal.
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal (0→1).
    menu_anim: Tween<f32>,
    /// Menú contextual del lienzo: `(x, y)` ancla en ventana. `None` cerrado.
    context_menu: Option<(f32, f32)>,
}

/// Recorrido de bienvenida cuando no se pasa archivo.
fn bienvenida() -> Recorrido {
    let slide = |t: &str, ps: &[&str]| ContenidoMarco::Texto {
        titulo: Some(t.into()),
        parrafos: ps.iter().map(|s| s.to_string()).collect(),
    };
    Recorrido::en_rejilla(
        vec![
            slide("pluma · deck", &["Presentaciones espaciales tipo Prezi.", "Pasá un .deck o un .md, o autoreá desde cero."]),
            slide("Presentar", &["Flechas / Espacio vuelan por la ruta.", "Home/Esc: vista general.   p: autoplay."]),
            slide("Editar (Tab)", &["Arrastrá para mover/seleccionar un marco.", "n: nuevo   Supr: borrar   [ ]: rotar."]),
            slide("Guardar", &["Ctrl+S guarda, Ctrl+O carga.", "Formato nativo postcard (.deck)."]),
        ],
        RejillaOpts { cols: 2, marco_w: 640.0, marco_h: 400.0, gap_x: 220.0, gap_y: 180.0 },
    )
}

/// Abre un archivo como Recorrido: `.md` se importa con el adaptador pluma;
/// cualquier otro se intenta como `.deck` binario. `None` si falla.
fn abrir(ruta: &str) -> Option<Recorrido> {
    let bytes = std::fs::read(ruta).ok()?;
    if ruta.ends_with(".md") {
        let md = String::from_utf8(bytes).ok()?;
        let doc = pluma_md::parse_md(&md, "es", "deck", 0);
        Some(recorrido_desde_atomos(&doc.atoms, RejillaOpts::default()))
    } else {
        Recorrido::deserializar(&bytes).ok()
    }
}

struct Deck;

impl App for Deck {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma · deck — presentaciones espaciales (Tab: presentar/editar)"
    }

    fn initial_size() -> (u32, u32) {
        (1200, 760)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        // Resuelve el recorrido inicial y el destino de guardado según el arg.
        let (rec, guardar_en) = match std::env::args().nth(1) {
            Some(p) if p.ends_with(".deck") => {
                let r = abrir(&p).unwrap_or_else(bienvenida);
                (r, PathBuf::from(p))
            }
            // .md u otro: importa si puede, pero guarda en un .deck para no
            // sobrescribir el fuente con binario.
            Some(p) => (abrir(&p).unwrap_or_else(bienvenida), PathBuf::from("recorrido.deck")),
            None => (bienvenida(), PathBuf::from("recorrido.deck")),
        };
        let mut state = RecorridoState::new();
        state.saltar_a_paso(&rec, 0, PANEL_INICIAL);
        handle.spawn_periodic(Duration::from_millis(16), || Msg::Tick);
        Model {
            rec,
            state,
            autoplay: Autoplay::default(),
            modo: Modo::Presentar,
            seleccionado: None,
            arrastrando: None,
            guardar_en,
            historial: Historial::new(64),
            theme: Theme::dark(),
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            context_menu: None,
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let panel = panel_actual().unwrap_or(PANEL_INICIAL);
        match msg {
            Msg::Zoom { mult, cursor } => {
                model.state.wheel(mult, (cursor.0 as f64, cursor.1 as f64), panel);
            }
            Msg::Pan { dx, dy } => model.state.arrastrar_delta(dx as f64, dy as f64),
            Msg::Arrastre { dx, dy, lx, ly } => {
                let modo = match model.arrastrando {
                    Some(m) => m,
                    None => {
                        let world = model.state.camara.screen_to_world((lx as f64, ly as f64), panel);
                        let m = model.rec.marco_en_punto(world);
                        model.arrastrando = Some(m);
                        if m.is_some() {
                            model.seleccionado = m;
                            // Una instantánea por arrastre (al agarrar el marco),
                            // no por cada move — undo revierte el movimiento entero.
                            model.historial.registrar(&model.rec);
                        }
                        m
                    }
                };
                match modo {
                    Some(id) => {
                        let (wdx, wdy) = model.state.camara.delta_pantalla_a_mundo(dx as f64, dy as f64);
                        model.rec.mover_marco(id, wdx, wdy);
                    }
                    None => model.state.arrastrar_delta(dx as f64, dy as f64),
                }
            }
            Msg::FinArrastre => model.arrastrando = None,
            Msg::NuevoMarco => {
                model.historial.registrar(&model.rec);
                let id = model.rec.marcos.iter().map(|m| m.id).max().unwrap_or(0) + 1;
                let (cx, cy) = model.state.camara.centro;
                let (w, h) = (520.0, 320.0);
                model.rec.agregar_marco(Marco::new(
                    id,
                    Rect::new(cx - w * 0.5, cy - h * 0.5, w, h),
                    ContenidoMarco::Texto { titulo: Some(format!("marco {id}")), parrafos: vec![] },
                ));
                model.rec.pasos.push(id);
                model.seleccionado = Some(id);
            }
            Msg::Eliminar => {
                if let Some(id) = model.seleccionado.take() {
                    model.historial.registrar(&model.rec);
                    model.rec.eliminar_marco(id);
                    let idx = model.state.paso.min(model.rec.n_pasos().saturating_sub(1));
                    model.state.saltar_a_paso(&model.rec, idx, panel);
                }
            }
            Msg::Rotar(d) => {
                if let Some(id) = model.seleccionado {
                    model.historial.registrar(&model.rec);
                    model.rec.rotar_marco(id, d);
                }
            }
            Msg::Deshacer | Msg::Rehacer => {
                let nuevo = match msg {
                    Msg::Deshacer => model.historial.deshacer(&model.rec),
                    _ => model.historial.rehacer(&model.rec),
                };
                if let Some(rec) = nuevo {
                    model.rec = rec;
                    // La selección puede haber dejado de existir; el paso se clampa.
                    if model.seleccionado.map_or(false, |id| model.rec.marco(id).is_none()) {
                        model.seleccionado = None;
                    }
                    let idx = model.state.paso.min(model.rec.n_pasos().saturating_sub(1));
                    model.state.saltar_a_paso(&model.rec, idx, panel);
                }
            }
            Msg::Guardar => match model.rec.serializar() {
                Ok(bytes) => {
                    let _ = std::fs::write(&model.guardar_en, &bytes);
                    eprintln!("guardado {} ({} bytes)", model.guardar_en.display(), bytes.len());
                }
                Err(e) => eprintln!("error al guardar: {e}"),
            },
            Msg::Cargar => {
                let r = std::fs::read(&model.guardar_en)
                    .map_err(|_| "no se pudo leer")
                    .and_then(|b| Recorrido::deserializar(&b));
                match r {
                    Ok(rec) => {
                        model.rec = rec;
                        model.seleccionado = None;
                        model.state.saltar_a_paso(&model.rec, 0, panel);
                        eprintln!("cargado {}", model.guardar_en.display());
                    }
                    Err(e) => eprintln!("error al cargar: {e}"),
                }
            }
            Msg::ToggleModo => {
                model.modo = match model.modo {
                    Modo::Presentar => Modo::Editar,
                    Modo::Editar => Modo::Presentar,
                };
                model.autoplay.pausa();
                eprintln!("modo: {}", if model.modo == Modo::Editar { "editar" } else { "presentar" });
            }
            Msg::VistaGeneral => {
                model.state.vista_general(&model.rec, panel);
            }
            Msg::ToggleAutoplay => {
                model.autoplay.toggle();
            }
            Msg::Siguiente => {
                model.state.siguiente(&model.rec, panel);
            }
            Msg::Anterior => {
                model.state.anterior(&model.rec, panel);
            }
            Msg::Tick => {
                model.state.avanzar(1.0 / 60.0);
                model.autoplay.tick(1.0 / 60.0, &mut model.state, &model.rec, panel);
            }
            Msg::CambiarTema => {
                model.theme = Theme::next_after(model.theme.name);
            }
            Msg::MenuOpen(which) => {
                model.menu_open = which;
                model.menu_active = usize::MAX;
                // Abrir un menú raíz cierra cualquier contextual.
                model.context_menu = None;
                // Animación de aparición/swap: cada vez que se abre (o se
                // cambia de) menú, el dropdown se funde+desliza de nuevo.
                if which.is_some() {
                    model.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    model.menu_active = menubar_nav(&menu, mi, model.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    if let Some(cmd) = menubar_command_at(&menu, mi, model.menu_active) {
                        return Deck::update(model, Msg::MenuCommand(cmd), handle);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::CloseMenus => {
                model.menu_open = None;
                model.menu_active = usize::MAX;
                model.context_menu = None;
            }
            Msg::MenuCommand(cmd) => {
                model.menu_open = None;
                model.menu_active = usize::MAX;
                if let Some(next) = comando_a_msg(&cmd) {
                    return Deck::update(model, next, handle);
                }
                // Comandos sin Msg directo (salir / no-op).
                if cmd == "file.quit" {
                    std::process::exit(0);
                }
            }
            Msg::ContextMenuOpen(x, y) => {
                model.menu_open = None;
                // En editar, el right-click también selecciona el marco bajo el
                // cursor (si lo hay) para que las acciones del menú apliquen a él.
                if model.modo == Modo::Editar {
                    let world = model.state.camara.screen_to_world((x as f64, y as f64), panel);
                    if let Some(id) = model.rec.marco_en_punto(world) {
                        model.seleccionado = Some(id);
                    }
                }
                model.context_menu = Some((x, y));
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model));

        let lienzo = match model.modo {
            Modo::Editar => recorrido_view_editor(&model.rec, &model.state, model.seleccionado)
                .draggable_at(|phase, dx, dy, lx, ly| match phase {
                    DragPhase::Move => Some(Msg::Arrastre { dx, dy, lx, ly }),
                    DragPhase::End => Some(Msg::FinArrastre),
                }),
            Modo::Presentar => recorrido_view(&model.rec, &model.state).draggable(|phase, dx, dy| match phase {
                DragPhase::Move => Some(Msg::Pan { dx, dy }),
                DragPhase::End => None,
            }),
        };

        // Column raíz: barra de menú arriba + lienzo a pantalla completa debajo.
        // El right-click va en la RAÍZ (origen 0,0 ⇒ local == ventana) para anclar
        // el menú contextual en coords de ventana, igual que el panel del lienzo.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(model.theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, lienzo])
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // Menú contextual del lienzo tiene prioridad sobre el dropdown principal.
        if let Some((x, y)) = model.context_menu {
            let viewport = viewport_of(model);
            // Acciones según el modo. En editar, sobre el marco seleccionado;
            // en presentar, navegación. Sólo comandos que mapean a Msg reales.
            let (header, items, on_pick): (String, Vec<ContextMenuItem>, Arc<dyn Fn(usize) -> Msg + Send + Sync>) =
                if model.modo == Modo::Editar {
                    let hay_sel = model.seleccionado.is_some();
                    // Las acciones sobre marco se deshabilitan (gris) sin selección.
                    let opt = |it: ContextMenuItem| if hay_sel { it } else { it.disabled() };
                    let rotar_l = opt(ContextMenuItem::action("Rotar ⟲"));
                    let rotar_r = opt(ContextMenuItem::action("Rotar ⟳"));
                    let borrar = opt(ContextMenuItem::action("Eliminar marco").destructive());
                    let items = vec![
                        ContextMenuItem::action("Nuevo marco"),
                        rotar_l,
                        rotar_r,
                        ContextMenuItem::separator(),
                        borrar,
                    ];
                    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(|i: usize| match i {
                        0 => Msg::NuevoMarco,
                        1 => Msg::Rotar(-0.08),
                        2 => Msg::Rotar(0.08),
                        4 => Msg::Eliminar,
                        _ => Msg::CloseMenus,
                    });
                    ("Editar marco".to_string(), items, on_pick)
                } else {
                    let items = vec![
                        ContextMenuItem::action("Siguiente"),
                        ContextMenuItem::action("Anterior"),
                        ContextMenuItem::action("Vista general"),
                        ContextMenuItem::separator(),
                        ContextMenuItem::action("Presentar (autoplay)"),
                    ];
                    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(|i: usize| match i {
                        0 => Msg::Siguiente,
                        1 => Msg::Anterior,
                        2 => Msg::VistaGeneral,
                        4 => Msg::ToggleAutoplay,
                        _ => Msg::CloseMenus,
                    });
                    ("Presentar".to_string(), items, on_pick)
                };
            return Some(context_menu_view(ContextMenuSpec {
                anchor: (x, y),
                viewport,
                header: Some(header),
                items,
                active: usize::MAX,
                on_pick,
                on_dismiss: Msg::CloseMenus,
                palette: ContextMenuPalette::from_theme(&model.theme),
            }));
        }
        // Si no, el dropdown del menú principal (con nav por teclado + animación).
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model),
            model.menu_active,
            model.menu_anim.value(),
        )
    }

    fn on_wheel(_m: &Self::Model, delta: WheelDelta, cursor: (f32, f32), _mods: Modifiers) -> Option<Self::Msg> {
        let panel = panel_actual()?;
        if !dentro(panel, cursor.0, cursor.1) {
            return None;
        }
        Some(Msg::Zoom { mult: ZOOM_BASE.powf(-delta.y as f64), cursor })
    }

    fn on_key(model: &Self::Model, ev: &KeyEvent) -> Option<Self::Msg> {
        if ev.state != KeyState::Pressed {
            return None;
        }
        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
        // cierra. Tiene prioridad sobre todo lo demás.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
            return match &ev.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        // Guardar/cargar en cualquier modo.
        if ev.modifiers.ctrl {
            return match &ev.key {
                Key::Character(c) if c.eq_ignore_ascii_case("s") => Some(Msg::Guardar),
                Key::Character(c) if c.eq_ignore_ascii_case("o") => Some(Msg::Cargar),
                // Ctrl+Z deshace; Ctrl+Shift+Z o Ctrl+Y rehacen.
                Key::Character(c) if c.eq_ignore_ascii_case("z") => {
                    Some(if ev.modifiers.shift { Msg::Rehacer } else { Msg::Deshacer })
                }
                Key::Character(c) if c.eq_ignore_ascii_case("y") => Some(Msg::Rehacer),
                _ => None,
            };
        }
        // Comunes a ambos modos.
        match &ev.key {
            Key::Named(NamedKey::Tab) => return Some(Msg::ToggleModo),
            Key::Named(NamedKey::Home | NamedKey::Escape) => return Some(Msg::VistaGeneral),
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown | NamedKey::Enter | NamedKey::Space) => {
                return Some(Msg::Siguiente)
            }
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) => return Some(Msg::Anterior),
            Key::Character(c) if c.eq_ignore_ascii_case("p") => return Some(Msg::ToggleAutoplay),
            _ => {}
        }
        // Sólo en modo editar.
        if model.modo == Modo::Editar {
            return match &ev.key {
                Key::Character(c) if c.as_str() == "n" => Some(Msg::NuevoMarco),
                Key::Character(c) if c.as_str() == "[" => Some(Msg::Rotar(-0.08)),
                Key::Character(c) if c.as_str() == "]" => Some(Msg::Rotar(0.08)),
                Key::Named(NamedKey::Delete | NamedKey::Backspace) => Some(Msg::Eliminar),
                _ => None,
            };
        }
        None
    }
}

/// Pila de undo/redo genérica para autoría. Antes de cada cambio se `registrar`a
/// el estado previo; `deshacer`/`rehacer` mueven el estado entre pasado y futuro.
/// Acotada a `max` entradas (descarta las más viejas).
struct Historial<T> {
    pasado: Vec<T>,
    futuro: Vec<T>,
    max: usize,
}

impl<T: Clone> Historial<T> {
    fn new(max: usize) -> Self {
        Self { pasado: Vec::new(), futuro: Vec::new(), max: max.max(1) }
    }

    /// Registra `actual` (estado **previo** al cambio que está por aplicarse) y
    /// limpia el redo — una rama nueva invalida los rehacer pendientes.
    fn registrar(&mut self, actual: &T) {
        self.pasado.push(actual.clone());
        if self.pasado.len() > self.max {
            self.pasado.remove(0);
        }
        self.futuro.clear();
    }

    /// Deshace: devuelve el último estado registrado y manda `actual` al futuro.
    fn deshacer(&mut self, actual: &T) -> Option<T> {
        let prev = self.pasado.pop()?;
        self.futuro.push(actual.clone());
        Some(prev)
    }

    /// Rehace: devuelve el último estado deshecho y manda `actual` al pasado.
    fn rehacer(&mut self, actual: &T) -> Option<T> {
        let next = self.futuro.pop()?;
        self.pasado.push(actual.clone());
        Some(next)
    }
}

/// Viewport para clampear overlays. El deck no trackea el resize, así que
/// usamos el tamaño inicial — basta para anclar los menús dentro de pantalla.
fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = Deck::initial_size();
    (w as f32, h as f32)
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(menu: &'a AppMenu, model: &'a Model) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme: &model.theme,
        viewport: viewport_of(model),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// Menú principal del deck. Sólo comandos que mapean a acciones reales del
/// `update`. "Editar" (Slide) aparece siempre porque la autoría no necesita
/// foco de texto; sus ítems se deshabilitan según el modo / selección reales.
fn app_menu(model: &Model) -> AppMenu {
    let editar = model.modo == Modo::Editar;
    let hay_sel = model.seleccionado.is_some();
    // Helper: ítem habilitado/deshabilitado según condición real.
    let item = |label: &str, cmd: &str, on: bool| {
        let it = MenuItem::new(label, cmd);
        if on { it } else { it.disabled() }
    };
    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Guardar", "file.save").shortcut("Ctrl+S"))
                .item(MenuItem::new("Cargar", "file.open").shortcut("Ctrl+O")),
        )
        .menu(
            Menu::new("Editar")
                .item(MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z"))
                .item(MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Shift+Z").separated())
                .item(item("Rotar ⟲", "edit.rotar_l", editar && hay_sel))
                .item(item("Rotar ⟳", "edit.rotar_r", editar && hay_sel)),
        )
        .menu(
            Menu::new("Slide")
                .item(item("Nuevo marco", "slide.nuevo", editar).shortcut("n"))
                .item(item("Eliminar marco", "slide.eliminar", editar && hay_sel).shortcut("Supr"))
                .item(MenuItem::new("Siguiente", "slide.siguiente").shortcut("→").separated())
                .item(MenuItem::new("Anterior", "slide.anterior").shortcut("←")),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new(
                    if editar { "Presentar (Tab)" } else { "Editar (Tab)" },
                    "view.modo",
                ))
                .item(MenuItem::new("Vista general", "view.general").shortcut("Home"))
                .item(MenuItem::new("Autoplay", "view.autoplay").shortcut("p"))
                .item(MenuItem::new("Cambiar tema", "view.theme").separated()),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Traduce un command id del menú principal al `Msg` real del deck. `None` para
/// los que no tienen Msg directo (`file.quit` se maneja aparte; `help.about` no-op).
fn comando_a_msg(cmd: &str) -> Option<Msg> {
    Some(match cmd {
        "file.save" => Msg::Guardar,
        "file.open" => Msg::Cargar,
        "edit.undo" => Msg::Deshacer,
        "edit.redo" => Msg::Rehacer,
        "edit.rotar_l" => Msg::Rotar(-0.08),
        "edit.rotar_r" => Msg::Rotar(0.08),
        "slide.nuevo" => Msg::NuevoMarco,
        "slide.eliminar" => Msg::Eliminar,
        "slide.siguiente" => Msg::Siguiente,
        "slide.anterior" => Msg::Anterior,
        "view.modo" => Msg::ToggleModo,
        "view.general" => Msg::VistaGeneral,
        "view.autoplay" => Msg::ToggleAutoplay,
        "view.theme" => Msg::CambiarTema,
        _ => return None,
    })
}

fn main() {
    llimphi_ui::run::<Deck>();
}

#[cfg(test)]
mod pruebas {
    use super::Historial;

    #[test]
    fn deshacer_rehacer_round_trip() {
        let mut h = Historial::new(10);
        // estado: 0 → (registrar 0) → 1 → (registrar 1) → 2
        h.registrar(&0);
        h.registrar(&1);
        let mut actual = 2;
        // deshacer dos veces: 2→1→0
        actual = h.deshacer(&actual).unwrap();
        assert_eq!(actual, 1);
        actual = h.deshacer(&actual).unwrap();
        assert_eq!(actual, 0);
        assert!(h.deshacer(&actual).is_none(), "sin más pasado");
        // rehacer dos veces: 0→1→2
        actual = h.rehacer(&actual).unwrap();
        assert_eq!(actual, 1);
        actual = h.rehacer(&actual).unwrap();
        assert_eq!(actual, 2);
        assert!(h.rehacer(&actual).is_none(), "sin más futuro");
    }

    #[test]
    fn registrar_tras_deshacer_limpia_el_futuro() {
        let mut h = Historial::new(10);
        h.registrar(&0);
        let actual = h.deshacer(&1).unwrap(); // actual ahora = 0, futuro = [1]
        assert_eq!(actual, 0);
        h.registrar(&actual); // rama nueva: el futuro se descarta
        assert!(h.rehacer(&actual).is_none());
    }

    #[test]
    fn respeta_el_tope_descartando_lo_mas_viejo() {
        let mut h = Historial::new(2);
        h.registrar(&1);
        h.registrar(&2);
        h.registrar(&3); // descarta el 1
        assert_eq!(h.deshacer(&99), Some(3));
        assert_eq!(h.deshacer(&3), Some(2));
        assert!(h.deshacer(&2).is_none());
    }
}
