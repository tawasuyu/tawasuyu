//! `nahual_file_explorer` — explorer de filesystem con menú contextual.
//!
//! Composición canónica del patrón "explorer = TreeView + provider":
//!
//! ```text
//!   FileExplorer
//!     ├── TreeView (widgets/tree, agnóstico)
//!     └── FsProvider (libs/providers/fs, async + tokio::io)
//! ```
//!
//! Estado:
//! - `expanded`: set de paths cuyo chevron está abierto.
//! - `children`: cache de hijos por path parent (cargado lazy via
//!   `cx.spawn` + provider).
//! - `pending`: set de loads en vuelo (anti re-trigger).
//! - `menu`: estado del menú contextual flotante (Fase 4.5).
//!
//! Menú contextual (Fase 4.5):
//! Right-click sobre una fila o el área vacía del árbol abre un menú con
//! las acciones FS habituales. Las mutaciones se ejecutan sincrónicamente
//! contra `std::fs` y luego se invalida el cache del directorio afectado
//! para forzar reload del provider.
//!
//! Acciones soportadas:
//! - **Open** (solo file): emite `FileExplorerEvent::FileOpened` (que el
//!   forwarder de la Shell traduce a `AppEvent::EntityOpened` al bus).
//! - **Copy path**: copia el path absoluto al clipboard.
//! - **New file**: crea un archivo vacío con nombre auto-generado
//!   (`new_file_NN.txt`) en el directorio elegido — sin necesitar text
//!   input por ahora; renombrar viene cuando sumemos un TextInput
//!   widget.
//! - **New folder**: crea un directorio con nombre auto (`new_folder_NN`).
//! - **Delete**: confirmación vía `window.prompt` (dialog del platform);
//!   si el usuario confirma, `remove_file` o `remove_dir_all` según
//!   corresponda.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{
    ClickEvent, ClipboardItem, Context, Entity, EventEmitter, IntoElement, Pixels, Point,
    PromptLevel, Render, SharedString, Window, div, prelude::*, px,
};

use nahual_core::{DataProvider, DisplayType, EntityNode};
use nahual_provider_fs::FileDataProvider;
use nahual_theme::Theme;
use nahual_widget_text_input::{TextInput, TextInputEvent};
use nahual_widget_tree::{RowId, RowKind, TreeEvent, TreeRow, TreeView};

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum FileExplorerEvent {
    FileSelected { path: String },
    FileOpened { path: String },
    RootChanged { path: String },
}

#[derive(Clone, Debug)]
struct MenuState {
    /// `None` ⇒ menú "fondo" (área vacía del tree). `Some` ⇒ sobre una
    /// fila concreta.
    target: Option<MenuTarget>,
    /// Posición absoluta donde se hizo el right-click. Para el overlay
    /// usamos coords window-relative (gpui las maneja como px).
    position: Point<Pixels>,
}

#[derive(Clone, Debug)]
struct MenuTarget {
    id: String,
    is_folder: bool,
}

pub struct FileExplorer {
    tree_view: Entity<TreeView>,
    provider: Arc<FileDataProvider>,

    root: String,
    expanded: HashSet<String>,
    children: HashMap<String, Vec<EntityNode>>,
    pending: HashSet<String>,
    menu: Option<MenuState>,
    /// Modal de rename activo. `None` ⇒ no hay modal.
    rename: Option<RenameState>,
}

/// Estado del modal de rename. El TextInput vive como sub-entity para
/// que pueda recibir focus + key events. El target_path lo lleva la
/// closure de subscripción (no lo necesitamos en `self` aparte).
#[derive(Clone)]
struct RenameState {
    original_name: String,
    input: Entity<TextInput>,
}

impl EventEmitter<FileExplorerEvent> for FileExplorer {}

impl FileExplorer {
    pub fn new(root: String, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();

        let tree_view = cx.new(|cx| TreeView::new("file-explorer-tree", cx));
        cx.subscribe(&tree_view, |this: &mut FileExplorer, _, ev, cx| {
            this.on_tree_event(ev, cx);
        })
        .detach();

        let mut expanded = HashSet::new();
        expanded.insert(root.clone());

        let mut me = Self {
            tree_view,
            provider: Arc::new(FileDataProvider::new()),
            root: root.clone(),
            expanded,
            children: HashMap::new(),
            pending: HashSet::new(),
            menu: None,
            rename: None,
        };
        me.load_children(root, cx);
        me
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    #[allow(dead_code)]
    pub fn set_root(&mut self, path: String, cx: &mut Context<Self>) {
        if path != self.root {
            self.root = path.clone();
            self.expanded.insert(path.clone());
            self.load_children(path.clone(), cx);
            self.push_rows(cx);
            cx.emit(FileExplorerEvent::RootChanged { path });
        }
    }

    // ----- load + cache -----

    fn load_children(&mut self, parent: String, cx: &mut Context<Self>) {
        if self.pending.contains(&parent) || self.children.contains_key(&parent) {
            return;
        }
        self.pending.insert(parent.clone());

        let provider = self.provider.clone();
        let parent_for_task = parent.clone();
        cx.spawn(async move |this, cx| {
            let result = provider.list_children(Some(&parent_for_task)).await;
            let _ = this.update(cx, |this, cx| {
                this.on_children_loaded(parent_for_task, result, cx);
            });
        })
        .detach();
    }

    /// Versión "force": invalida el cache antes de re-pedir. Se usa tras
    /// una mutación FS (new/delete) para que la UI refleje el cambio.
    fn refresh_dir(&mut self, parent: String, cx: &mut Context<Self>) {
        self.children.remove(&parent);
        self.pending.remove(&parent);
        self.load_children(parent, cx);
    }

    fn on_children_loaded(
        &mut self,
        parent: String,
        result: Result<Vec<EntityNode>, String>,
        cx: &mut Context<Self>,
    ) {
        self.pending.remove(&parent);
        match result {
            Ok(mut children) => {
                sort_entries(&mut children);
                self.children.insert(parent, children);
                self.push_rows(cx);
            }
            Err(_) => {
                self.children.insert(parent, Vec::new());
                self.push_rows(cx);
            }
        }
    }

    fn push_rows(&self, cx: &mut Context<Self>) {
        let mut rows = Vec::new();
        rows.push(TreeRow {
            id: RowId::new(self.root.clone()),
            label: self.root.clone(),
            depth: 0,
            kind: RowKind::Branch,
            expanded: self.expanded.contains(&self.root),
            icon: Some("📂".to_string()),
        });
        if self.expanded.contains(&self.root) {
            self.append_children(&self.root, 1, &mut rows);
        }

        self.tree_view
            .update(cx, |tree, cx| tree.set_rows(rows, cx));
    }

    fn append_children(&self, parent: &str, depth: u32, out: &mut Vec<TreeRow>) {
        let Some(children) = self.children.get(parent) else { return };
        for entry in children {
            let kind = match entry.display_type {
                DisplayType::Folder => RowKind::Branch,
                _ => RowKind::Leaf,
            };
            let icon = match entry.display_type {
                DisplayType::Folder => "📁",
                DisplayType::File => "📄",
                DisplayType::Stream => "📡",
            };
            let is_expanded = self.expanded.contains(&entry.id);
            out.push(TreeRow {
                id: RowId::new(entry.id.clone()),
                label: entry.name.clone(),
                depth,
                kind,
                expanded: is_expanded,
                icon: Some(icon.to_string()),
            });
            if is_expanded {
                self.append_children(&entry.id, depth + 1, out);
            }
        }
    }

    // ----- TreeView events -----

    fn on_tree_event(&mut self, event: &TreeEvent, cx: &mut Context<Self>) {
        match event {
            TreeEvent::ChevronToggled(id) => {
                let path = id.as_str().to_string();
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path.clone());
                    self.load_children(path, cx);
                }
                self.push_rows(cx);
            }
            TreeEvent::RowClicked(id) => {
                let path = id.as_str();
                if let Some(entry) = self.find_entry(path) {
                    if !matches!(entry.display_type, DisplayType::Folder) {
                        cx.emit(FileExplorerEvent::FileSelected {
                            path: path.to_string(),
                        });
                    }
                }
                // Click primario en cualquier lado también cierra el
                // menú contextual si estaba abierto.
                if self.menu.is_some() {
                    self.menu = None;
                    cx.notify();
                }
            }
            TreeEvent::RowDoubleClicked(id) => {
                let path = id.as_str();
                if let Some(entry) = self.find_entry(path) {
                    if !matches!(entry.display_type, DisplayType::Folder) {
                        cx.emit(FileExplorerEvent::FileOpened {
                            path: path.to_string(),
                        });
                    }
                }
            }
            TreeEvent::ContextMenuRequested { id, position } => {
                self.open_menu(id.as_ref().map(|i| i.as_str().to_string()), *position, cx);
            }
            TreeEvent::ActiveChanged(_) => {}
        }
    }

    fn find_entry(&self, id: &str) -> Option<&EntityNode> {
        for entries in self.children.values() {
            if let Some(e) = entries.iter().find(|e| e.id == id) {
                return Some(e);
            }
        }
        None
    }

    fn is_folder_path(&self, id: &str) -> bool {
        self.find_entry(id)
            .map(|e| matches!(e.display_type, DisplayType::Folder))
            .unwrap_or_else(|| std::path::Path::new(id).is_dir())
    }

    /// Resuelve el directorio donde crear un nuevo archivo/carpeta según
    /// el target del menú: si target es folder → adentro; si target es
    /// file → su parent; si target es None (fondo) → root.
    fn parent_for_new(&self, target: &Option<MenuTarget>) -> String {
        match target {
            None => self.root.clone(),
            Some(t) if t.is_folder => t.id.clone(),
            Some(t) => std::path::Path::new(&t.id)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.root.clone()),
        }
    }

    // ----- menú: open/close -----

    fn open_menu(
        &mut self,
        id: Option<String>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let target = id.map(|id| {
            let is_folder = self.is_folder_path(&id);
            MenuTarget { id, is_folder }
        });
        self.menu = Some(MenuState { target, position });
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    // ----- acciones del menú -----

    fn action_open(&mut self, target: MenuTarget, cx: &mut Context<Self>) {
        if !target.is_folder {
            cx.emit(FileExplorerEvent::FileOpened { path: target.id });
        }
        self.close_menu(cx);
    }

    fn action_copy_path(
        &mut self,
        target: MenuTarget,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(target.id));
        self.close_menu(cx);
    }

    fn action_new_file(
        &mut self,
        target: Option<MenuTarget>,
        cx: &mut Context<Self>,
    ) {
        let parent = self.parent_for_new(&target);
        if let Some(name) = next_available_name(&parent, "new_file_", ".txt", false) {
            let full = std::path::Path::new(&parent).join(&name);
            if let Err(e) = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&full)
            {
                eprintln!("[FileExplorer] new file {:?}: {}", full, e);
            }
        }
        self.refresh_dir(parent, cx);
        self.close_menu(cx);
    }

    fn action_new_folder(
        &mut self,
        target: Option<MenuTarget>,
        cx: &mut Context<Self>,
    ) {
        let parent = self.parent_for_new(&target);
        if let Some(name) = next_available_name(&parent, "new_folder_", "", true) {
            let full = std::path::Path::new(&parent).join(&name);
            if let Err(e) = std::fs::create_dir(&full) {
                eprintln!("[FileExplorer] new folder {:?}: {}", full, e);
            }
        }
        self.refresh_dir(parent, cx);
        self.close_menu(cx);
    }

    fn action_rename(
        &mut self,
        target: MenuTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Nombre actual = file_name del path. Si por alguna razón no
        // tiene file_name (path raíz), usamos el path completo.
        let original_name = std::path::Path::new(&target.id)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| target.id.clone());

        let initial = original_name.clone();
        let input = cx.new(|cx| TextInput::new(initial, cx));

        // Subscribirse a los eventos del input para confirmar/cancelar.
        cx.subscribe(&input, {
            let target_path = target.id.clone();
            let original_name = original_name.clone();
            move |this: &mut FileExplorer, _, ev: &TextInputEvent, cx| match ev {
                TextInputEvent::Confirmed(new_name) => {
                    this.commit_rename(&target_path, &original_name, new_name.clone(), cx);
                }
                TextInputEvent::Cancelled => {
                    this.close_rename(cx);
                }
            }
        })
        .detach();

        // Pedir focus para que las próximas teclas vayan al input. Hay
        // que hacerlo después de que el render lo monte; lo más seguro
        // es delay un frame con cx.spawn + immediate await.
        input.update(cx, |i, _| i.request_focus(window));

        self.rename = Some(RenameState {
            original_name,
            input,
        });
        self.close_menu(cx);
    }

    fn close_rename(&mut self, cx: &mut Context<Self>) {
        if self.rename.take().is_some() {
            cx.notify();
        }
    }

    fn commit_rename(
        &mut self,
        target_path: &str,
        original_name: &str,
        new_name: String,
        cx: &mut Context<Self>,
    ) {
        let trimmed = new_name.trim();
        let parent_dir = std::path::Path::new(target_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from(self.root.clone()));

        if !trimmed.is_empty() && trimmed != original_name {
            let from = std::path::PathBuf::from(target_path);
            let to = parent_dir.join(trimmed);
            if let Err(e) = std::fs::rename(&from, &to) {
                eprintln!("[FileExplorer] rename {:?} → {:?}: {}", from, to, e);
            }
        }

        let parent_str = parent_dir.to_string_lossy().into_owned();
        self.refresh_dir(parent_str, cx);
        self.close_rename(cx);
    }

    fn action_delete(
        &mut self,
        target: MenuTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Prompt nativo del platform — devuelve un Receiver con el índice
        // del botón clickeado. Esperamos el resultado en un task spawn.
        let path = target.id.clone();
        let is_folder = target.is_folder;
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let parent_dir = std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.clone());

        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("¿Borrar \"{}\"?", name),
            None,
            &["Borrar", "Cancelar"],
            cx,
        );

        cx.spawn(async move |this, cx| {
            let Ok(idx) = answer.await else { return };
            // 0 = Borrar, 1 = Cancelar.
            if idx != 0 {
                return;
            }
            let res = if is_folder {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            if let Err(e) = res {
                eprintln!("[FileExplorer] delete {}: {}", path, e);
            }
            let _ = this.update(cx, |this, cx| {
                this.refresh_dir(parent_dir, cx);
            });
        })
        .detach();

        self.close_menu(cx);
    }
}

// ---- helpers FS ----

fn sort_entries(entries: &mut Vec<EntityNode>) {
    entries.sort_by(|a, b| {
        let a_dir = matches!(a.display_type, DisplayType::Folder);
        let b_dir = matches!(b.display_type, DisplayType::Folder);
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// Primer nombre `prefix{n}{suffix}` (n = 1..=999) que no exista en `dir`.
/// Para folder no agregamos sufijo. Devuelve None si los 999 nombres ya
/// estaban en uso (improbable).
fn next_available_name(
    dir: &str,
    prefix: &str,
    suffix: &str,
    is_folder: bool,
) -> Option<String> {
    for n in 1..=999u32 {
        let candidate = format!("{}{}{}", prefix, n, suffix);
        let full = std::path::Path::new(dir).join(&candidate);
        let exists = if is_folder {
            full.is_dir()
        } else {
            full.exists()
        };
        if !exists {
            return Some(candidate);
        }
    }
    None
}

// =====================================================================
// Render
// =====================================================================

const MENU_WIDTH: f32 = 200.0;

impl Render for FileExplorer {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let pending_count = self.pending.len();

        // Capa 1: header + tree.
        let header = div()
            .h(px(28.0))
            .px(px(8.0))
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme.fg_muted)
                    .child("📂"),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme.fg_text)
                    .child(SharedString::from(self.root.clone())),
            )
            .child(
                div()
                    .ml_auto()
                    .text_size(px(10.0))
                    .text_color(theme.fg_muted)
                    .child(SharedString::from(if pending_count > 0 {
                        format!("⏳ {}", pending_count)
                    } else {
                        String::new()
                    })),
            );

        let body = div().flex_grow().min_h(px(0.0)).child(self.tree_view.clone());

        // Capa 2 (overlay condicional): menú contextual.
        let menu_overlay = self.menu.clone().map(|menu| self.render_menu(&theme, menu, cx));
        // Capa 3 (overlay condicional): modal de rename.
        let rename_overlay = self.rename.clone().map(|st| self.render_rename(&theme, st));

        let mut root = div()
            .id("file-explorer-root")
            .size_full()
            .relative()
            .bg(theme.bg_panel.clone())
            .flex()
            .flex_col()
            .child(header)
            .child(body);

        if let Some(overlay) = menu_overlay {
            root = root.child(overlay);
        }
        if let Some(overlay) = rename_overlay {
            root = root.child(overlay);
        }

        root
    }
}

impl FileExplorer {
    fn render_menu(
        &self,
        theme: &Theme,
        menu: MenuState,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let on_entry = menu.target.is_some();
        let target = menu.target.clone();
        let is_folder = target.as_ref().map(|t| t.is_folder).unwrap_or(false);

        let mut items = div()
            .flex()
            .flex_col()
            .py(px(4.0))
            .min_w(px(MENU_WIDTH))
            .bg(theme.bg_panel_alt.clone())
            .border_1()
            .border_color(theme.border_strong)
            .rounded(px(6.0));

        // Open: solo file targets.
        if on_entry && !is_folder {
            let t = target.clone().unwrap();
            items = items.child(
                menu_item("fe-menu-open", "Abrir", theme).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.action_open(t.clone(), cx);
                    },
                )),
            );
        }

        // Copy path.
        if let Some(t) = target.clone() {
            let t_clone = t.clone();
            items = items.child(
                menu_item("fe-menu-copy", "Copiar ruta", theme).on_click(cx.listener(
                    move |this, _: &ClickEvent, w, cx| {
                        this.action_copy_path(t_clone.clone(), w, cx);
                    },
                )),
            );
        }

        // Rename — solo cuando hay target.
        if let Some(t) = target.clone() {
            items = items.child(
                menu_item("fe-menu-rename", "Renombrar…", theme).on_click(cx.listener(
                    move |this, _: &ClickEvent, w, cx| {
                        this.action_rename(t.clone(), w, cx);
                    },
                )),
            );
            items = items.child(separator(theme));
        }

        // New file.
        let new_target = target.clone();
        items = items.child(
            menu_item("fe-menu-newfile", "Nuevo archivo", theme).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.action_new_file(new_target.clone(), cx);
                },
            )),
        );

        // New folder.
        let new_target_folder = target.clone();
        items = items.child(
            menu_item("fe-menu-newfolder", "Nueva carpeta", theme).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.action_new_folder(new_target_folder.clone(), cx);
                },
            )),
        );

        // Delete: solo entries existentes.
        if let Some(t) = target.clone() {
            items = items.child(separator(theme));
            items = items.child(
                menu_item("fe-menu-delete", "Borrar", theme).on_click(cx.listener(
                    move |this, _: &ClickEvent, w, cx| {
                        this.action_delete(t.clone(), w, cx);
                    },
                )),
            );
        }

        // Wrapper absolute en la posición del click. Las coords del
        // ContextMenuRequested son window-coords absolutas; las pasamos
        // directo con .left/.top.
        div()
            .absolute()
            .left(menu.position.x)
            .top(menu.position.y)
            .child(items)
    }
}

impl FileExplorer {
    fn render_rename(&self, theme: &Theme, state: RenameState) -> impl IntoElement {
        // Backdrop oscuro semi-transparente que cubre todo el explorer +
        // caja centrada con el TextInput. Click sobre el backdrop NO
        // cierra (queremos forzar Enter/Escape para evitar pérdida
        // accidental de input).
        div()
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::hsla(0.0, 0.0, 0.0, 0.55))
            .child(
                div()
                    .min_w(px(360.0))
                    .p(px(16.0))
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .bg(theme.bg_panel_alt.clone())
                    .border_1()
                    .border_color(theme.border_strong)
                    .rounded(px(8.0))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(theme.fg_text)
                            .child(SharedString::from(format!(
                                "Renombrar \"{}\"",
                                state.original_name
                            ))),
                    )
                    .child(state.input.clone())
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(theme.fg_muted)
                            .child("Enter = confirmar — Escape = cancelar"),
                    ),
            )
    }
}

fn menu_item(
    id: &'static str,
    label: &'static str,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px(px(12.0))
        .py(px(6.0))
        .text_size(px(12.0))
        .text_color(theme.fg_text)
        .hover(|s| s.bg(theme.bg_row_hover))
        .child(label)
}

fn separator(theme: &Theme) -> gpui::Div {
    div()
        .my(px(3.0))
        .h(px(1.0))
        .w_full()
        .bg(theme.border)
}
