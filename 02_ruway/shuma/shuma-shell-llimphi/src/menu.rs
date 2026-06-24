//! Menú principal (barra) + menú contextual de terminal del chasis shuma.
//!
//! El input del shell es una línea de comando (`shuma_line::LineState`),
//! NO un `EditorState`/`TextInputState` estándar — por eso este menú no
//! usa el widget `edit-menu` (que necesita un `EditorState` con modelo de
//! selección). En su lugar arma a mano un menú contextual de terminal con
//! sólo las acciones que el módulo shell YA expone: pegar, limpiar la
//! entrada, limpiar la pantalla y cancelar el comando vivo.
//!
//! El menú principal (Archivo / Editar / Ver / Ayuda) mapea cada comando
//! al `Msg` real correspondiente del chasis o del módulo shell focado.

use std::sync::Arc;

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_theme::Theme;
use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers, NamedKey, View};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{
    menubar_overlay_animated, menubar_view, MenuBarSpec, DEFAULT_HEIGHT as MENU_H,
};

use super::{Model, Msg, ModuleMsg, ModuleState, Slot, Which};

// ─── Estado del shell focado (lo que habilita/deshabilita el menú) ──

/// Snapshot de lo que el menú necesita saber del shell que recibe las
/// teclas ahora mismo. `None` si la tab/slot activo no es un shell.
pub(crate) struct FocusInfo {
    /// Slot al que enrutar las acciones del menú.
    pub slot: Slot,
    /// `true` si la línea de comando tiene texto (habilita "Limpiar entrada").
    pub has_input: bool,
    /// `true` si hay un comando ejecutándose (habilita "Cancelar").
    pub running: bool,
}

/// Encuentra el shell focado siguiendo la misma prioridad que
/// `forward_key_to_focused_shell`: slot `main` primero, luego el tab
/// activo. Devuelve `None` si ninguno de los dos es un shell.
pub(crate) fn focused_shell(model: &Model) -> Option<FocusInfo> {
    let from = |slot: Slot, state: &ModuleState| match state {
        ModuleState::Shell(s) => Some(FocusInfo {
            slot,
            has_input: !s.input.is_empty(),
            running: s.is_running(),
        }),
        _ => None,
    };
    if let Some(inst) = model.main.as_ref() {
        if let Some(info) = from(Slot::Main, &inst.state) {
            return Some(info);
        }
    }
    // El shell de la sesión activa es el canvas principal → siempre recibe
    // teclas (a menos que un menú las intercepte).
    if let Some(s) = model.active() {
        if let Some(info) = from(
            Slot::Session(model.active_session, Which::Shell),
            &s.shell().state,
        ) {
            return Some(info);
        }
    }
    None
}

// ─── Menú principal ────────────────────────────────────────────────

/// Arma el `AppMenu` reflejando el estado real del shell focado: los
/// ítems de Editar se deshabilitan cuando la acción no aplica.
pub(crate) fn app_menu(model: &Model) -> AppMenu {
    let focus = focused_shell(model);
    let has_input = focus.as_ref().map(|f| f.has_input).unwrap_or(false);
    let running = focus.as_ref().map(|f| f.running).unwrap_or(false);
    let is_shell = focus.is_some();

    // Alias para la función de localización.
    let t = rimay_localize::t;

    // Archivo: lo único universal y honesto es salir del proceso.
    // «Endockar» (ventana → barra) o «Modo ventana» (barra → ventana) según el
    // modo actual. Re-lanza el binario en el modo opuesto (ver `respawn_mode`).
    let dock_label = if model.dock_mode {
        "Modo ventana"
    } else {
        "Endockar"
    };
    let archivo = Menu::new(t("file"))
        .item(MenuItem::new(dock_label, "window.toggle-dock").separated())
        .item(MenuItem::new(t("exit"), "app.quit").shortcut("Ctrl+Q"));

    // Editar: opera sobre la línea de comando del shell focado. Sin
    // copiar/cortar porque `LineState` no tiene modelo de selección.
    let mut pegar = MenuItem::new(t("paste"), "edit.paste").shortcut("Ctrl+V");
    let mut limpiar_in = MenuItem::new(t("shuma-shell-clear-input"), "edit.clear-input");
    if !is_shell {
        pegar = pegar.disabled();
    }
    if !has_input {
        limpiar_in = limpiar_in.disabled();
    }
    let editar = Menu::new(t("edit")).item(pegar).item(limpiar_in);

    // Ver: limpiar pantalla + cancelar comando + selector de tabs.
    let mut limpiar_pant = MenuItem::new(t("shuma-shell-clear-screen"), "term.clear");
    let mut cancelar = MenuItem::new(t("shuma-shell-cancel-cmd"), "term.cancel").shortcut("Ctrl+C");
    if !is_shell {
        limpiar_pant = limpiar_pant.disabled();
    }
    if !running {
        cancelar = cancelar.disabled();
    }
    let mut ver = Menu::new(t("view")).item(limpiar_pant).item(cancelar);
    // Disposiciones guardadas (estilo sesiones de tmux): guardar/restaurar el
    // espacio de trabajo entero. Siempre disponible (no depende del shell).
    ver = ver.item(MenuItem::new(t("shuma-layouts"), "view.layouts").separated());
    // Una entrada por sesión para saltar directo (mapea a `Msg::SelectSession`).
    for (i, s) in model.sessions.iter().enumerate() {
        let mut it = MenuItem::new(s.name.clone(), format!("view.session.{i}"));
        if i == 0 {
            it = it.separated();
        }
        if i == model.active_session {
            it = it.disabled(); // ya estás acá
        }
        ver = ver.item(it);
    }

    // Ayuda: imprime una línea "acerca de" en la entrada del shell
    // focado (efecto visible y real; sin diálogos que el chasis no tiene).
    let mut acerca = MenuItem::new(t("shuma-shell-about"), "help.about");
    if !is_shell {
        acerca = acerca.disabled();
    }
    let ayuda = Menu::new(t("help")).item(acerca);

    // Menú de idioma: autónimos sin traducir (convención del SO). El item
    // activo lleva ✔. El comando `lang.<code>` lo resuelve `handle_command`
    // → set_locale + persiste en wawa-config.
    let cur = rimay_localize::current_locale();
    let lang_item = |label: &str, code: &str| {
        let mut it = MenuItem::new(label, format!("lang.{code}"));
        if cur == code {
            it = it.icon("\u{2714}");
        }
        it
    };
    let idioma = Menu::new(t("language"))
        .item(lang_item("Español", "es-PE"))
        .item(lang_item("English", "en-US"))
        .item(lang_item("Runasimi", "qu-PE"));

    AppMenu::new()
        .menu(archivo)
        .menu(editar)
        .menu(ver)
        .menu(perfiles_menu(model))
        .menu(ayuda)
        .menu(idioma)
}

/// El menú **Perfiles**: tres bloques conmutables con un clic — atajos
/// (globales), apariencia (global + de la sesión actual) y perfiles de sesión
/// (contextos tipo Firefox). El activo lleva ●.
fn perfiles_menu(model: &Model) -> Menu {
    const DOT: &str = "\u{25CF}"; // ●
    let header = |label: &str| MenuItem::new(label, "noop").disabled().separated();
    let mut menu = Menu::new("Perfiles");

    // Gestor (crear/duplicar/renombrar/borrar).
    menu = menu.item(MenuItem::new("Gestionar perfiles…", "prof.manage"));

    // ── Atajos (global) ──
    menu = menu.item(header("Atajos"));
    let sk_active = model.shortcuts.active().to_string();
    for name in model.shortcuts.names() {
        let mut it = MenuItem::new(name.clone(), format!("prof.sk.{name}"));
        if name == sk_active {
            it = it.icon(DOT);
        }
        menu = menu.item(it);
    }

    // ── Apariencia (default global) ──
    menu = menu.item(header("Apariencia (global)"));
    let ap_active = model.appearance.active().to_string();
    for name in model.appearance.names() {
        let mut it = MenuItem::new(name.clone(), format!("prof.ap.{name}"));
        if name == ap_active {
            it = it.icon(DOT);
        }
        menu = menu.item(it);
    }

    // ── Apariencia de ESTA sesión (la "ventana") ──
    menu = menu.item(header("Apariencia (esta sesión)"));
    let sess_ap = model
        .active()
        .and_then(|s| s.appearance.clone());
    let mut como_global = MenuItem::new("Como el global", "prof.aps.~");
    if sess_ap.is_none() {
        como_global = como_global.icon(DOT);
    }
    menu = menu.item(como_global);
    for name in model.appearance.names() {
        let mut it = MenuItem::new(name.clone(), format!("prof.aps.{name}"));
        if sess_ap.as_deref() == Some(name.as_str()) {
            it = it.icon(DOT);
        }
        menu = menu.item(it);
    }

    // ── Sesión (contextos tipo Firefox) ──
    menu = menu.item(header("Sesión / contexto"));
    let se_active = model.session_profiles.active().to_string();
    for name in model.session_profiles.names() {
        let mut it = MenuItem::new(name.clone(), format!("prof.sess.{name}"));
        if *name == se_active {
            it = it.icon(DOT);
        }
        menu = menu.item(it);
    }
    menu = menu.item(MenuItem::new("Nuevo contexto", "prof.sess.new").separated());

    menu
}

/// `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
pub(crate) fn menubar_spec<'a>(
    menu: &'a AppMenu,
    model: &Model,
    theme: &'a Theme,
) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: viewport(),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// La fila de títulos — primer hijo del column raíz de `view()`.
pub(crate) fn menubar_row(model: &Model, theme: &Theme) -> View<Msg> {
    let menu = app_menu(model);
    menubar_view(&menubar_spec(&menu, model, theme))
}

// ─── Menú contextual de terminal (right-click) ─────────────────────

/// Construye el overlay a mostrar: prioriza el menú contextual de
/// terminal; si no, el dropdown del menú principal abierto.
pub(crate) fn overlay(model: &Model) -> Option<View<Msg>> {
    if let Some((i, x, y)) = model.tab_ctx {
        return Some(tab_context_menu(model, i, x, y));
    }
    if let Some((x, y)) = model.ctx_menu {
        return Some(terminal_context_menu(model, x, y));
    }
    let menu = app_menu(model);
    menubar_overlay_animated(
        &menubar_spec(&menu, model, &model.theme),
        model.menu_active,
        model.menu_anim.value(),
    )
}

fn terminal_context_menu(model: &Model, x: f32, y: f32) -> View<Msg> {
    let focus = focused_shell(model);
    let has_input = focus.as_ref().map(|f| f.has_input).unwrap_or(false);
    let running = focus.as_ref().map(|f| f.running).unwrap_or(false);
    let is_shell = focus.is_some();

    // "Pegar" reusa la ruta Ctrl+V del módulo, que internamente decide
    // si pega a la línea de comando o al PTY (cuando hay un TUI vt100).
    let t = rimay_localize::t;
    let mut pegar = ContextMenuItem::action(t("paste")).with_shortcut("Ctrl+V");
    let mut limpiar_in = ContextMenuItem::action(t("shuma-shell-clear-input"));
    let mut limpiar_pant = ContextMenuItem::action(t("shuma-shell-clear-screen"));
    let mut cancelar = ContextMenuItem::action(t("shuma-shell-cancel-cmd")).with_shortcut("Ctrl+C");

    if !is_shell {
        pegar = pegar.disabled();
        limpiar_pant = limpiar_pant.disabled();
    }
    if !has_input {
        limpiar_in = limpiar_in.disabled();
    }
    if !running {
        cancelar = cancelar.disabled();
    }

    // Orden de items — el índice es el que recibe `on_pick`.
    let items = vec![
        pegar,                          // 0
        limpiar_in,                     // 1
        ContextMenuItem::separator(),   // 2
        limpiar_pant,                   // 3
        cancelar,                       // 4
    ];

    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(|i: usize| {
        let cmd = match i {
            0 => "edit.paste",
            1 => "edit.clear-input",
            3 => "term.clear",
            4 => "term.cancel",
            _ => "noop",
        };
        Msg::MenuCommand(cmd.to_string())
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport(),
        header: Some(rimay_localize::t("terminal")),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

/// Menú contextual de una **tab** (click derecho sobre el chip): operaciones de
/// tab sin la ✕ riesgosa. `i` es el índice de la tab clickeada.
fn tab_context_menu(model: &Model, i: usize, x: f32, y: f32) -> View<Msg> {
    let n_tabs = model
        .active()
        .map(|s| s.workspace.tabs.len())
        .unwrap_or(1);

    let mut cerrar = ContextMenuItem::action("Cerrar tab");
    let mut cerrar_otras = ContextMenuItem::action("Cerrar otras");
    if n_tabs <= 1 {
        cerrar = cerrar.disabled();
        cerrar_otras = cerrar_otras.disabled();
    }

    // Orden de items — el índice es el que recibe `on_pick`.
    let items = vec![
        ContextMenuItem::action("Nueva tab"), // 0
        ContextMenuItem::separator(),         // 1
        cerrar,                               // 2
        cerrar_otras,                         // 3
    ];

    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |k: usize| match k {
        0 => Msg::TabNew,
        2 => Msg::TabClose(i),
        3 => Msg::TabCloseOthers(i),
        _ => Msg::CloseMenus,
    });

    context_menu_view(ContextMenuSpec {
        anchor: (x, y),
        viewport: viewport(),
        header: Some(format!("Tab {}", i + 1)),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: Msg::CloseMenus,
        palette: ContextMenuPalette::from_theme(&model.theme),
    })
}

// ─── Ruteo de comandos del menú a Msg/acciones reales ──────────────

/// Traduce el `command` string de un ítem de menú a una transición del
/// modelo. Devuelve el modelo modificado (cerrando antes los menús).
pub(crate) fn handle_command(mut model: Model, cmd: &str) -> Model {
    model.menu_open = None;
    model.menu_active = usize::MAX;
    model.ctx_menu = None;

    // Cambio de idioma desde el menú "Idioma": aplica el locale en caliente
    // y lo persiste en la capa de usuario de wawa-config. El watcher de la
    // propia app (y el del resto) reentra con `WawaConfigChanged`, así el
    // cambio se propaga a todas las apps abiertas y sobrevive reinicios.
    if let Some(code) = cmd.strip_prefix("lang.") {
        let _ = rimay_localize::set_locale(code);
        let mut cfg = wawa_config::WawaConfig::load();
        cfg.lang = code.to_string();
        let _ = cfg.save();
        return model;
    }

    // ── Perfiles ──
    // Abre el modal de gestión (crear/duplicar/renombrar/borrar).
    if cmd == "prof.manage" {
        model.perfiles_modal_open = true;
        model.prof_name_focused = true;
        return model;
    }
    // Atajos (global): conmuta el keymap activo y lo persiste.
    if let Some(name) = cmd.strip_prefix("prof.sk.") {
        if model.shortcuts.set_active(name).is_ok() {
            model.pending_prefix = false;
            if let Some(p) = crate::perfiles::shortcuts::ShortcutProfiles::default_path() {
                let _ = model.shortcuts.save(&p);
            }
        }
        return model;
    }
    // Apariencia de la sesión activa (la "ventana"): "~" = como el global.
    if let Some(name) = cmd.strip_prefix("prof.aps.") {
        let val = if name == "~" { None } else { Some(name.to_string()) };
        if let Some(s) = model.sessions.get_mut(model.active_session) {
            s.appearance = val;
        }
        super::persist::save_sessions(&model);
        crate::perfiles::apply_active_appearance(&mut model);
        return model;
    }
    // Apariencia global (default de toda ventana).
    if let Some(name) = cmd.strip_prefix("prof.ap.") {
        if model.appearance.set_active(name).is_ok() {
            if let Some(p) = crate::perfiles::appearance::AppearanceProfiles::default_path() {
                let _ = model.appearance.save(&p);
            }
            crate::perfiles::apply_active_appearance(&mut model);
        }
        return model;
    }
    // Perfil de sesión (contexto tipo Firefox).
    if cmd == "prof.sess.new" {
        // Auto-nombre "contexto N" libre.
        let mut n = model.session_profiles.names().len();
        let name = loop {
            let candidate = format!("contexto {n}");
            if !model.session_profiles.contains(&candidate) {
                break candidate;
            }
            n += 1;
        };
        if model.session_profiles.create(&name).is_ok() {
            if let Some(p) = crate::perfiles::sessions::SessionProfiles::default_path() {
                let _ = model.session_profiles.save(&p);
            }
            model = super::switch_session_profile(model, &name);
        }
        return model;
    }
    if let Some(name) = cmd.strip_prefix("prof.sess.") {
        model = super::switch_session_profile(model, name);
        return model;
    }

    // Selector de sesión: "view.session.<i>".
    if let Some(rest) = cmd.strip_prefix("view.session.") {
        if let Ok(i) = rest.parse::<usize>() {
            if i < model.sessions.len() {
                model.active_session = i;
            }
        }
        return model;
    }

    match cmd {
        "app.quit" => {
            std::process::exit(0);
        }
        "window.toggle-dock" => {
            // Re-lanza en el modo opuesto y cierra esta instancia. La sesión no
            // se migra viva (arranca limpia) — ver `respawn_mode`.
            crate::respawn_mode(!model.dock_mode);
            std::process::exit(0);
        }
        "edit.paste" => route_to_shell(model, shell_paste_key()),
        "edit.clear-input" => {
            if let Some(focus) = focused_shell(&model) {
                clear_input(&mut model, &focus.slot);
            }
            model
        }
        "view.layouts" => {
            model.layouts_modal_open = true;
            model.layout_name_focused = true; // listo para tipear el nombre
            model
        }
        "term.clear" => route_to_shell(model, ModuleMsg::Shell(shuma_module_shell::Msg::Clear)),
        "term.cancel" => route_to_shell(model, ModuleMsg::Shell(shuma_module_shell::Msg::Cancel)),
        "help.about" => {
            let line = format!(
                "# shuma — shell soberano · {} sesiones",
                model.sessions.len()
            );
            route_to_shell(
                model,
                ModuleMsg::Shell(shuma_module_shell::Msg::InsertAtCursor(line)),
            )
        }
        _ => model,
    }
}

/// Enruta un `ModuleMsg` al shell focado (si lo hay). No-op si el slot
/// activo no es un shell.
fn route_to_shell(model: Model, msg: ModuleMsg) -> Model {
    match focused_shell(&model) {
        Some(focus) => super::apply_module_msg(model, focus.slot, msg),
        None => model,
    }
}

/// Vacía la línea de comando del shell en `slot` mutando su `LineState`
/// directamente (no hay un `Msg` de "limpiar entrada" en el módulo).
fn clear_input(model: &mut Model, slot: &Slot) {
    let inst = super::instance_for_slot_mut(model, slot);
    if let Some(inst) = inst {
        if let ModuleState::Shell(s) = &mut inst.state {
            s.input.clear();
        }
    }
}

/// `KeyEvent` sintético Ctrl+V — reusa la ruta de paste que el módulo
/// shell ya implementa (clipboard → input, o clipboard → PTY si hay TUI).
fn shell_paste_key() -> ModuleMsg {
    ModuleMsg::Shell(shuma_module_shell::Msg::Key(KeyEvent {
        key: Key::Character("v".into()),
        state: KeyState::Pressed,
        text: None,
        modifiers: Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
        repeat: false,
    }))
}

// ─── Navegación por teclado del menú (Esc cierra) ──────────────────

/// Si hay algún menú abierto, intercepta Esc para cerrarlo. Devuelve
/// `Some(Msg::CloseMenus)` para que `on_key` corte el reenvío al shell.
pub(crate) fn intercept_key(model: &Model, e: &KeyEvent) -> Option<Msg> {
    // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
    // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc
    // cierra. El context-menu de terminal queda mouse-only (sólo Esc).
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
    if model.ctx_menu.is_some() && matches!(e.key, Key::Named(NamedKey::Escape)) {
        return Some(Msg::CloseMenus);
    }
    None
}

/// Viewport para clampear los menús — shuma no trackea el tamaño de la
/// ventana, así que usamos el tamaño inicial (igual que `nada`).
fn viewport() -> (f32, f32) {
    let (w, h) = <super::Shell as llimphi_ui::App>::initial_size();
    (w as f32, h as f32)
}
