//! `nahual-shell-llimphi` — MVP del shell nahual sobre Llimphi.
//!
//! Composición mínima: barra superior con la ruta + split draggable con
//! lista de entradas a la izquierda y `nahual-text-viewer-llimphi` a la
//! derecha. Foco en validar la composición Llimphi y consumir widgets
//! reusables, no en paridad con el shell GPUI.
//!
//! Lo que **sí** hace este MVP:
//! - Navegación con teclado: ↑/↓ y rueda mueven la selección/scroll;
//!   Enter entra a un directorio o abre un archivo; Backspace sube al
//!   padre.
//! - Click en una fila: selecciona; si es archivo, lo previsualiza.
//! - Preview de archivos texto pequeños (delegado al crate
//!   `nahual-text-viewer-llimphi`, ≤ 256 KB, UTF-8 sin null bytes).
//! - Recorte real de paneles (`clip = true`): contenido virtualizado o
//!   texto largo no sangra a vecinos.
//! - Splitter draggable (vía `llimphi-widget-splitter`).
//!
//! Lo que **todavía** no:
//! - `layout.json` / `Persister` / hot-reload.
//! - Otros containers (Tabs, Tiled) y otros viewers (Image, Database).
//! - AppBus: el viewer recibe el path directo desde el modelo. Cuando
//!   tengamos un bus, el shell publica `EntitySelected` y los viewers
//!   se suscriben.

use std::cmp::min;
use std::fs;
use std::path::{Path, PathBuf};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_theme::Theme;
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use nahual_text_viewer_llimphi::{
    load_preview, text_viewer_view, PreviewState, TextViewerPalette,
    DEFAULT_PREVIEW_BYTES_MAX,
};

fn main() {
    llimphi_ui::run::<Shell>();
}

// ---------------------------------------------------------------------
// Modelo
// ---------------------------------------------------------------------

const ROW_HEIGHT: f32 = 22.0;
/// Cuántas filas mostramos a la vez en el panel de la lista. Calibrado
/// para un viewport de 1200×800 (alto del panel ≈ 760, header ≈ 24).
const VISIBLE_ROWS: usize = 32;
/// "Líneas" de la rueda que equivalen a una fila. Touchpads suelen mandar
/// fracciones; sumamos hasta tener ±1 fila para mover.
const WHEEL_LINES_PER_ROW: f32 = 1.0;

#[derive(Clone)]
struct Entry {
    name: String,
    is_dir: bool,
}

struct Model {
    cwd: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
    /// Índice del primer entry visible en el panel. Se ajusta al mover
    /// la selección para mantenerla dentro de la ventana.
    visible_offset: usize,
    /// Acumulador fraccional de la rueda — para touchpads que mandan
    /// deltas chicos. Cuando supera ±WHEEL_LINES_PER_ROW se materializa
    /// como un step de scroll y se vacía.
    wheel_accum: f32,
    /// Ancho del panel izquierdo en px. Lo muta el drag del splitter.
    list_width: f32,
    preview: PreviewState,
    /// Path del archivo que está previsualizado (si lo hay), para mostrar
    /// el nombre en el header del panel derecho.
    preview_of: Option<PathBuf>,
}

#[derive(Clone)]
enum Msg {
    Up,
    Down,
    OpenSelected,
    Parent,
    Select(usize),
    /// Scroll en filas — positivo abajo, negativo arriba.
    Scroll(i32),
    /// Drag del divisor — positivo = lista crece.
    ResizeList(f32),
}

// ---------------------------------------------------------------------
// App
// ---------------------------------------------------------------------

struct Shell;

impl App for Shell {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "nahual · shell"
    }

    fn initial_size() -> (u32, u32) {
        (1200, 800)
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let entries = scan_dir(&cwd);
        Model {
            cwd,
            entries,
            selected: 0,
            visible_offset: 0,
            wheel_accum: 0.0,
            list_width: 400.0,
            preview: PreviewState::Empty,
            preview_of: None,
        }
    }

    fn on_key(_model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        match &e.key {
            Key::Named(NamedKey::ArrowUp) => Some(Msg::Up),
            Key::Named(NamedKey::ArrowDown) => Some(Msg::Down),
            Key::Named(NamedKey::Enter) => Some(Msg::OpenSelected),
            Key::Named(NamedKey::Backspace) => Some(Msg::Parent),
            _ => None,
        }
    }

    fn on_wheel(
        model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Self::Msg> {
        // Acumulamos en `wheel_accum` desde `update`. Acá calculamos cuántas
        // filas mover según el delta de **este** evento sumado al acumulado.
        let total = model.wheel_accum + delta.y;
        let steps = (total / WHEEL_LINES_PER_ROW).trunc() as i32;
        if steps == 0 {
            // Sigue siendo sub-fila — emitimos un Scroll(0) para que el
            // update guarde el delta en el acumulador.
            return Some(Msg::Scroll(0));
        }
        Some(Msg::Scroll(steps))
    }

    fn update(model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Up => {
                if m.selected > 0 {
                    m.selected -= 1;
                    sync_offset(&mut m);
                    preview_selected(&mut m);
                }
            }
            Msg::Down => {
                if m.selected + 1 < m.entries.len() {
                    m.selected += 1;
                    sync_offset(&mut m);
                    preview_selected(&mut m);
                }
            }
            Msg::Select(idx) => {
                if idx < m.entries.len() {
                    m.selected = idx;
                    sync_offset(&mut m);
                    preview_selected(&mut m);
                }
            }
            Msg::OpenSelected => {
                let Some(entry) = m.entries.get(m.selected).cloned() else {
                    return m;
                };
                if entry.is_dir {
                    let new_cwd = m.cwd.join(&entry.name);
                    if let Ok(canonical) = fs::canonicalize(&new_cwd) {
                        m.cwd = canonical;
                    } else {
                        m.cwd = new_cwd;
                    }
                    m.entries = scan_dir(&m.cwd);
                    m.selected = 0;
                    m.visible_offset = 0;
                    m.preview = PreviewState::Empty;
                    m.preview_of = None;
                }
            }
            Msg::Parent => {
                let Some(parent) = m.cwd.parent().map(Path::to_path_buf) else {
                    return m;
                };
                let prev_name = m
                    .cwd
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string());
                m.cwd = parent;
                m.entries = scan_dir(&m.cwd);
                m.selected = prev_name
                    .and_then(|n| m.entries.iter().position(|e| e.name == n))
                    .unwrap_or(0);
                m.visible_offset = 0;
                sync_offset(&mut m);
                m.preview = PreviewState::Empty;
                m.preview_of = None;
                preview_selected(&mut m);
            }
            Msg::ResizeList(dx) => {
                m.list_width = (m.list_width + dx).clamp(220.0, 900.0);
            }
            Msg::Scroll(steps) => {
                // Convertimos pasos en un nuevo offset; el acumulador se
                // ajusta para quedarse con la fracción residual.
                m.wheel_accum -= steps as f32 * WHEEL_LINES_PER_ROW;
                if steps != 0 {
                    let len = m.entries.len();
                    let max_offset = len.saturating_sub(VISIBLE_ROWS);
                    if steps > 0 {
                        m.visible_offset = min(m.visible_offset + steps as usize, max_offset);
                    } else {
                        let drop = (-steps) as usize;
                        m.visible_offset = m.visible_offset.saturating_sub(drop);
                    }
                }
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = Theme::dark();
        let splitter_palette = SplitterPalette::from_theme(&theme);
        let viewer_palette = TextViewerPalette::from_theme(&theme);
        let header = header_bar(model, &theme);
        let list_pane = build_list_pane(model, &theme);
        let viewer_pane = text_viewer_view::<Msg>(
            &model.preview,
            model.preview_of.as_deref(),
            &viewer_palette,
        );

        let body = splitter_two(
            Direction::Row,
            list_pane,
            PaneSize::Fixed(model.list_width),
            viewer_pane,
            PaneSize::Flex,
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizeList(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, body])
    }
}

// ---------------------------------------------------------------------
// Vistas
// ---------------------------------------------------------------------

fn header_bar(model: &Model, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(
        format!("nahual · {}", model.cwd.display()),
        12.0,
        theme.fg_text,
        Alignment::Start,
    )
}

fn build_list_pane(model: &Model, theme: &Theme) -> View<Msg> {
    let start = model.visible_offset;
    let end = min(model.entries.len(), start + VISIBLE_ROWS);
    let rows: Vec<ListRow<Msg>> = (start..end)
        .map(|idx| {
            let entry = &model.entries[idx];
            let icon = if entry.is_dir { "▸ " } else { "  " };
            let label = if entry.is_dir {
                format!("{}{}/", icon, entry.name)
            } else {
                format!("{}{}", icon, entry.name)
            };
            ListRow {
                label,
                selected: idx == model.selected,
                on_click: Msg::Select(idx),
            }
        })
        .collect();

    let caption = format!(
        "{} entradas · ↑↓ navega · Enter entra · ⌫ sube",
        model.entries.len()
    );
    let truncated_hint = if model.entries.len() > end {
        Some(format!(
            "… y {} más (rueda o ↓ para ver más)",
            model.entries.len() - end
        ))
    } else {
        None
    };

    let list = list_view(ListSpec {
        rows,
        total: model.entries.len(),
        caption: Some(caption),
        truncated_hint,
        row_height: ROW_HEIGHT,
        palette: ListPalette::from_theme(theme),
    });

    // El splitter envuelve esto en un pane con el ancho del Model.
    list
}

// ---------------------------------------------------------------------
// Lógica de directorio / preview
// ---------------------------------------------------------------------

fn scan_dir(path: &Path) -> Vec<Entry> {
    let Ok(it) = fs::read_dir(path) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = it
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Entry { name, is_dir }
        })
        .collect();
    // Directorios primero, después por nombre (case-insensitive).
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    entries
}

/// Mantiene `visible_offset` para que `selected` siempre quede en la
/// ventana visible. Se llama después de cualquier cambio de selección.
fn sync_offset(m: &mut Model) {
    if m.selected < m.visible_offset {
        m.visible_offset = m.selected;
    }
    let bottom = m.visible_offset + VISIBLE_ROWS;
    if m.selected >= bottom {
        m.visible_offset = m.selected + 1 - VISIBLE_ROWS;
    }
}

fn preview_selected(m: &mut Model) {
    let Some(entry) = m.entries.get(m.selected) else {
        m.preview = PreviewState::Empty;
        m.preview_of = None;
        return;
    };
    if entry.is_dir {
        m.preview = PreviewState::Empty;
        m.preview_of = None;
        return;
    }
    let path = m.cwd.join(&entry.name);
    m.preview = load_preview(&path, DEFAULT_PREVIEW_BYTES_MAX);
    m.preview_of = Some(path);
}
