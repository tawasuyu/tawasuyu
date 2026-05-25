//! `nahual_database_explorer` — explorer de SQLite.
//!
//! Mismo patrón que `nahual_file_explorer` pero con `SqliteProvider`. La
//! UX es idéntica (TreeView con lazy load por chevron); cambia solo el
//! origen de los datos: filas de una tabla `items(id, parent_id, name,
//! display_type, content)` en lugar del filesystem.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::{
    Context, Entity, EventEmitter, IntoElement, Render, SharedString, Window, div, prelude::*,
    px,
};

use nahual_core::{DataProvider, DisplayType, EntityNode};
use nahual_provider_sqlite::SqliteDataProvider;
use nahual_theme::Theme;
use nahual_widget_tree::{RowId, RowKind, TreeEvent, TreeRow, TreeView};

#[derive(Clone, Debug)]
#[allow(dead_code)] // Consumido por el AppBus en Fase 4+.
pub enum DatabaseExplorerEvent {
    EntitySelected { id: String },
    EntityOpened { id: String },
}

pub struct DatabaseExplorer {
    tree_view: Entity<TreeView>,
    provider: Arc<SqliteDataProvider>,
    db_path: String,

    expanded: HashSet<String>,
    children: HashMap<String, Vec<EntityNode>>,
    pending: HashSet<String>,
    /// Mensaje de error si la DB no abrió. Se muestra en el header.
    open_error: Option<String>,
}

const ROOT_KEY: &str = "__db_root__";

impl EventEmitter<DatabaseExplorerEvent> for DatabaseExplorer {}

impl DatabaseExplorer {
    /// `db_path` es la ruta al .sqlite. Si no existe se crea con la tabla
    /// `items` y un seed mínimo (ver `SqliteDataProvider::new`).
    pub fn new(db_path: String, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();

        let tree_view = cx.new(|cx| TreeView::new("db-explorer-tree", cx));
        cx.subscribe(&tree_view, |this: &mut DatabaseExplorer, _, ev, cx| {
            this.on_tree_event(ev, cx);
        })
        .detach();

        let (provider, open_error) = match SqliteDataProvider::new(&db_path) {
            Ok(p) => (Some(Arc::new(p)), None),
            Err(e) => (None, Some(e)),
        };

        let mut expanded = HashSet::new();
        expanded.insert(ROOT_KEY.to_string());

        let mut me = Self {
            tree_view,
            // Usamos un dummy provider si la DB no abrió. La UI mostrará el
            // error en el header; cualquier load_children retornará vacío.
            provider: provider.unwrap_or_else(|| {
                Arc::new(SqliteDataProvider::new(":memory:").expect("memory db"))
            }),
            db_path,
            expanded,
            children: HashMap::new(),
            pending: HashSet::new(),
            open_error,
        };
        // Cargar el root (parent_id NULL en SQLite, mapped to None acá).
        me.load_children(ROOT_KEY.to_string(), cx);
        me
    }

    pub fn db_path(&self) -> &str {
        &self.db_path
    }

    fn load_children(&mut self, parent_key: String, cx: &mut Context<Self>) {
        if self.pending.contains(&parent_key) || self.children.contains_key(&parent_key) {
            return;
        }
        self.pending.insert(parent_key.clone());

        let provider = self.provider.clone();
        let parent_for_task = parent_key.clone();
        cx.spawn(async move |this, cx| {
            // ROOT_KEY → None; cualquier otro → Some(actual id).
            let arg: Option<String> = if parent_for_task == ROOT_KEY {
                None
            } else {
                Some(parent_for_task.clone())
            };
            let result = provider.list_children(arg.as_deref()).await;
            let _ = this.update(cx, |this, cx| {
                this.on_children_loaded(parent_for_task, result, cx);
            });
        })
        .detach();
    }

    fn on_children_loaded(
        &mut self,
        parent_key: String,
        result: Result<Vec<EntityNode>, String>,
        cx: &mut Context<Self>,
    ) {
        self.pending.remove(&parent_key);
        let mut entries = result.unwrap_or_default();
        sort_entries(&mut entries);
        self.children.insert(parent_key, entries);
        self.push_rows(cx);
    }

    fn push_rows(&self, cx: &mut Context<Self>) {
        let mut rows = Vec::new();
        // Una row "raíz virtual" para que el árbol tenga un anchor visible.
        rows.push(TreeRow {
            id: RowId::new(ROOT_KEY),
            label: format!("(db) {}", self.db_path),
            depth: 0,
            kind: RowKind::Branch,
            expanded: self.expanded.contains(ROOT_KEY),
            icon: Some("🗄️".to_string()),
        });
        if self.expanded.contains(ROOT_KEY) {
            self.append_children(ROOT_KEY, 1, &mut rows);
        }

        self.tree_view
            .update(&mut *cx, |tree, cx| tree.set_rows(rows, cx));
    }

    fn append_children(&self, parent: &str, depth: u32, out: &mut Vec<TreeRow>) {
        let Some(children) = self.children.get(parent) else { return };
        for entry in children {
            let kind = match entry.display_type {
                DisplayType::Folder => RowKind::Branch,
                _ => RowKind::Leaf,
            };
            let icon = match entry.display_type {
                DisplayType::Folder => "📂",
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

    fn on_tree_event(&mut self, event: &TreeEvent, cx: &mut Context<Self>) {
        match event {
            TreeEvent::ChevronToggled(id) => {
                let key = id.as_str().to_string();
                if !self.expanded.remove(&key) {
                    self.expanded.insert(key.clone());
                    self.load_children(key, cx);
                }
                self.push_rows(cx);
            }
            TreeEvent::RowClicked(id) => {
                let key = id.as_str();
                if key == ROOT_KEY {
                    return;
                }
                if let Some(entry) = self.find_entry(key) {
                    if !matches!(entry.display_type, DisplayType::Folder) {
                        cx.emit(DatabaseExplorerEvent::EntitySelected {
                            id: key.to_string(),
                        });
                    }
                }
            }
            TreeEvent::RowDoubleClicked(id) => {
                let key = id.as_str();
                if key == ROOT_KEY {
                    return;
                }
                if let Some(entry) = self.find_entry(key) {
                    if !matches!(entry.display_type, DisplayType::Folder) {
                        cx.emit(DatabaseExplorerEvent::EntityOpened {
                            id: key.to_string(),
                        });
                    }
                }
            }
            TreeEvent::ContextMenuRequested { .. } | TreeEvent::ActiveChanged(_) => {}
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
}

impl Render for DatabaseExplorer {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let pending_count = self.pending.len();

        div()
            .size_full()
            .bg(theme.bg_panel.clone())
            .flex()
            .flex_col()
            .child(
                div()
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
                            .child("🗄️"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.fg_text)
                            .child(SharedString::from(self.db_path.clone())),
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
                    ),
            )
            .child(if let Some(err) = self.open_error.clone() {
                // Si la DB no abrió, mostramos el error y no pintamos el
                // árbol vacío — sería confuso.
                div()
                    .p(px(12.0))
                    .text_size(px(11.0))
                    .text_color(theme.accent_strong)
                    .child(SharedString::from(format!("error abriendo DB: {}", err)))
            } else {
                div().flex_grow().min_h(px(0.0)).child(self.tree_view.clone())
            })
    }
}

fn sort_entries(entries: &mut Vec<EntityNode>) {
    entries.sort_by(|a, b| {
        let a_dir = matches!(a.display_type, DisplayType::Folder);
        let b_dir = matches!(b.display_type, DisplayType::Folder);
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}
