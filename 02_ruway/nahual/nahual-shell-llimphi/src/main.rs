//! `nahual-shell-llimphi` — MVP del shell nahual sobre Llimphi.
//!
//! Composición mínima: barra superior con la ruta + split horizontal con
//! lista de entradas a la izquierda y previsualización de texto a la
//! derecha. Foco en validar la composición Llimphi y consumir widgets
//! reusables (`llimphi-widget-list`), no en paridad con el shell GPUI.
//!
//! Lo que **sí** hace este MVP:
//! - Navegación con teclado: ↑/↓ y rueda mueven la selección/scroll;
//!   Enter entra a un directorio o abre un archivo; Backspace sube al
//!   padre.
//! - Click en una fila: selecciona; si es archivo, lo previsualiza.
//! - Preview de archivos texto pequeños (≤ 256 KB, sólo UTF-8 sin null
//!   bytes). El resto se etiqueta como "binario" o "muy grande".
//! - Recorte real de paneles (`clip = true`): contenido virtualizado o
//!   texto largo no sangra a vecinos.
//!
//! Lo que **todavía** no:
//! - `layout.json` / `Persister` / hot-reload.
//! - Otros containers (Tabs, Tiled) y otros viewers (Image, Database).
//! - Splitter draggable (necesita tracking de drag en llimphi-ui).

use std::cmp::min;
use std::fs;
use std::path::{Path, PathBuf};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};

fn main() {
    llimphi_ui::run::<Shell>();
}

// ---------------------------------------------------------------------
// Modelo
// ---------------------------------------------------------------------

const PREVIEW_BYTES_MAX: u64 = 256 * 1024;
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

enum Preview {
    Empty,
    Text(String),
    Binary,
    TooBig(u64),
    Error(String),
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
    preview: Preview,
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
            preview: Preview::Empty,
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
                    m.preview = Preview::Empty;
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
                m.preview = Preview::Empty;
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
        let palette = Palette::default();
        let splitter_palette = SplitterPalette {
            divider: Color::from_rgba8(34, 40, 54, 255),
            divider_hover: Color::from_rgba8(110, 140, 220, 255),
            thickness: 6.0,
        };
        let header = header_bar(model, &palette);
        let list_pane = build_list_pane(model, &palette);
        let viewer_pane = viewer_pane_view(model, &palette);

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
        .fill(palette.bg_app)
        .children(vec![header, body])
    }
}

// ---------------------------------------------------------------------
// Vistas
// ---------------------------------------------------------------------

fn header_bar(model: &Model, palette: &Palette) -> View<Msg> {
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
    .fill(palette.bg_panel)
    .text_aligned(
        format!("nahual · {}", model.cwd.display()),
        12.0,
        palette.fg_text,
        Alignment::Start,
    )
}

fn build_list_pane(model: &Model, palette: &Palette) -> View<Msg> {
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
        palette: ListPalette {
            bg_panel: palette.bg_panel,
            bg_selected: palette.bg_sel,
            fg_text: palette.fg_text,
            fg_muted: palette.fg_muted,
        },
    });

    // El splitter envuelve esto en un pane con el ancho del Model.
    list
}

fn viewer_pane_view(model: &Model, palette: &Palette) -> View<Msg> {
    let header_text = match &model.preview_of {
        Some(p) => p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        None => "(seleccioná un archivo)".to_string(),
    };

    let cap = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start);

    let (body_text, body_color) = match &model.preview {
        Preview::Empty => ("—".to_string(), palette.fg_muted),
        Preview::Text(s) => (s.clone(), palette.fg_text),
        Preview::Binary => (
            "(archivo binario — sin preview)".to_string(),
            palette.fg_muted,
        ),
        Preview::TooBig(n) => (
            format!("(archivo muy grande: {} bytes — sin preview)", n),
            palette.fg_muted,
        ),
        Preview::Error(e) => (format!("(error: {e})"), palette.fg_destructive),
    };

    let body = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(body_text, 12.0, body_color, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_app)
    .clip(true) // recorta texto largo al pane.
    .children(vec![cap, body])
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
        m.preview = Preview::Empty;
        m.preview_of = None;
        return;
    };
    if entry.is_dir {
        m.preview = Preview::Empty;
        m.preview_of = None;
        return;
    }
    let path = m.cwd.join(&entry.name);
    m.preview_of = Some(path.clone());
    match fs::metadata(&path) {
        Ok(meta) if meta.len() > PREVIEW_BYTES_MAX => {
            m.preview = Preview::TooBig(meta.len());
            return;
        }
        Err(e) => {
            m.preview = Preview::Error(e.to_string());
            return;
        }
        _ => {}
    }
    match fs::read(&path) {
        Ok(bytes) => {
            if bytes.contains(&0) {
                m.preview = Preview::Binary;
            } else {
                match String::from_utf8(bytes) {
                    Ok(s) => m.preview = Preview::Text(truncate_preview(&s)),
                    Err(_) => m.preview = Preview::Binary,
                }
            }
        }
        Err(e) => m.preview = Preview::Error(e.to_string()),
    }
}

/// Llimphi-text wrappea hasta `max_width`; con archivos largos parley
/// puede tardar. Recortamos las primeras N líneas para mantener el render
/// instantáneo aunque el archivo entre en el límite de PREVIEW_BYTES_MAX.
fn truncate_preview(s: &str) -> String {
    const MAX_LINES: usize = 200;
    const MAX_CHARS: usize = 8_000;
    let mut out = String::new();
    for (i, line) in s.lines().enumerate() {
        if i >= MAX_LINES || out.len() + line.len() + 1 > MAX_CHARS {
            out.push_str("\n…");
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------
// Paleta
// ---------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Palette {
    bg_app: Color,
    bg_panel: Color,
    bg_sel: Color,
    fg_text: Color,
    fg_muted: Color,
    fg_destructive: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            bg_app: Color::from_rgba8(14, 16, 22, 255),
            bg_panel: Color::from_rgba8(22, 26, 36, 255),
            bg_sel: Color::from_rgba8(58, 78, 128, 255),
            fg_text: Color::from_rgba8(214, 222, 232, 255),
            fg_muted: Color::from_rgba8(140, 152, 170, 255),
            fg_destructive: Color::from_rgba8(220, 110, 110, 255),
        }
    }
}
