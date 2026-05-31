//! `llimphi-widget-edit-menu` — el menú de edición estándar para
//! cualquier campo de texto Llimphi.
//!
//! Tanto el input single-line ([`llimphi_widget_text_input`]) como el
//! editor IDE enriquecido ([`llimphi_widget_text_editor`]) se apoyan en
//! el mismo [`EditorState`]. Este widget arma, a partir de ese estado,
//! el menú contextual canónico:
//!
//! ```text
//!   ┃ EDICIÓN
//!   ┃ Deshacer            Ctrl+Z
//!   ┃ Rehacer             Ctrl+Y
//!   ┃ ─────────────────────
//!   ┃ Cortar              Ctrl+X
//!   ┃ Copiar              Ctrl+C
//!   ┃ Pegar               Ctrl+V
//!   ┃ Eliminar            Supr
//!   ┃ ─────────────────────
//!   ┃ Seleccionar todo    Ctrl+A
//! ```
//!
//! Cada ítem se habilita o no según el estado real (sin selección →
//! Cortar/Copiar/Eliminar grises; sin historial → Deshacer gris; etc).
//!
//! Uso típico, en tres pasos por app:
//! 1. El campo emite la posición del click derecho — `View::on_right_click_at`
//!    → `Msg::AbrirMenuEdicion(x, y)`. El `update` guarda el ancla.
//! 2. `App::view_overlay` devuelve
//!    `Some(context_menu_view(edit_menu::edit_context_menu(...)))` cuando el
//!    ancla está presente.
//! 3. El pick produce `Msg::Edicion(EditAction)`; el `update` llama a
//!    [`apply`] con el `EditorState` del campo focuseado y el clipboard.

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_theme::Theme;
use llimphi_ui::{Key, KeyEvent, KeyState, Modifiers, NamedKey};
use llimphi_widget_context_menu::{ContextMenuItem, ContextMenuPalette, ContextMenuSpec};
use llimphi_widget_text_editor::{ApplyResult, Clipboard, EditorState};

/// Una acción de edición del menú estándar. Es `Copy` para que el
/// `on_pick` la capture sin clonar y la app la rebote en un `Msg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditAction {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    /// Borra la selección (Supr/Delete sin mover el resto).
    Delete,
    SelectAll,
}

/// Banderas que deciden qué ítems van habilitados. Derivalas del estado
/// del campo focuseado con [`EditFlags::from_editor`].
#[derive(Debug, Clone, Copy)]
pub struct EditFlags {
    /// Hay selección no-vacía → Cortar/Copiar/Eliminar habilitados.
    pub has_selection: bool,
    /// Hay algo que deshacer.
    pub can_undo: bool,
    /// Hay algo que rehacer.
    pub can_redo: bool,
    /// El clipboard tiene contenido pegable → Pegar habilitado. Si no se
    /// puede saber barato, pasá `true` (Pegar no-opea si está vacío).
    pub can_paste: bool,
    /// El buffer no está vacío → Seleccionar todo habilitado.
    pub has_text: bool,
    /// Campo enmascarado (password): Cortar/Copiar se deshabilitan para
    /// no filtrar el secreto al clipboard.
    pub masked: bool,
}

impl Default for EditFlags {
    fn default() -> Self {
        Self {
            has_selection: false,
            can_undo: false,
            can_redo: false,
            can_paste: true,
            has_text: false,
            masked: false,
        }
    }
}

impl EditFlags {
    /// Deriva las banderas del estado del editor. `can_paste` se deja en
    /// `true` (consultar el clipboard real requiere `&mut`; pegar vacío
    /// es no-op). `masked` lo decide el caller (el input lo sabe vía
    /// `TextInputState::is_masked`).
    pub fn from_editor(state: &EditorState, masked: bool) -> Self {
        Self {
            has_selection: state.has_selection(),
            can_undo: state.can_undo(),
            can_redo: state.can_redo(),
            can_paste: true,
            has_text: !state.is_empty(),
            masked,
        }
    }

    /// Igual que [`Self::from_editor`] pero fijando `can_paste`
    /// explícitamente (cuando el caller ya sabe si el clipboard tiene
    /// algo, p.ej. consultándolo una vez por frame).
    pub fn from_editor_with_paste(state: &EditorState, masked: bool, can_paste: bool) -> Self {
        Self {
            can_paste,
            ..Self::from_editor(state, masked)
        }
    }
}

/// Los ítems del menú + la acción de cada uno alineadas por índice. Las
/// filas separador llevan una acción de relleno (`SelectAll`) que **nunca
/// se dispara**: el `context-menu` no engancha `on_click` en separadores
/// ni en ítems deshabilitados, así que `on_pick(i)` sólo recibe índices
/// de ítems-acción habilitados. Mantener un `EditAction` por fila (en vez
/// de `Option`) permite que el closure de `on_pick` capture sólo `Arc`s
/// y no un `Msg` crudo — clave para satisfacer `Send + Sync` sin exigirle
/// esos bounds al `Msg` de la app.
fn entries(flags: EditFlags) -> (Vec<ContextMenuItem>, Vec<EditAction>) {
    let mut items: Vec<ContextMenuItem> = Vec::with_capacity(9);
    let mut actions: Vec<EditAction> = Vec::with_capacity(9);
    const FILL: EditAction = EditAction::SelectAll;

    let mut push = |item: ContextMenuItem, action: EditAction| {
        items.push(item);
        actions.push(action);
    };

    let undo = ContextMenuItem::action("Deshacer").icon("\u{21A9}").with_shortcut("Ctrl+Z");
    push(
        if flags.can_undo { undo } else { undo.disabled() },
        EditAction::Undo,
    );
    let redo = ContextMenuItem::action("Rehacer").icon("\u{21AA}").with_shortcut("Ctrl+Y");
    push(
        if flags.can_redo { redo } else { redo.disabled() },
        EditAction::Redo,
    );

    push(ContextMenuItem::separator(), FILL);

    let can_copy = flags.has_selection && !flags.masked;
    let cut = ContextMenuItem::action("Cortar").icon("\u{2702}").with_shortcut("Ctrl+X");
    push(if can_copy { cut } else { cut.disabled() }, EditAction::Cut);
    let copy = ContextMenuItem::action("Copiar").icon("\u{29C9}").with_shortcut("Ctrl+C");
    push(if can_copy { copy } else { copy.disabled() }, EditAction::Copy);
    let paste = ContextMenuItem::action("Pegar").icon("\u{2398}").with_shortcut("Ctrl+V");
    push(
        if flags.can_paste { paste } else { paste.disabled() },
        EditAction::Paste,
    );
    let del = ContextMenuItem::action("Eliminar")
        .icon("\u{2717}")
        .with_shortcut("Supr")
        .destructive();
    push(
        if flags.has_selection { del } else { del.disabled() },
        EditAction::Delete,
    );

    push(ContextMenuItem::separator(), FILL);

    let sel = ContextMenuItem::action("Seleccionar todo").icon("\u{2750}").with_shortcut("Ctrl+A");
    push(
        if flags.has_text { sel } else { sel.disabled() },
        EditAction::SelectAll,
    );

    (items, actions)
}

/// Sólo los ítems (para componer un menú custom que incluya el bloque de
/// edición seguido de acciones propias de la app).
pub fn edit_menu_items(flags: EditFlags) -> Vec<ContextMenuItem> {
    entries(flags).0
}

/// Arma el [`ContextMenuSpec`] del menú de edición listo para
/// `context_menu_view`. `on_action` rebota cada pick en un `Msg` de la
/// app; `on_dismiss` cierra al click-fuera o Esc.
pub fn edit_context_menu<Msg, F>(
    anchor: (f32, f32),
    viewport: (f32, f32),
    theme: &Theme,
    flags: EditFlags,
    on_action: F,
    on_dismiss: Msg,
) -> ContextMenuSpec<Msg>
where
    Msg: Clone + 'static,
    F: Fn(EditAction) -> Msg + Send + Sync + 'static,
{
    let (items, actions) = entries(flags);
    let actions = Arc::new(actions);
    let on_action = Arc::new(on_action);
    let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(move |i: usize| {
        // `i` siempre cae en un ítem-acción habilitado (los separadores y
        // deshabilitados no enganchan click). El `SelectAll` de relleno de
        // los separadores nunca se alcanza.
        let a = actions.get(i).copied().unwrap_or(EditAction::SelectAll);
        (on_action)(a)
    });

    ContextMenuSpec {
        anchor,
        viewport,
        header: Some("Edición".to_string()),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss,
        palette: ContextMenuPalette::from_theme(theme),
    }
}

/// Aplica una [`EditAction`] al `EditorState`. Reutiliza
/// `apply_key_with_clipboard` (sintetizando la tecla equivalente) para
/// heredar exactamente el mismo comportamiento — incluido el bookkeeping
/// de parseo incremental — que el atajo de teclado. Devuelve el
/// [`ApplyResult`] para que el caller decida si persistir el cambio.
pub fn apply(state: &mut EditorState, action: EditAction, clipboard: &mut dyn Clipboard) -> ApplyResult {
    match action {
        EditAction::SelectAll => {
            state.select_all();
            ApplyResult::CursorMoved
        }
        EditAction::Undo => state.apply_key_with_clipboard(&ctrl_char("z"), clipboard),
        EditAction::Redo => state.apply_key_with_clipboard(&ctrl_char("y"), clipboard),
        EditAction::Cut => state.apply_key_with_clipboard(&ctrl_char("x"), clipboard),
        EditAction::Copy => state.apply_key_with_clipboard(&ctrl_char("c"), clipboard),
        EditAction::Paste => state.apply_key_with_clipboard(&ctrl_char("v"), clipboard),
        EditAction::Delete => state.apply_key_with_clipboard(&named(NamedKey::Delete), clipboard),
    }
}

fn ctrl_char(s: &str) -> KeyEvent {
    KeyEvent {
        key: Key::Character(s.into()),
        state: KeyState::Pressed,
        text: Some(s.to_string()),
        modifiers: Modifiers {
            ctrl: true,
            ..Modifiers::default()
        },
        repeat: false,
    }
}

fn named(k: NamedKey) -> KeyEvent {
    KeyEvent {
        key: Key::Named(k),
        state: KeyState::Pressed,
        text: None,
        modifiers: Modifiers::default(),
        repeat: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_widget_text_editor::MemClipboard;

    fn lleno() -> EditorState {
        let mut s = EditorState::new();
        s.set_text("hola mundo");
        s
    }

    #[test]
    fn select_all_y_copy_llevan_todo_al_clipboard() {
        let mut s = lleno();
        let r = apply(&mut s, EditAction::SelectAll, &mut MemClipboard::new());
        assert_eq!(r, ApplyResult::CursorMoved);
        assert!(s.has_selection());
        let mut clip = MemClipboard::new();
        apply(&mut s, EditAction::Copy, &mut clip);
        assert_eq!(clip.get().as_deref(), Some("hola mundo"));
    }

    #[test]
    fn cut_borra_y_copia() {
        let mut s = lleno();
        s.select_all();
        let mut clip = MemClipboard::new();
        let r = apply(&mut s, EditAction::Cut, &mut clip);
        assert_eq!(r, ApplyResult::Changed);
        assert!(s.is_empty());
        assert_eq!(clip.get().as_deref(), Some("hola mundo"));
    }

    #[test]
    fn paste_inserta_del_clipboard() {
        let mut s = EditorState::new();
        let mut clip = MemClipboard::with("XYZ");
        apply(&mut s, EditAction::Paste, &mut clip);
        assert_eq!(s.text(), "XYZ");
    }

    #[test]
    fn flags_sin_seleccion_deshabilitan_copiar() {
        let s = lleno();
        let flags = EditFlags::from_editor(&s, false);
        assert!(!flags.has_selection);
        let items = edit_menu_items(flags);
        // "Cortar" es el primer ítem tras el separador (índice 3).
        assert!(!items[3].enabled, "Cortar debería estar deshabilitado sin selección");
    }

    #[test]
    fn masked_deshabilita_copiar_aun_con_seleccion() {
        let mut s = lleno();
        s.select_all();
        let flags = EditFlags::from_editor(&s, true);
        let items = edit_menu_items(flags);
        assert!(!items[4].enabled, "Copiar debería estar gris en campo enmascarado");
    }
}
