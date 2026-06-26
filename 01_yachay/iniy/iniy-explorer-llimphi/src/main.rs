// `iniy-explorer-llimphi` — visualiza el corpus de iniy en Llimphi.
//
// Lee la DB SQLite de iniy y muestra:
// - Header con conteos del corpus.
// - Lista de fuentes con su reputación (score derivado del grafo NLI).
// - Lista de aserciones, cada una coloreada por su opinión dominante
//   (verde=creencia, rojo=descreencia, gris=incertidumbre) y atribuida
//   a su fuente efectiva.
//
// MVP feo: lectura única al arrancar, sin polling. Re-lanzar el binario
// para ver actualizaciones tras correr `iniy nli` o `iniy extract` de nuevo.
//
// Path de la DB: env `INIY_DB` o `./iniy.db`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_raster::vello::kurbo::{Affine, BezPath, Circle as KurboCircle, Stroke};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, PaintRect, View};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_card::{card_view, CardOptions, CardPalette};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_empty::{empty_view, EmptyPalette};
use llimphi_widget_toast::{toast_stack_view, Toast};
use llimphi_icons::Icon;
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_motion::{animate, motion, Tween};
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey};

use app_bus::{AppMenu, Menu, MenuItem};
use std::sync::Arc;

use iniy_core::{Asercion, AsercionId, FuenteId, Implicacion, Opinion};
use iniy_graph::GrafoCreencias;
use iniy_store::{AsercionAtribuida, FuenteResumen, Store};

const MAX_ASERCIONES_VISIBLES: usize = 60;
/// Cuánto vive un toast antes de auto-descartarse.
const TOAST_TTL: Duration = Duration::from_secs(4);
const ACCENT_CREENCIA: Color = Color::from_rgba8(0xa3, 0xbe, 0x8c, 0xff);     // verde
const ACCENT_DESCREENCIA: Color = Color::from_rgba8(0xbf, 0x61, 0x6a, 0xff);  // rojo
const ACCENT_INCERTIDUMBRE: Color = Color::from_rgba8(0x88, 0x88, 0x99, 0xff); // gris
const ACCENT_CITADA: Color = Color::from_rgba8(0xeb, 0xcb, 0x8b, 0xff);       // ámbar

#[derive(Clone)]
enum Msg {
    /// Toggle: si el id ya estaba seleccionado, deselecciona.
    Seleccionar(AsercionId),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Navegación por teclado dentro del dropdown del menú principal
    /// (`+1` baja, `-1` sube).
    MenuNav(i32),
    /// Ejecuta la fila activa del menú principal (Enter).
    MenuActivate,
    /// Tick de animación del dropdown (sólo re-render).
    MenuTick,
    /// Cicla el tema claro/oscuro.
    CambiarTema,
    /// Recarga el corpus desde la DB (re-lee SQLite y re-calcula layout).
    Recargar,
    /// Right-click en la raíz → abre el menú contextual anclado en
    /// `(x, y)` de ventana sobre la aserción seleccionada. Sin selección
    /// es no-op.
    MenuContextual(f32, f32),
    /// Un toast cumplió su `duration`: se descarta del stack.
    ToastExpire(u64),
}

struct Model {
    db_path: PathBuf,
    error: Option<String>,
    aserciones: Vec<AsercionAtribuida>,
    fuentes: Vec<FuenteResumen>,
    reputaciones: std::collections::HashMap<FuenteId, f32>,
    /// Pre-computado en init con Fruchterman-Reingold. Coordenadas en [0,1].
    /// Compartido por `Arc` con el painter para no clonar en cada frame.
    posiciones: std::sync::Arc<std::collections::HashMap<AsercionId, (f32, f32)>>,
    /// Pre-computado: (premisa, hipotesis, entailment, contradiction).
    /// Solo relaciones no triviales (al menos una > 0).
    aristas_grafo: std::sync::Arc<Vec<(AsercionId, AsercionId, f32, f32)>>,
    n_implicaciones: usize,
    seleccionada: Option<AsercionId>,
    theme: Theme,
    /// Barra de menú principal: índice del menú raíz abierto (`None` cerrado).
    menu_open: Option<usize>,
    /// Fila activa (resaltada por teclado) dentro del dropdown abierto.
    /// `usize::MAX` = ninguna.
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal.
    menu_anim: Tween<f32>,
    /// Menú contextual sobre la aserción seleccionada: `(x, y)` ancla en
    /// ventana. `None` cerrado.
    menu_contextual: Option<(f32, f32)>,
    /// Toasts vivos (confirmaciones/errores de recarga del corpus).
    toasts: Vec<Toast>,
    /// Id incremental para correlacionar toast ↔ Msg de expiración.
    next_toast: u64,
}

struct Explorer;

impl App for Explorer {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "iniy explorer"
    }

    fn initial_size() -> (u32, u32) {
        (1000, 700)
    }

    fn init(_handle: &Handle<Msg>) -> Model {
        let db_path = std::env::var("INIY_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("iniy.db"));

        let theme = Theme::dark();

        match cargar_modelo(&db_path) {
            Ok((aserciones, fuentes, reputaciones, n_implicaciones, imps)) => {
                let aristas_grafo = std::sync::Arc::new(
                    imps.iter()
                        .filter(|i| i.relacion.entailment > 0.0 || i.relacion.contradiction > 0.0)
                        .map(|i| (i.premisa, i.hipotesis, i.relacion.entailment, i.relacion.contradiction))
                        .collect::<Vec<_>>(),
                );
                let posiciones = std::sync::Arc::new(layout_fruchterman_reingold(
                    &aserciones,
                    &aristas_grafo,
                ));
                Model {
                    db_path,
                    error: None,
                    aserciones,
                    fuentes,
                    reputaciones,
                    posiciones,
                    aristas_grafo,
                    n_implicaciones,
                    seleccionada: None,
                    theme,
                    menu_open: None,
                    menu_active: usize::MAX,
                    menu_anim: Tween::idle(1.0),
                    menu_contextual: None,
                    toasts: Vec::new(),
                    next_toast: 0,
                }
            }
            Err(e) => Model {
                db_path,
                error: Some(e.to_string()),
                aserciones: Vec::new(),
                fuentes: Vec::new(),
                reputaciones: std::collections::HashMap::new(),
                posiciones: std::sync::Arc::new(std::collections::HashMap::new()),
                aristas_grafo: std::sync::Arc::new(Vec::new()),
                n_implicaciones: 0,
                seleccionada: None,
                theme,
                menu_open: None,
                menu_active: usize::MAX,
                menu_anim: Tween::idle(1.0),
                menu_contextual: None,
                toasts: Vec::new(),
                next_toast: 0,
            },
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Seleccionar(id) => {
                model.seleccionada = if model.seleccionada == Some(id) { None } else { Some(id) };
                // Seleccionar/deseleccionar cierra cualquier menú contextual.
                model.menu_contextual = None;
            }
            Msg::MenuOpen(which) => {
                model.menu_open = which;
                model.menu_active = usize::MAX;
                // Abrir un menú raíz cierra cualquier contextual.
                model.menu_contextual = None;
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
                        model.menu_open = None;
                        model.menu_active = usize::MAX;
                        return handle_menu_command(model, &cmd, handle);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::CloseMenus => {
                model.menu_open = None;
                model.menu_active = usize::MAX;
                model.menu_contextual = None;
            }
            Msg::MenuCommand(cmd) => {
                model.menu_open = None;
                model.menu_active = usize::MAX;
                return handle_menu_command(model, &cmd, handle);
            }
            Msg::CambiarTema => {
                model.theme = Theme::next_after(model.theme.name);
            }
            Msg::Recargar => {
                model = recargar_corpus(model);
                // Confirmá la acción real del usuario con un toast efímero.
                let id = model.next_toast;
                model.next_toast += 1;
                let toast = match model.error.clone() {
                    Some(e) => Toast::error(id, format!("No se pudo recargar: {e}"), TOAST_TTL),
                    None => Toast::success(
                        id,
                        format!("Corpus recargado · {} aserciones", model.aserciones.len()),
                        TOAST_TTL,
                    ),
                };
                push_toast(&mut model, handle, toast);
            }
            Msg::ToastExpire(id) => {
                model.toasts.retain(|t| t.id != id);
            }
            Msg::MenuContextual(x, y) => {
                // Sólo si hay una aserción seleccionada.
                if model.seleccionada.is_some() {
                    model.menu_open = None;
                    model.menu_contextual = Some((x, y));
                }
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
        let header_text = format!(
            "iniy · {}  ·  {} fuentes  ·  {} aserciones  ·  {} relaciones",
            model.db_path.display(),
            model.fuentes.len(),
            model.aserciones.len(),
            model.n_implicaciones,
        );
        let header =
            app_header::<Msg>(header_text, Vec::new(), &AppHeaderPalette::from_theme(&theme));

        let mut chrome: Vec<View<Msg>> = vec![menubar, header];

        if let Some(err) = &model.error {
            chrome.push(banner_view::<Msg>(BannerKind::Error, err.clone()));
            return con_toasts(model, rama_columna(theme, chrome));
        }
        if model.aserciones.is_empty() {
            // Empty-state con orientación en vez de un cartel chato: ícono
            // apagado + qué correr para poblar el corpus.
            let cuerpo_vacio = View::new(Style {
                flex_grow: 1.0,
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                min_size: Size { width: length(0.0_f32), height: length(0.0_f32) },
                ..Default::default()
            })
            .children(vec![empty_view::<Msg>(
                Icon::Archive,
                "corpus vacío",
                Some("Corré `iniy ingest <ruta>` y luego `iniy extract <doc-id>` para poblarlo."),
                &EmptyPalette::from_theme(&theme),
            )]);
            chrome.push(cuerpo_vacio);
            return con_toasts(model, rama_columna(theme, chrome));
        }

        let palette = CardPalette::from_theme(&theme);

        // Bloque "fuentes" — primera mitad horizontal del cuerpo.
        let fuentes_titulo = etiqueta_seccion("fuentes", theme.fg_muted);
        let mut fuentes_cards: Vec<View<Msg>> = vec![fuentes_titulo];
        for f in &model.fuentes {
            fuentes_cards.push(fuente_card(f, model.reputaciones.get(&f.fuente.id).copied(), &theme, &palette));
        }
        let panel_fuentes = panel_columna(theme, fuentes_cards);

        // Bloque "aserciones" — segunda mitad horizontal.
        let asercs_titulo = etiqueta_seccion("aserciones", theme.fg_muted);
        let mut aserc_cards: Vec<View<Msg>> = vec![asercs_titulo];
        for att in model.aserciones.iter().take(MAX_ASERCIONES_VISIBLES) {
            let sel = model.seleccionada == Some(att.asercion.id);
            aserc_cards.push(asercion_card(att, sel, &theme, &palette));
        }
        if model.aserciones.len() > MAX_ASERCIONES_VISIBLES {
            aserc_cards.push(
                texto_simple(
                    format!("… +{} más", model.aserciones.len() - MAX_ASERCIONES_VISIBLES),
                    11.0,
                    theme.fg_muted,
                ),
            );
        }
        let panel_asercs = panel_columna(theme, aserc_cards);

        // Panel central: grafo dibujado vía paint_with.
        let panel_grafo = grafo_panel(model);

        let body = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(8.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![panel_fuentes, panel_grafo, panel_asercs]);

        chrome.push(body);
        con_toasts(model, rama_columna(theme, chrome))
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // El menú contextual de la aserción tiene prioridad si está abierto.
        if let Some((x, y)) = model.menu_contextual {
            // Sólo se muestra si hay selección viva (la apertura lo garantiza,
            // pero el corpus pudo recargarse — revalidamos).
            let sel = model.seleccionada?;
            let att = model
                .aserciones
                .iter()
                .find(|a| a.asercion.id == sel)?;
            let header = truncar(&att.asercion.texto, 48);
            let viewport = viewport_of(model);
            // Acciones reales del explorer de sólo lectura: deseleccionar la
            // aserción y recargar el corpus. No inventamos edición.
            let items = vec![
                ContextMenuItem::action("Deseleccionar"),
                ContextMenuItem::action("Recargar corpus"),
            ];
            let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| match i {
                0 => Msg::Seleccionar(sel), // toggle ⇒ deselecciona la activa
                _ => Msg::Recargar,
            });
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
        // Si no, el dropdown del menú principal.
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &model.theme),
            model.menu_active,
            model.menu_anim.value(),
        )
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
        // cierra. Consume la tecla.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
            return Some(match &event.key {
                Key::Named(NamedKey::Escape) => Msg::CloseMenus,
                Key::Named(NamedKey::ArrowLeft) => Msg::MenuOpen(Some((mi + n - 1) % n)),
                Key::Named(NamedKey::ArrowRight) => Msg::MenuOpen(Some((mi + 1) % n)),
                Key::Named(NamedKey::ArrowDown) => Msg::MenuNav(1),
                Key::Named(NamedKey::ArrowUp) => Msg::MenuNav(-1),
                Key::Named(NamedKey::Enter) => Msg::MenuActivate,
                _ => return None,
            });
        }
        None
    }
}

/// Viewport para clampear overlays: el explorer no trackea el tamaño de
/// ventana, así que usamos `initial_size()`.
fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = Explorer::initial_size();
    (w as f32, h as f32)
}

/// Empuja un toast al stack y programa su expiración en un worker.
fn push_toast(model: &mut Model, handle: &Handle<Msg>, toast: Toast) {
    let id = toast.id;
    model.toasts.push(toast);
    handle.spawn(move || {
        std::thread::sleep(TOAST_TTL);
        Msg::ToastExpire(id)
    });
}

/// Superpone el stack de toasts vivos sobre `root` (esquina inferior
/// derecha). Sin toasts vivos devuelve `root` tal cual.
fn con_toasts(model: &Model, root: View<Msg>) -> View<Msg> {
    let now = Instant::now();
    let alive: Vec<Toast> = model.toasts.iter().filter(|t| t.is_alive(now)).cloned().collect();
    if alive.is_empty() {
        return root;
    }
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![
        root,
        toast_stack_view(&alive, viewport_of(model), Msg::ToastExpire),
    ])
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(menu: &'a AppMenu, model: &Model, theme: &'a Theme) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: viewport_of(model),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// El menú principal del explorer. Archivo / Ver / Ayuda — sólo comandos
/// que mapean a acciones reales (recargar, deseleccionar, tema). Sin
/// "Editar": el explorer no tiene campos de texto editables.
fn app_menu(model: &Model) -> AppMenu {
    let ver = Menu::new("Ver")
        .item(MenuItem::new("Recargar corpus", "file.recargar").shortcut("Ctrl+R"))
        .item(MenuItem::new("Cambiar tema", "view.tema").separated());
    // "Deseleccionar" sólo tiene sentido con una aserción activa.
    let deseleccionar = if model.seleccionada.is_some() {
        MenuItem::new("Deseleccionar", "view.deseleccionar")
    } else {
        MenuItem::new("Deseleccionar", "view.deseleccionar").disabled()
    };
    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Recargar corpus", "file.recargar").shortcut("Ctrl+R"))
                .item(MenuItem::new("Salir", "file.salir").shortcut("Ctrl+Q").separated()),
        )
        .menu(ver.item(deseleccionar))
        .menu(Menu::new("Ayuda").item(MenuItem::new("Acerca de", "help.about")))
}

/// Traduce un command id del menú principal al `Msg`/efecto real.
fn handle_menu_command(mut model: Model, cmd: &str, handle: &Handle<Msg>) -> Model {
    match cmd {
        "file.recargar" => {
            handle.dispatch(Msg::Recargar);
            model
        }
        "file.salir" => std::process::exit(0),
        "view.tema" => {
            handle.dispatch(Msg::CambiarTema);
            model
        }
        "view.deseleccionar" => {
            model.seleccionada = None;
            model.menu_contextual = None;
            model
        }
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => model,
    }
}

/// Re-lee la DB SQLite y re-computa layout/aristas en sitio. Tras una
/// recarga la selección puede quedar colgada (la aserción desapareció);
/// si ya no existe, la descartamos.
fn recargar_corpus(mut model: Model) -> Model {
    match cargar_modelo(&model.db_path) {
        Ok((aserciones, fuentes, reputaciones, n_implicaciones, imps)) => {
            let aristas_grafo = std::sync::Arc::new(
                imps.iter()
                    .filter(|i| i.relacion.entailment > 0.0 || i.relacion.contradiction > 0.0)
                    .map(|i| (i.premisa, i.hipotesis, i.relacion.entailment, i.relacion.contradiction))
                    .collect::<Vec<_>>(),
            );
            let posiciones =
                std::sync::Arc::new(layout_fruchterman_reingold(&aserciones, &aristas_grafo));
            // Conservar la selección sólo si la aserción sigue presente.
            if let Some(sel) = model.seleccionada {
                if !aserciones.iter().any(|a| a.asercion.id == sel) {
                    model.seleccionada = None;
                }
            }
            model.aserciones = aserciones;
            model.fuentes = fuentes;
            model.reputaciones = reputaciones;
            model.posiciones = posiciones;
            model.aristas_grafo = aristas_grafo;
            model.n_implicaciones = n_implicaciones;
            model.error = None;
        }
        Err(e) => {
            model.error = Some(e.to_string());
        }
    }
    model.menu_open = None;
    model.menu_contextual = None;
    model
}

fn rama_columna(theme: Theme, children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    // Right-click en la raíz (origen 0,0 ⇒ local == ventana) abre el menú
    // contextual sobre la aserción seleccionada.
    .on_right_click_at(|x, y, _w, _h| Some(Msg::MenuContextual(x, y)))
    .children(children)
}

fn panel_columna(theme: Theme, children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(0.25_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .clip(true)
    .children(children)
}

fn grafo_panel(model: &Model) -> View<Msg> {
    use llimphi_ui::llimphi_raster::vello::Scene;
    let theme = model.theme;
    let posiciones = model.posiciones.clone();
    let aristas = model.aristas_grafo.clone();
    let opiniones: std::sync::Arc<std::collections::HashMap<AsercionId, Opinion>> =
        std::sync::Arc::new(
            model.aserciones.iter()
                .map(|a| (a.asercion.id, a.asercion.opinion_autoral))
                .collect()
        );
    let seleccionada = model.seleccionada;
    // Pre-computar vecinos directos de la selección (si hay).
    let vecinos: std::sync::Arc<std::collections::HashSet<AsercionId>> = std::sync::Arc::new(
        match seleccionada {
            Some(sel) => aristas.iter()
                .filter_map(|(p, h, _, _)| {
                    if *p == sel { Some(*h) }
                    else if *h == sel { Some(*p) }
                    else { None }
                })
                .collect(),
            None => std::collections::HashSet::new(),
        }
    );
    let bg = theme.bg_panel;

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(0.5_f32), height: percent(1.0_f32) },
        flex_grow: 2.0,
        ..Default::default()
    })
    .fill(bg)
    .clip(true)
    .paint_with(move |scene: &mut Scene, _ts, rect: PaintRect| {
        if posiciones.is_empty() {
            return;
        }
        let xform = Affine::IDENTITY;
        let hay_seleccion = seleccionada.is_some();
        // Aristas primero, para que los nodos las cubran en el centro.
        for (premisa, hipotesis, ent, contra) in aristas.iter() {
            let (Some(p), Some(h)) = (posiciones.get(premisa), posiciones.get(hipotesis)) else { continue; };
            let x1 = rect.x + p.0 * rect.w;
            let y1 = rect.y + p.1 * rect.h;
            let x2 = rect.x + h.0 * rect.w;
            let y2 = rect.y + h.1 * rect.h;
            let incidente = hay_seleccion && (seleccionada == Some(*premisa) || seleccionada == Some(*hipotesis));
            let alpha: u8 = if !hay_seleccion || incidente { 0xc0 } else { 0x30 };
            let (color, ancho) = if contra > ent {
                (Color::from_rgba8(0xbf, 0x61, 0x6a, alpha), 1.5 + contra * 3.0)
            } else {
                (Color::from_rgba8(0xa3, 0xbe, 0x8c, alpha), 1.5 + ent * 3.0)
            };
            let mut path = BezPath::new();
            path.move_to((x1 as f64, y1 as f64));
            path.line_to((x2 as f64, y2 as f64));
            scene.stroke(&Stroke::new(ancho as f64), xform, color, None, &path);
        }
        // Nodos.
        for (id, (x, y)) in posiciones.iter() {
            let cx = (rect.x + x * rect.w) as f64;
            let cy = (rect.y + y * rect.h) as f64;
            let op = opiniones.get(id).copied().unwrap_or(Opinion::vacua(0.5).unwrap());
            let es_sel = seleccionada == Some(*id);
            let es_vecino = vecinos.contains(id);
            let prominente = !hay_seleccion || es_sel || es_vecino;
            let alpha: u8 = if prominente { 0xff } else { 0x50 };
            let color = if op.creencia >= op.descreencia && op.creencia >= op.incertidumbre {
                Color::from_rgba8(0xa3, 0xbe, 0x8c, alpha)
            } else if op.descreencia >= op.incertidumbre {
                Color::from_rgba8(0xbf, 0x61, 0x6a, alpha)
            } else {
                Color::from_rgba8(0x88, 0x88, 0x99, alpha)
            };
            // Radio escalado por probabilidad esperada (más opinión, más visible).
            let r = (3.5 + op.creencia.max(op.descreencia) * 4.0) as f64;
            let c = KurboCircle::new((cx, cy), r);
            scene.fill(Fill::NonZero, xform, color, None, &c);
            // Halo oscuro para definirlo sobre el fondo.
            scene.stroke(&Stroke::new(0.8), xform,
                Color::from_rgba8(0x1a, 0x1a, 0x20, alpha), None, &c);
            // Anillo de selección (amber).
            if es_sel {
                let halo = KurboCircle::new((cx, cy), r + 5.0);
                scene.stroke(&Stroke::new(2.5), xform,
                    Color::from_rgba8(0xeb, 0xcb, 0x8b, 0xff), None, &halo);
            } else if es_vecino {
                // Anillo más sutil para vecinos directos (azul-grisáceo).
                let halo = KurboCircle::new((cx, cy), r + 3.0);
                scene.stroke(&Stroke::new(1.5), xform,
                    Color::from_rgba8(0x81, 0xa1, 0xc1, 0xc0), None, &halo);
            }
        }
    })
}

fn etiqueta_seccion(s: impl Into<String>, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(s, 11.0, color, Alignment::Start)
}

fn texto_simple(s: impl Into<String>, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(size + 6.0) },
        padding: Rect {
            left: length(2.0_f32),
            right: length(2.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(s, size, color, Alignment::Start)
}

fn accent_por_opinion(op: &Opinion) -> Color {
    if op.creencia >= op.descreencia && op.creencia >= op.incertidumbre {
        ACCENT_CREENCIA
    } else if op.descreencia >= op.incertidumbre {
        ACCENT_DESCREENCIA
    } else {
        ACCENT_INCERTIDUMBRE
    }
}

fn asercion_card(att: &AsercionAtribuida, seleccionada: bool, theme: &Theme, palette: &CardPalette) -> View<Msg> {
    let op = &att.asercion.opinion_autoral;
    let accent = if att.citada { ACCENT_CITADA } else { accent_por_opinion(op) };

    let texto = texto_simple(
        truncar(&att.asercion.texto, 100),
        12.0,
        theme.fg_text,
    );

    let fuente_str = match &att.fuente {
        Some(f) => {
            let kind = f.kind.as_deref().map(|k| format!(" [{k}]")).unwrap_or_default();
            let cita = if att.citada { " (citada)" } else { "" };
            format!("{}{}{}  ·  {}", f.nombre, kind, cita, att.doc_titulo)
        }
        None => format!("(sin fuente)  ·  {}", att.doc_titulo),
    };
    let fuente_line = texto_simple(fuente_str, 10.0, theme.fg_muted);

    let op_line = texto_simple(
        format!("b={:.2}  d={:.2}  u={:.2}  ·  p̂={:.2}",
            op.creencia, op.descreencia, op.incertidumbre, op.probabilidad_esperada()),
        10.0,
        theme.fg_muted,
    );

    let card = card_view::<Msg>(
        vec![texto, fuente_line, op_line],
        CardOptions { accent: Some(accent), ..Default::default() },
        palette,
    );
    // Marco extra si está seleccionada: re-fill con un highlight bg.
    let card = if seleccionada {
        card.fill(Color::from_rgba8(0x40, 0x40, 0x60, 0xff))
    } else {
        card
    };
    let id = att.asercion.id;
    // Pop-in en la primera aparición de cada aserción (key estable por id):
    // al cargar/recargar el corpus las tarjetas entran con un leve escalado.
    card.on_click(Msg::Seleccionar(id))
        .animated_pop_in(hash_id(&id), motion::NORMAL)
}

fn fuente_card(f: &FuenteResumen, reputacion: Option<f32>, theme: &Theme, palette: &CardPalette) -> View<Msg> {
    let kind = f.fuente.kind.as_deref().map(|k| format!(" [{k}]")).unwrap_or_default();
    let cabecera = texto_simple(
        format!("{}{}", f.fuente.nombre, kind),
        12.0,
        theme.fg_text,
    );
    let conteo = texto_simple(
        format!("{} docs  ·  {} aserciones", f.n_docs, f.n_aserciones),
        10.0,
        theme.fg_muted,
    );
    let mut hijos = vec![cabecera, conteo];
    let accent = if let Some(rep) = reputacion {
        hijos.push(texto_simple(
            format!("reputación: {:+.2}", rep),
            10.0,
            theme.fg_muted,
        ));
        if rep > 0.1 {
            ACCENT_CREENCIA
        } else if rep < -0.1 {
            ACCENT_DESCREENCIA
        } else {
            ACCENT_INCERTIDUMBRE
        }
    } else {
        ACCENT_INCERTIDUMBRE
    };
    card_view::<Msg>(
        hijos,
        CardOptions { accent: Some(accent), ..Default::default() },
        palette,
    )
    .animated_pop_in(hash_fuente(&f.fuente.id), motion::NORMAL)
}

fn truncar(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut o: String = s.chars().take(n).collect();
    o.push('…');
    o
}

fn cargar_modelo(
    db_path: &std::path::Path,
) -> anyhow::Result<(
    Vec<AsercionAtribuida>,
    Vec<FuenteResumen>,
    std::collections::HashMap<FuenteId, f32>,
    usize,
    Vec<Implicacion>,
)> {
    let store = Store::abrir(db_path)?;
    let aserciones = store.cargar_aserciones_atribuidas_todas()?;
    let fuentes = store.listar_fuentes()?;
    let imps = store.cargar_implicaciones_todas()?;
    // Reputaciones: prefiere la tabla persistida (más rápida); fallback al
    // cálculo on-the-fly si la tabla está vacía (DB pre-tabla o nunca corrió
    // `iniy reputacion --recalcular`).
    let persistidas = store.cargar_reputaciones_todas().unwrap_or_default();
    let reputaciones = if !persistidas.is_empty() {
        persistidas.into_iter().map(|r| (r.fuente_id, r.score)).collect()
    } else {
        iniy_store::calcular_reputaciones(&aserciones, &imps)
    };
    let n = imps.len();
    Ok((aserciones, fuentes, reputaciones, n, imps))
}

/// Fruchterman-Reingold simplificado en espacio normalizado [0,1]².
/// Coordenadas iniciales determinísticas por hash del id; 80 iteraciones
/// con cooling lineal. Sin aristas, los nodos se distribuyen
/// repulsivamente en una grilla aproximada.
fn layout_fruchterman_reingold(
    aserciones: &[AsercionAtribuida],
    aristas: &[(AsercionId, AsercionId, f32, f32)],
) -> std::collections::HashMap<AsercionId, (f32, f32)> {
    use std::collections::HashMap;
    let n = aserciones.len();
    if n == 0 {
        return HashMap::new();
    }
    let mut pos: Vec<(f32, f32)> = aserciones
        .iter()
        .map(|a| {
            let h = hash_id(&a.asercion.id);
            let x = ((h & 0xFFFF) as f32 / 0xFFFF as f32) * 0.9 + 0.05;
            let y = (((h >> 16) & 0xFFFF) as f32 / 0xFFFF as f32) * 0.9 + 0.05;
            (x, y)
        })
        .collect();
    let id_a_idx: HashMap<AsercionId, usize> = aserciones.iter().enumerate()
        .map(|(i, a)| (a.asercion.id, i))
        .collect();
    let aristas_idx: Vec<(usize, usize, f32)> = aristas.iter()
        .filter_map(|(p, h, e, c)| {
            let pi = *id_a_idx.get(p)?;
            let hi = *id_a_idx.get(h)?;
            // peso = max(entailment, contradiction) — la fuerza de la conexión.
            Some((pi, hi, e.max(*c)))
        })
        .collect();

    let k = (1.0_f32 / n as f32).sqrt();        // distancia ideal entre nodos
    let mut t: f32 = 0.10;                       // temperatura (paso máximo)
    let iter = 80usize;
    for it in 0..iter {
        let mut disp = vec![(0.0_f32, 0.0_f32); n];
        // Repulsivas: todas con todas.
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = pos[i].0 - pos[j].0;
                let dy = pos[i].1 - pos[j].1;
                let d = (dx * dx + dy * dy).sqrt().max(1e-4);
                let f = (k * k) / d;
                let ux = dx / d;
                let uy = dy / d;
                disp[i].0 += ux * f;
                disp[i].1 += uy * f;
                disp[j].0 -= ux * f;
                disp[j].1 -= uy * f;
            }
        }
        // Atractivas: por arista, ponderada por peso.
        for &(a, b, w) in &aristas_idx {
            let dx = pos[a].0 - pos[b].0;
            let dy = pos[a].1 - pos[b].1;
            let d = (dx * dx + dy * dy).sqrt().max(1e-4);
            let f = (d * d) / k * w;
            let ux = dx / d;
            let uy = dy / d;
            disp[a].0 -= ux * f;
            disp[a].1 -= uy * f;
            disp[b].0 += ux * f;
            disp[b].1 += uy * f;
        }
        // Aplicar desplazamiento, limitado por t. Mantener en [0.05, 0.95].
        for i in 0..n {
            let (mx, my) = disp[i];
            let m = (mx * mx + my * my).sqrt().max(1e-4);
            let dx = (mx / m) * m.min(t);
            let dy = (my / m) * m.min(t);
            pos[i].0 = (pos[i].0 + dx).clamp(0.05, 0.95);
            pos[i].1 = (pos[i].1 + dy).clamp(0.05, 0.95);
        }
        // Cooling lineal.
        t = 0.10 * (1.0 - it as f32 / iter as f32);
    }
    aserciones.iter().enumerate().map(|(i, a)| (a.asercion.id, pos[i])).collect()
}

fn hash_bytes(bytes: [u8; 16]) -> u64 {
    // FNV-1a sobre los 16 bytes del Ulid.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn hash_id(id: &AsercionId) -> u64 {
    hash_bytes(id.0.to_bytes())
}

fn hash_fuente(id: &FuenteId) -> u64 {
    // Namespace distinto al de aserciones para que las keys no colisionen.
    hash_bytes(id.0.to_bytes()) ^ 0xF0E1_D2C3_B4A5_9687
}

/// Cálculo de reputación duplicado del CLI (versión simplificada: solo el
/// score). Para que el explorer no dependa de iniy-cli.
// `calcular_reputaciones` (scoring puro de fuentes) vive ahora en
// `iniy_store` (regla #2): el frontend lo consume, no lo reimplementa.

fn main() {
    llimphi_ui::run::<Explorer>();
}

// Silenciar warnings de imports no usados en este MVP.
#[allow(dead_code)]
fn _suppress_unused() {
    let _ = Asercion {
        id: AsercionId::nuevo(),
        doc_id: iniy_core::DocId::nuevo(),
        chunk_id: iniy_core::ChunkId::nuevo(),
        texto: String::new(),
        opinion_autoral: Opinion::vacua(0.5).unwrap(),
    };
    let _ = GrafoCreencias::nuevo();
}
