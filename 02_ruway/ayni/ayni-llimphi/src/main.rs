// =============================================================================
//  ayni :: ayni-llimphi — la cara gráfica completa del chat soberano
// -----------------------------------------------------------------------------
//  Frontend Llimphi (bucle Elm) DELGADO sobre `ayni-app::Nucleo` —ahí vive toda
//  la lógica: transporte (TCP o minga), persistencia local-first, cifrado 1:1,
//  adjuntos con su blob, y la confianza de P7 (membresía, atestaciones, recibos
//  simétricos)—. La UI sólo pinta el núcleo y captura la intención del humano.
//
//  Dos columnas, controles co-locados (sin toolbars brutas):
//    · GENTE   — miembros (clic = seleccionar), otros vistos, acciones sobre el
//                seleccionado (admitir/expulsar/atestar) y el grafo de confianza.
//    · CHARLA  — el hilo (con scroll y recibos "✓N") + compose con toggles de
//                cifrado/recibos, adjuntar y enviar. La barra `/` acepta comandos.
//
//  Configuración por entorno:
//    AYNI_NOMBRE      nombre → identidad Ed25519 determinista (BLAKE3 del nombre)
//    AYNI_TRANSPORTE  tcp (default) | minga
//    AYNI_ESCUCHAR    bind (default según transporte)
//    AYNI_CONECTAR    peer al que conectarse al arrancar (opcional)
//    AYNI_DATA        ruta del store sled (default ./ayni-<nombre>.db)
//    AYNI_CIFRAR      si está, arranca con el cifrado activo
//    AYNI_RECIBOS     si está, arranca emitiendo recibos (simétrico: actívenlo ambos)
// =============================================================================

use std::env;
use std::sync::Arc;

use ayni_app::{hex_corto, AgoraId, Enlace, EventoRed, Identidad, Nucleo, Tipo};

use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Dimension, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent, Rect};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};

use llimphi_theme::Theme;
use llimphi_widget_text_input::TextInputState;
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_context_menu::{context_menu_view_ex, ContextMenuExtras};
use llimphi_motion::{animate, motion, Tween};
use llimphi_clipboard::SystemClipboard;

/// Cuántos mensajes pinta la ventana visible del hilo (con scroll por rueda).
const VISIBLES: usize = 16;

/// Cierre de cambio de idioma: aplica el locale en caliente y lo persiste.
fn aplicar_idioma(code: &str) {
    let _ = rimay_localize::set_locale(code);
    let mut cfg = wawa_config::WawaConfig::load();
    cfg.lang = code.to_string();
    let _ = cfg.save();
}

#[derive(Clone)]
enum Msg {
    Tecla(KeyEvent),
    Enviar,
    Red(EventoRed),
    /// Selecciona una identidad como blanco de las acciones de GENTE.
    Seleccionar(AgoraId),
    Admitir,
    Expulsar,
    /// Atestiguar al seleccionado (nivel 5 por defecto desde el botón).
    Atestar,
    AcusarRecibo,
    ToggleCifrar,
    ToggleRecibos,
    /// Desplazar el hilo: +N hacia mensajes más viejos, -N hacia los recientes.
    Scroll(i32),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` = cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Navegación por teclado en el menú principal (`+1` baja, `-1` sube).
    MenuNav(i32),
    /// Enter en el menú principal: ejecuta la fila activa.
    MenuActivate,
    /// Tick de animación de menús (sólo re-render).
    MenuTick,
    /// Navegación por teclado en el menú de edición.
    EditNav(i32),
    /// Enter en el menú de edición: ejecuta la fila activa.
    EditActivate,
    /// Right-click en el área de trabajo → abre el menú de edición en
    /// `(x, y)` de ventana, operando sobre el input de mensaje.
    EditMenuOpen(f32, f32),
    /// Acción elegida en el menú de edición.
    EditMenuAction(EditAction),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
}

struct Modelo {
    nucleo: Nucleo,
    enlace: Arc<Enlace>,
    /// El input de mensaje, sobre `EditorState` (selección/undo/clipboard).
    entrada: TextInputState,
    nombre: String,
    transporte: &'static str,
    /// El blanco de las acciones de membresía/confianza.
    seleccionado: Option<AgoraId>,
    /// Mensajes desplazados desde el fondo (0 = pegado a lo más nuevo).
    scroll: usize,
    /// Línea de estado: resultado del último comando/adjuntar.
    aviso: String,
    /// Tema semántico para los widgets reutilizados (menú/edición).
    theme: Theme,
    /// Menú principal: índice del menú raíz abierto (`None` cerrado).
    menu_open: Option<usize>,
    /// Fila resaltada por teclado en el menú principal (`usize::MAX` = ninguna).
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal (0→1).
    menu_anim: Tween<f32>,
    /// Menú de edición contextual: ancla `(x, y)` en ventana (`None` cerrado).
    edit_menu: Option<(f32, f32)>,
    /// Fila resaltada por teclado en el menú de edición (`usize::MAX` = ninguna).
    edit_active: usize,
    /// Animación de aparición del menú de edición (0→1).
    edit_anim: Tween<f32>,
    /// Clipboard del sistema para Cortar/Copiar/Pegar del menú de edición.
    clipboard: SystemClipboard,
}

struct Ayni;

impl App for Ayni {
    type Model = Modelo;
    type Msg = Msg;

    fn title() -> &'static str {
        "ayni · chat soberano"
    }

    fn initial_size() -> (u32, u32) {
        (900, 760)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let nombre = env::var("AYNI_NOMBRE").unwrap_or_else(|_| "yo".into());
        let tipo = Tipo::desde_nombre(&env::var("AYNI_TRANSPORTE").unwrap_or_default());
        let bind = env::var("AYNI_ESCUCHAR").unwrap_or_else(|_| tipo.bind_por_defecto().into());

        let seed = *blake3::hash(nombre.as_bytes()).as_bytes();
        let identidad = Identidad::desde_semilla(seed, nombre.clone());

        let ruta = env::var("AYNI_DATA").unwrap_or_else(|_| format!("./ayni-{nombre}.db"));
        let nucleo = Nucleo::nuevo(
            identidad,
            Some(std::path::Path::new(&ruta)),
            env::var("AYNI_CIFRAR").is_ok(),
            env::var("AYNI_RECIBOS").is_ok(),
        );

        let (enlace, rx) = Enlace::abrir(tipo, &bind)
            .unwrap_or_else(|e| panic!("ayni: no pude abrir el transporte en {bind}: {e}"));
        let transporte = enlace.etiqueta();
        let dir = enlace.direccion_local();
        let enlace = Arc::new(enlace);

        if let Ok(peer) = env::var("AYNI_CONECTAR") {
            let _ = enlace.conectar(&peer);
        }

        // Hilo de red: cada EventoRed se reinyecta al bucle Elm.
        let h = handle.clone();
        std::thread::spawn(move || {
            for evento in rx {
                h.dispatch(Msg::Red(evento));
            }
        });

        Modelo {
            nucleo,
            enlace,
            entrada: TextInputState::new(),
            nombre,
            transporte,
            seleccionado: None,
            scroll: 0,
            aviso: format!("escuchando en {dir} · {transporte}"),
            theme: Theme::dark(),
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_menu: None,
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
            clipboard: SystemClipboard::new(),
        }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Menú principal abierto: flechas navegan, ←/→ cambian de menú raíz
        // (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc cierra.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        // Menú de edición abierto: ↑/↓ navegan, Enter ejecuta, Esc cierra.
        if model.edit_menu.is_some() {
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::EditNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::EditNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::EditActivate),
                _ => None,
            };
        }
        match &e.key {
            Key::Named(NamedKey::Enter) => Some(Msg::Enviar),
            _ => Some(Msg::Tecla(e.clone())),
        }
    }

    fn on_wheel(
        _model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Self::Msg> {
        if delta.y.abs() < f32::EPSILON {
            return None;
        }
        // y>0 ⇒ rueda hacia abajo ⇒ ver más viejos (subir el offset).
        Some(Msg::Scroll(if delta.y > 0.0 { 3 } else { -3 }))
    }

    fn update(mut model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let enlace = model.enlace.clone();
        match msg {
            Msg::Tecla(e) => {
                // Todo el tecleo (incluido Backspace/Delete, selección con
                // Shift+flechas, undo/redo) lo maneja el `EditorState` del input.
                model.entrada.apply_key(&e);
            }
            Msg::Enviar => {
                let texto = model.entrada.text().trim().to_string();
                model.entrada.clear();
                if texto.is_empty() {
                    // nada
                } else if let Some(cmd) = texto.strip_prefix('/') {
                    model.aviso = ejecutar_comando(&mut model.nucleo, enlace.as_ref(), cmd);
                } else {
                    model.nucleo.enviar_texto(enlace.as_ref(), &texto);
                    model.scroll = 0;
                }
            }
            Msg::Seleccionar(id) => {
                model.seleccionado = Some(id);
                model.aviso = format!("seleccionado {}", hex_corto(&id));
            }
            Msg::Admitir => {
                if let Some(s) = model.seleccionado {
                    model.nucleo.admitir(enlace.as_ref(), s);
                    model.aviso = format!("admitiste a {}", hex_corto(&s));
                }
            }
            Msg::Expulsar => {
                if let Some(s) = model.seleccionado {
                    model.nucleo.expulsar(enlace.as_ref(), s);
                    model.aviso = format!("expulsaste a {}", hex_corto(&s));
                }
            }
            Msg::Atestar => {
                if let Some(s) = model.seleccionado {
                    model.nucleo.atestar(enlace.as_ref(), s, 5);
                    model.aviso = format!("das fe de {} (nivel 5)", hex_corto(&s));
                }
            }
            Msg::AcusarRecibo => {
                model.nucleo.acusar_cabezas(enlace.as_ref());
                model.aviso = "acuse de recibo enviado".into();
            }
            Msg::ToggleCifrar => {
                model.nucleo.cifrar = !model.nucleo.cifrar;
                model.aviso = format!("cifrado {}", si_no(model.nucleo.cifrar));
            }
            Msg::ToggleRecibos => {
                model.nucleo.recibos = !model.nucleo.recibos;
                model.aviso = format!("recibos {}", si_no(model.nucleo.recibos));
            }
            Msg::Scroll(d) => {
                let max = model.nucleo.conv.len().saturating_sub(VISIBLES);
                let nuevo = model.scroll as i32 + d;
                model.scroll = nuevo.clamp(0, max as i32) as usize;
            }
            Msg::Red(evento) => {
                model.nucleo.al_evento(enlace.as_ref(), evento);
            }
            Msg::MenuOpen(idx) => {
                model.menu_open = idx;
                model.menu_active = usize::MAX;
                model.edit_menu = None;
                if idx.is_some() {
                    model.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuCommand(cmd) => {
                model = handle_menu_command(model, cmd, enlace.as_ref());
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
                        model = handle_menu_command(model, cmd, enlace.as_ref());
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::EditNav(dir) => {
                let flags =
                    EditFlags::from_editor(model.entrada.editor(), model.entrada.is_masked());
                model.edit_active = editmenu::edit_menu_step(flags, model.edit_active, dir);
            }
            Msg::EditActivate => {
                let flags =
                    EditFlags::from_editor(model.entrada.editor(), model.entrada.is_masked());
                if let Some(action) = editmenu::edit_menu_action_at(flags, model.edit_active) {
                    model.edit_menu = None;
                    editmenu::apply(model.entrada.editor_mut(), action, &mut model.clipboard);
                }
            }
            Msg::EditMenuOpen(x, y) => {
                model.edit_menu = Some((x, y));
                model.edit_active = usize::MAX;
                model.menu_open = None;
                model.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(handle, motion::FAST, || Msg::MenuTick);
            }
            Msg::EditMenuAction(action) => {
                model.edit_menu = None;
                editmenu::apply(model.entrada.editor_mut(), action, &mut model.clipboard);
            }
            Msg::CloseMenus => {
                model.menu_open = None;
                model.menu_active = usize::MAX;
                model.edit_menu = None;
                model.edit_active = usize::MAX;
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let fondo = Color::from_rgba8(18, 21, 28, 255);

        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model));
        let barra = barra_superior(model);
        let cuerpo = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .children(vec![panel_gente(model), columna_charla(model)]);

        // El right-click se engancha en la raíz (origen 0,0 → las coords
        // locales que llegan al handler ya son de ventana) y abre el menú
        // de edición sobre el input de mensaje.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(fondo)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::EditMenuOpen(x, y)))
        .children(vec![menubar, barra, cuerpo, barra_estado(model)])
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // El menú de edición tiene prioridad si está abierto.
        if let Some((x, y)) = model.edit_menu {
            let flags = EditFlags::from_editor(model.entrada.editor(), model.entrada.is_masked());
            let (w, h) = Self::initial_size();
            let mut spec = editmenu::edit_context_menu(
                (x, y),
                (w as f32, h as f32),
                &model.theme,
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            );
            spec.active = model.edit_active;
            return Some(context_menu_view_ex(
                spec,
                ContextMenuExtras {
                    appear: model.edit_anim.value(),
                    ..Default::default()
                },
            ));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model),
            model.menu_active,
            model.menu_anim.value(),
        )
    }
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(menu: &'a app_bus::AppMenu, model: &'a Modelo) -> MenuBarSpec<'a, Msg> {
    let (w, h) = Ayni::initial_size();
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme: &model.theme,
        viewport: (w as f32, h as f32),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

// === Menú principal + comandos ===============================================

/// Construye el menú principal de ayni reflejando el estado actual: los
/// ítems de «Editar» se ponen grises cuando el input no tiene selección /
/// historial / texto; los de «Ayni» según haya alguien seleccionado.
fn app_menu(model: &Modelo) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};
    let ed = model.entrada.editor();
    let has_sel = ed.has_selection();
    let can_undo = ed.can_undo();
    let can_redo = ed.can_redo();
    let has_text = !ed.is_empty();
    let masked = model.entrada.is_masked();
    let hay_sel = model.seleccionado.is_some();

    // Etiquetas de UI localizadas. El segundo argumento de MenuItem::new
    // es el id de comando estable — NO se localiza.
    let t = rimay_localize::t;

    let mut undo = MenuItem::new(t("undo"), "edit.undo").shortcut("Ctrl+Z");
    if !can_undo {
        undo = undo.disabled();
    }
    let mut redo = MenuItem::new(t("redo"), "edit.redo").shortcut("Ctrl+Y");
    if !can_redo {
        redo = redo.disabled();
    }
    let mut cut = MenuItem::new(t("cut"), "edit.cut").shortcut("Ctrl+X").separated();
    let mut copy = MenuItem::new(t("copy"), "edit.copy").shortcut("Ctrl+C");
    if !has_sel || masked {
        cut = cut.disabled();
        copy = copy.disabled();
    }
    let paste = MenuItem::new(t("paste"), "edit.paste").shortcut("Ctrl+V");
    let mut sel_all = MenuItem::new(t("select-all"), "edit.selectall")
        .shortcut("Ctrl+A")
        .separated();
    if !has_text {
        sel_all = sel_all.disabled();
    }

    let mut admitir = MenuItem::new(t("ayni-menu-admitir"), "ayni.admitir");
    let mut atestar = MenuItem::new(t("ayni-menu-atestar"), "ayni.atestar");
    let mut expulsar = MenuItem::new(t("ayni-menu-expulsar"), "ayni.expulsar").separated();
    if !hay_sel {
        admitir = admitir.disabled();
        atestar = atestar.disabled();
        expulsar = expulsar.disabled();
    }

    // Menú de idioma: autónimos sin traducir (convención del SO).
    // El item activo lleva ✔. El comando `lang.<code>` lo resuelve
    // `handle_menu_command` → set_locale + persiste en wawa-config.
    let cur = rimay_localize::current_locale();
    let lang_item = |label: &str, code: &str| {
        let mut it = MenuItem::new(label, format!("lang.{code}"));
        if cur == code {
            it = it.icon("\u{2714}");
        }
        it
    };

    AppMenu::new()
        .menu(
            Menu::new(t("file"))
                .item(MenuItem::new(t("ayni-menu-enviar-msg"), "msg.enviar").shortcut("Enter"))
                .item(MenuItem::new(t("ayni-menu-adjuntar"), "msg.adjuntar").separated())
                .item(MenuItem::new(t("ayni-menu-acuse"), "ayni.recibo")),
        )
        .menu(
            Menu::new(t("edit"))
                .item(undo)
                .item(redo)
                .item(cut)
                .item(copy)
                .item(paste)
                .item(sel_all),
        )
        .menu(
            Menu::new("Ayni")
                .item(MenuItem::new(t("ayni-menu-cifrado"), "ayni.cifrar"))
                .item(MenuItem::new(t("ayni-menu-recibos"), "ayni.recibos").separated())
                .item(admitir)
                .item(atestar)
                .item(expulsar),
        )
        .menu(
            Menu::new(t("help"))
                .item(MenuItem::new(t("ayni-menu-comandos-barra"), "ayuda.comandos")),
        )
        .menu(
            Menu::new(t("language"))
                .item(lang_item("Español", "es-PE"))
                .item(lang_item("English", "en-US"))
                .item(lang_item("Runasimi", "qu-PE")),
        )
}

/// Traduce el `command` del menú principal a la acción real y la ejecuta.
/// Cierra el menú antes de actuar.
fn handle_menu_command(mut model: Modelo, command: String, enlace: &Enlace) -> Modelo {
    model.menu_open = None;
    // Cambio de idioma desde el menú "Idioma": aplica el locale en caliente
    // y lo persiste en wawa-config para que otras apps lo vean también.
    if let Some(code) = command.strip_prefix("lang.") {
        aplicar_idioma(code);
        return model;
    }
    match command.as_str() {
        // Edición → rebota a la acción del menú de edición sobre el input.
        "edit.undo" => {
            editmenu::apply(model.entrada.editor_mut(), EditAction::Undo, &mut model.clipboard);
        }
        "edit.redo" => {
            editmenu::apply(model.entrada.editor_mut(), EditAction::Redo, &mut model.clipboard);
        }
        "edit.cut" => {
            editmenu::apply(model.entrada.editor_mut(), EditAction::Cut, &mut model.clipboard);
        }
        "edit.copy" => {
            editmenu::apply(model.entrada.editor_mut(), EditAction::Copy, &mut model.clipboard);
        }
        "edit.paste" => {
            editmenu::apply(model.entrada.editor_mut(), EditAction::Paste, &mut model.clipboard);
        }
        "edit.selectall" => {
            editmenu::apply(model.entrada.editor_mut(), EditAction::SelectAll, &mut model.clipboard);
        }
        // Mensajería.
        "msg.enviar" => {
            let texto = model.entrada.text().trim().to_string();
            model.entrada.clear();
            if !texto.is_empty() {
                if let Some(cmd) = texto.strip_prefix('/') {
                    model.aviso = ejecutar_comando(&mut model.nucleo, enlace, cmd);
                } else {
                    model.nucleo.enviar_texto(enlace, &texto);
                    model.scroll = 0;
                }
            }
        }
        "msg.adjuntar" => {
            model.aviso = "adjuntar: escribí «/adjuntar <ruta>» en el compose".into();
        }
        // Ayni — membresía / confianza / transporte.
        "ayni.recibo" => {
            model.nucleo.acusar_cabezas(enlace);
            model.aviso = "acuse de recibo enviado".into();
        }
        "ayni.cifrar" => {
            model.nucleo.cifrar = !model.nucleo.cifrar;
            model.aviso = format!("cifrado {}", si_no(model.nucleo.cifrar));
        }
        "ayni.recibos" => {
            model.nucleo.recibos = !model.nucleo.recibos;
            model.aviso = format!("recibos {}", si_no(model.nucleo.recibos));
        }
        "ayni.admitir" => {
            if let Some(s) = model.seleccionado {
                model.nucleo.admitir(enlace, s);
                model.aviso = format!("admitiste a {}", hex_corto(&s));
            }
        }
        "ayni.atestar" => {
            if let Some(s) = model.seleccionado {
                model.nucleo.atestar(enlace, s, 5);
                model.aviso = format!("das fe de {} (nivel 5)", hex_corto(&s));
            }
        }
        "ayni.expulsar" => {
            if let Some(s) = model.seleccionado {
                model.nucleo.expulsar(enlace, s);
                model.aviso = format!("expulsaste a {}", hex_corto(&s));
            }
        }
        "ayuda.comandos" => {
            model.aviso = ejecutar_comando(&mut model.nucleo, enlace, "ayuda");
        }
        _ => {}
    }
    model
}

// === Paleta ==================================================================
const FONDO: (u8, u8, u8) = (18, 21, 28);
const BARRA: (u8, u8, u8) = (28, 33, 44);
const PANEL: (u8, u8, u8) = (23, 27, 36);
const CLARO: (u8, u8, u8) = (222, 230, 240);
const TENUE: (u8, u8, u8) = (120, 135, 155);
const MIO: (u8, u8, u8) = (120, 220, 170);
const AJENO: (u8, u8, u8) = (150, 185, 235);
const SOCIAL: (u8, u8, u8) = (210, 180, 120);
const SEL: (u8, u8, u8) = (60, 90, 80);
const VERDE: (u8, u8, u8) = (70, 200, 140);
const ACENTO: (u8, u8, u8) = (90, 130, 200);

fn c(rgb: (u8, u8, u8)) -> Color {
    Color::from_rgba8(rgb.0, rgb.1, rgb.2, 255)
}

// === Barra superior ==========================================================
fn barra_superior(model: &Modelo) -> View<Msg> {
    let yo = model.nucleo.yo();
    let estado_cifra = match (model.nucleo.cifrar, model.nucleo.tiene_canal()) {
        (true, true) => "🔒 E2EE",
        (true, false) => "🔓 esperando clave",
        _ => "claro",
    };
    let titulo = View::new(estilo_flex_fila(1.0))
        .text_aligned(
            format!(
                "ayni · {} [{}] · {} · {} peer(s) · {}",
                model.nombre,
                hex_corto(&yo),
                model.transporte,
                model.enlace.num_peers(),
                estado_cifra,
            ),
            15.0,
            c(CLARO),
            Alignment::Start,
        );

    let toggles = View::new(Style {
        flex_direction: FlexDirection::Row,
        gap: gap_h(8.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![
        chip(
            format!("cifrar: {}", si_no(model.nucleo.cifrar)),
            if model.nucleo.cifrar { VERDE } else { TENUE },
            Msg::ToggleCifrar,
        ),
        chip(
            format!("recibos: {}", si_no(model.nucleo.recibos)),
            if model.nucleo.recibos { VERDE } else { TENUE },
            Msg::ToggleRecibos,
        ),
    ]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(46.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        padding: lados(16.0, 0.0),
        ..Default::default()
    })
    .fill(c(BARRA))
    .children(vec![titulo, toggles])
}

// === Panel GENTE (membresía + confianza) =====================================
fn panel_gente(model: &Modelo) -> View<Msg> {
    let yo = model.nucleo.yo();
    let memb = model.nucleo.conv.membresia();
    let confianza = model.nucleo.conv.confianza_desde(&yo);
    let conocidos = model.nucleo.conocidos();

    let t = rimay_localize::t;

    let mut hijos: Vec<View<Msg>> = Vec::new();
    hijos.push(rotulo(&t("ayni-label-gente-miembros")));

    for id in &memb.miembros {
        hijos.push(fila_persona(model, id, &memb, &yo));
    }

    // Otros vistos que aún no son miembros.
    let otros: Vec<AgoraId> = conocidos
        .iter()
        .filter(|id| !memb.contiene(id))
        .copied()
        .collect();
    if !otros.is_empty() {
        hijos.push(rotulo(&t("ayni-label-otros-vistos")));
        for id in &otros {
            hijos.push(fila_persona(model, id, &memb, &yo));
        }
    }

    // Acciones sobre el seleccionado.
    hijos.push(rotulo(&t("ayni-label-acciones")));
    let etiqueta_sel = match model.seleccionado {
        Some(s) => format!("blanco: {}", hex_corto(&s)),
        None => t("ayni-label-elige-alguien"),
    };
    hijos.push(
        View::new(estilo_fila_auto()).text_aligned(etiqueta_sel, 13.0, c(TENUE), Alignment::Start),
    );
    hijos.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            gap: gap_h(6.0),
            margin: Rect {
                left: length(0.0),
                right: length(0.0),
                top: length(4.0),
                bottom: length(4.0),
            },
            ..Default::default()
        })
        .children(vec![
            boton(&t("ayni-btn-admitir"), ACENTO, Msg::Admitir),
            boton(&t("ayni-btn-atestar"), SOCIAL, Msg::Atestar),
        ]),
    );
    hijos.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            gap: gap_h(6.0),
            ..Default::default()
        })
        .children(vec![
            boton(&t("ayni-btn-expulsar"), (150, 80, 80), Msg::Expulsar),
            boton(&t("ayni-btn-acuse"), VERDE, Msg::AcusarRecibo),
        ]),
    );

    // Grafo de confianza desde uno mismo.
    hijos.push(rotulo(&t("ayni-label-confianza")));
    if confianza.is_empty() {
        hijos.push(
            View::new(estilo_fila_auto()).text_aligned(
                t("ayni-label-sin-atestaciones"),
                13.0,
                c(TENUE),
                Alignment::Start,
            ),
        );
    } else {
        for (id, saltos) in &confianza {
            hijos.push(
                View::new(estilo_fila_auto()).text_aligned(
                    format!("{} · {}↑", hex_corto(id), saltos),
                    13.0,
                    c(SOCIAL),
                    Alignment::Start,
                ),
            );
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(238.0_f32),
            height: percent(1.0_f32),
        },
        gap: gap_v(5.0),
        padding: lados(12.0, 12.0),
        ..Default::default()
    })
    .fill(c(PANEL))
    .clip(true)
    .children(hijos)
}

/// Una fila de persona: clic = seleccionar; resalta al seleccionado.
fn fila_persona(model: &Modelo, id: &AgoraId, memb: &ayni_app::Membresia, yo: &AgoraId) -> View<Msg> {
    let mut etiqueta = hex_corto(id);
    if Some(*id) == memb.fundador {
        etiqueta.push_str(" ·fund");
    }
    if id == yo {
        etiqueta.push_str(" ←vos");
    }
    if model.nucleo.reciproca(id) {
        etiqueta.push_str(" ✓rx");
    }
    let seleccionado = model.seleccionado == Some(*id);
    let fondo = if seleccionado { SEL } else { PANEL };
    let color = if id == yo { MIO } else { CLARO };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: lados(6.0, 0.0),
        ..Default::default()
    })
    .fill(c(fondo))
    .hover_fill(c(SEL))
    .radius(5.0)
    .text_aligned(etiqueta, 13.0, c(color), Alignment::Start)
    .on_click(Msg::Seleccionar(*id))
}

// === Columna CHARLA (hilo + compose) =========================================
fn columna_charla(model: &Modelo) -> View<Msg> {
    let yo = model.nucleo.yo();
    let recibos = model.nucleo.conv.recibos();
    let nodos = model.nucleo.conv.instantanea();
    let n = nodos.len();
    let fin = n.saturating_sub(model.scroll);
    let ini = fin.saturating_sub(VISIBLES);

    let mut filas: Vec<View<Msg>> = Vec::new();
    if n == 0 {
        filas.push(
            View::new(estilo_fila_auto()).text_aligned(
                rimay_localize::t("ayni-label-sin-mensajes"),
                14.0,
                c(TENUE),
                Alignment::Start,
            ),
        );
    }
    for nodo in &nodos[ini..fin] {
        let propio = *nodo.autor() == yo;
        let es_social = !matches!(
            nodo.contenido.carga,
            ayni_app::Carga::Texto(_) | ayni_app::Carga::Cifrado(_)
        );
        let color = if es_social {
            SOCIAL
        } else if propio {
            MIO
        } else {
            AJENO
        };
        let vistos = recibos.get(&nodo.id()).map(|s| s.len()).unwrap_or(0);
        let sello = if vistos > 0 { format!("  ✓{vistos}") } else { String::new() };
        let linea = format!(
            "[{}] {}{}",
            hex_corto(nodo.autor()),
            model.nucleo.texto_visible(nodo),
            sello
        );
        filas.push(
            View::new(estilo_fila_auto()).text_aligned(linea, 15.0, c(color), Alignment::Start),
        );
    }

    let hint = if model.scroll > 0 {
        format!("⟂ scroll +{} (rueda)   ", model.scroll)
    } else {
        String::new()
    };
    let lista = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        gap: gap_v(7.0),
        padding: lados(16.0, 12.0),
        ..Default::default()
    })
    .fill(c(FONDO))
    .clip(true)
    .children(filas);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![lista, fila_compose(model, &hint)])
}

fn fila_compose(model: &Modelo, hint: &str) -> View<Msg> {
    let actual = model.entrada.text();
    let (texto, color) = if actual.is_empty() {
        (
            format!("{hint}{}", rimay_localize::t("ayni-compose-placeholder")),
            TENUE,
        )
    } else {
        (format!("{actual}▏"), CLARO)
    };
    let caja = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        padding: lados(12.0, 0.0),
        ..Default::default()
    })
    .fill(c((36, 42, 55)))
    .radius(8.0)
    .text_aligned(texto, 15.0, c(color), Alignment::Start);

    let fila = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(56.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: gap_h(8.0),
        padding: lados(16.0, 8.0),
        ..Default::default()
    })
    .fill(c(BARRA))
    .children(vec![caja, boton(&rimay_localize::t("ayni-btn-enviar"), VERDE, Msg::Enviar)]);
    fila
}

// === Barra de estado =========================================================
fn barra_estado(model: &Modelo) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: lados(16.0, 0.0),
        ..Default::default()
    })
    .fill(c(PANEL))
    .text_aligned(format!("· {}", model.aviso), 12.0, c(TENUE), Alignment::Start)
}

// === Comandos de la barra `/` ================================================
fn ejecutar_comando(nucleo: &mut Nucleo, enlace: &Enlace, cmd: &str) -> String {
    let mut campos = cmd.split_whitespace();
    let verbo = campos.next().unwrap_or("");
    match verbo {
        "adjuntar" | "adj" => {
            let ruta = campos.collect::<Vec<_>>().join(" ");
            if ruta.is_empty() {
                return "uso: /adjuntar <ruta>".into();
            }
            match nucleo.adjuntar(enlace, &ruta) {
                Ok(n) => format!("adjuntado: {n}"),
                Err(e) => e,
            }
        }
        "admitir" | "expulsar" | "atestar" => {
            let Some(pref) = campos.next() else {
                return format!("uso: /{verbo} <hex>");
            };
            let Some(sujeto) = nucleo.resolver(pref) else {
                return format!("no conozco a «{pref}»");
            };
            match verbo {
                "admitir" => {
                    nucleo.admitir(enlace, sujeto);
                    format!("admitiste a {}", hex_corto(&sujeto))
                }
                "expulsar" => {
                    nucleo.expulsar(enlace, sujeto);
                    format!("expulsaste a {}", hex_corto(&sujeto))
                }
                _ => {
                    let nivel: u8 = campos.next().and_then(|s| s.parse().ok()).unwrap_or(5);
                    nucleo.atestar(enlace, sujeto, nivel);
                    format!("das fe de {} (nivel {nivel})", hex_corto(&sujeto))
                }
            }
        }
        "recibo" => {
            nucleo.acusar_cabezas(enlace);
            "acuse de recibo enviado".into()
        }
        "ayuda" | "" => {
            "/adjuntar <ruta> · /admitir <hex> · /expulsar <hex> · /atestar <hex> [nivel] · /recibo"
                .into()
        }
        otro => format!("comando desconocido: «{otro}» (/ayuda)"),
    }
}

// === Helpers de vista ========================================================
fn chip(label: String, color_borde: (u8, u8, u8), msg: Msg) -> View<Msg> {
    View::new(Style {
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: lados(10.0, 5.0),
        ..Default::default()
    })
    .fill(c((40, 46, 60)))
    .radius(12.0)
    .text(label, 13.0, c(color_borde))
    .on_click(msg)
}

fn boton(label: &str, bg: (u8, u8, u8), msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(86.0_f32),
            height: length(34.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(c(bg))
    .radius(8.0)
    .text(label.to_string(), 14.0, c((12, 18, 14)))
    .on_click(msg)
}

fn rotulo(texto: &str) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        margin: Rect {
            left: length(0.0),
            right: length(0.0),
            top: length(8.0),
            bottom: length(0.0),
        },
        ..Default::default()
    })
    .text_aligned(texto.to_uppercase(), 11.0, c(ACENTO), Alignment::Start)
}

fn estilo_fila_auto() -> Style {
    Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    }
}

fn estilo_flex_fila(grow: f32) -> Style {
    Style {
        size: Size {
            width: Dimension::auto(),
            height: Dimension::auto(),
        },
        flex_grow: grow,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    }
}

fn lados(h: f32, v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(h),
        right: length(h),
        top: length(v),
        bottom: length(v),
    }
}

fn gap_h(x: f32) -> Size<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Size {
        width: length(x),
        height: length(0.0),
    }
}

fn gap_v(y: f32) -> Size<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Size {
        width: length(0.0),
        height: length(y),
    }
}

fn si_no(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

fn main() {
    rimay_localize::init();
    let wawa_cfg = wawa_config::WawaConfig::load();
    let _ = rimay_localize::set_locale(&wawa_cfg.lang);
    llimphi_ui::run::<Ayni>();
}
