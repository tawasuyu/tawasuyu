//! `menu` — menú principal de khipu y menú de edición contextual.
//!
//! Construye el `AppMenu` reflejando el estado del campo focuseado e
//! informa los `EditFlags` del campo activo. El despacho de comandos a
//! `Msg` reales vive en `app.rs` para evitar un ciclo de módulos.

use llimphi_theme::Theme;
use llimphi_widget_edit_menu::{EditFlags};
use llimphi_widget_menubar::{MenuBarSpec, DEFAULT_HEIGHT as MENU_H};
use llimphi_widget_text_editor::EditorState;

use crate::modelo::{Focus, Model, Msg};

// =====================================================================
// Helpers de foco
// =====================================================================

/// Devuelve el `EditorState` del campo focuseado (referencia inmutable) y
/// si está enmascarado (passphrase). Sin foco editable devuelve `None`.
pub(crate) fn focused_editor(model: &Model) -> (Option<&EditorState>, bool) {
    match model.focus {
        Focus::Body => (Some(&model.body), false),
        Focus::Title => (Some(model.title.editor()), false),
        Focus::Tags => (Some(model.tags.editor()), false),
        Focus::Search => (Some(model.search.editor()), false),
        Focus::PeerAddr => (Some(model.peer_input.editor()), false),
        Focus::Region => (Some(model.region_input.editor()), false),
        Focus::Passphrase => (Some(model.passphrase.editor()), model.passphrase.is_masked()),
        Focus::None => (None, false),
    }
}

/// `EditFlags` del campo focuseado, para nav/ejecución por teclado del
/// menú de edición. Sin campo focuseado, flags vacíos (todo gris).
pub(crate) fn focused_edit_flags(model: &Model) -> EditFlags {
    let (editor, masked) = focused_editor(model);
    match editor {
        Some(ed) => EditFlags::from_editor(ed, masked),
        None => EditFlags::default(),
    }
}

// =====================================================================
// Menú principal
// =====================================================================

/// Construye el menú principal de khipu reflejando el estado del campo
/// focuseado (ítems de Editar grises sin selección / historial).
pub(crate) fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};
    let (editor, _masked) = focused_editor(model);
    let has_sel = editor.map(|e| e.has_selection()).unwrap_or(false);
    let can_undo = editor.map(|e| e.can_undo()).unwrap_or(false);
    let can_redo = editor.map(|e| e.can_redo()).unwrap_or(false);
    let has_field = editor.is_some();
    let has_sel_note = model.selected.is_some();

    let mut undo = MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z");
    if !can_undo {
        undo = undo.disabled();
    }
    let mut redo = MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Y");
    if !can_redo {
        redo = redo.disabled();
    }
    let mut cut = MenuItem::new("Cortar", "edit.cut").shortcut("Ctrl+X").separated();
    let mut copy = MenuItem::new("Copiar", "edit.copy").shortcut("Ctrl+C");
    if !has_sel {
        cut = cut.disabled();
        copy = copy.disabled();
    }
    let mut paste = MenuItem::new("Pegar", "edit.paste").shortcut("Ctrl+V");
    if !has_field {
        paste = paste.disabled();
    }
    let mut sel_all = MenuItem::new("Seleccionar todo", "edit.selectall")
        .shortcut("Ctrl+A")
        .separated();
    if !has_field {
        sel_all = sel_all.disabled();
    }

    let mut delete_note = MenuItem::new("Borrar nota", "note.delete");
    if !has_sel_note {
        delete_note = delete_note.disabled();
    }
    let archive_label = if model.show_archive {
        "Ocultar archivadas"
    } else {
        "Ver archivadas"
    };

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Nueva nota", "note.new").shortcut("Ctrl+N"))
                .item(delete_note)
                .item(MenuItem::new(archive_label, "note.archive").separated())
                .item(MenuItem::new("Exportar sobre…", "share.export"))
                .item(MenuItem::new("Importar sobre…", "share.import")),
        )
        .menu(
            Menu::new("Editar")
                .item(undo)
                .item(redo)
                .item(cut)
                .item(copy)
                .item(paste)
                .item(sel_all),
        )
        .menu(
            Menu::new("Compartir")
                .item(MenuItem::new("Publicar (P2P)", "share.publish"))
                .item(MenuItem::new("Recibir de un par…", "share.receive")),
        )
        .menu(
            Menu::new("Ayuda")
                .item(MenuItem::new("Buscar (foco)", "view.search").shortcut("Ctrl+F"))
                .item(MenuItem::new("Acerca de khipu", "help.about")),
        )
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
/// Recibe el tamaño del viewport para no crear una dependencia de `KhipuApp`.
pub(crate) fn menubar_spec<'a>(
    menu: &'a app_bus::AppMenu,
    model: &Model,
    theme: &'a Theme,
    viewport: (f32, f32),
) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport,
        height: MENU_H,
        on_open: std::sync::Arc::new(Msg::MenuOpen),
        on_command: std::sync::Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}
