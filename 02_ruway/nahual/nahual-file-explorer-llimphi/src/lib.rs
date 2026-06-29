//! `nahual-file-explorer-llimphi` — explorador de directorios sobre
//! Llimphi.
//!
//! Reemplazo Llimphi del `nahual-file-explorer` GPUI. Crate fino que
//! encapsula:
//! - [`Entry`] / [`FileExplorerState`] — modelo (cwd, entries,
//!   selección, scroll, ancho del pane si aplica).
//! - Transiciones puras: [`FileExplorerState::up`], `down`,
//!   `open_selected`, `parent`, `select`, `scroll` y `apply_wheel`
//!   (cada una devuelve un boolean o un [`OpenedFile`] que el caller
//!   usa para decidir si previsualizar/abrir).
//! - [`file_explorer_view`] — pinta la lista virtualizada con
//!   `llimphi-widget-list`, devolviendo Msgs vía un mapper de
//!   `usize` (índice fila) a `Msg` que el caller provee.
//!
//! El estado es puro: no abre archivos, no llama a `fs::read` —
//! sólo navega. La lectura/preview es responsabilidad del consumidor
//! (`nahual-text-viewer-llimphi`, `nahual-image-viewer-llimphi`,
//! etc.). Reaccionar a `OpenedFile` es típicamente "cargar preview
//! con `load_preview`/`load_image` y guardarlo en mi modelo".

#![forbid(unsafe_code)]

use std::cmp::min;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use llimphi_icons::Icon;
use llimphi_theme::{alpha, motion};
use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::View;
use llimphi_widget_empty::{empty_view, EmptyPalette};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};

/// Hash estable de una cadena → `key` para las animaciones implícitas de
/// Llimphi. El mismo `cwd` produce siempre la misma key entre rebuilds,
/// así el pop-in corre sólo al cambiar de carpeta (no en cada repintado
/// por selección o scroll).
fn key_of(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Deriva una [`EmptyPalette`] desde la [`ListPalette`] (que no acarrea un
/// `Theme`). Mismo criterio que `EmptyPalette::from_theme`: ícono y
/// descripción apagados sobre `fg_muted`.
fn empty_palette(p: &ListPalette) -> EmptyPalette {
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    let dim = |a: u8| {
        let [r, g, b, _] = p.fg_muted.components;
        AlphaColor::new([r, g, b, a as f32 / 255.0])
    };
    EmptyPalette {
        fg_icon: dim(alpha::HINT),
        fg_title: p.fg_muted,
        fg_desc: dim(alpha::DISABLED),
    }
}

/// Una entrada del directorio actual.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

/// Cuántas filas mostramos a la vez. Calibrado para un viewport típico
/// (alto del pane ≈ 760 px ÷ 22 px/row ≈ 34 filas). El caller puede
/// crear el state con otro valor si tiene viewports distintos.
pub const DEFAULT_VISIBLE_ROWS: usize = 32;
/// Alto en px de cada fila. Lo usa `ListSpec`.
pub const DEFAULT_ROW_HEIGHT: f32 = 22.0;
/// "Líneas" de la rueda que equivalen a una fila.
pub const WHEEL_LINES_PER_ROW: f32 = 1.0;

/// Resultado de [`FileExplorerState::open_selected`]: si la selección
/// es un directorio, ya se hizo el `cd` (`Directory`); si es archivo,
/// devuelve el path para que el caller decida qué hacer (preview,
/// abrir con app externa, etc.).
#[derive(Clone, Debug)]
pub enum OpenedFile {
    Directory,
    File(PathBuf),
}

/// Estado del explorador. Puro: mutarlo es trivial y no toca IO
/// excepto en `cd` (que sí relee el directorio nuevo). El caller lo
/// mantiene en su `Model` y pasa `&mut state` a las transiciones.
pub struct FileExplorerState {
    pub cwd: PathBuf,
    pub entries: Vec<Entry>,
    pub selected: usize,
    pub visible_offset: usize,
    /// Acumulador fraccional de la rueda — para touchpads que mandan
    /// deltas chicos. `apply_wheel` lo lee, calcula los pasos enteros
    /// y deja la fracción residual.
    pub wheel_accum: f32,
    pub visible_rows: usize,
}

impl FileExplorerState {
    /// Construye un explorador anclado en `cwd`. Si el path no se
    /// puede leer, las entries quedan vacías; mostrar mensaje queda
    /// para el caller.
    pub fn new(cwd: PathBuf) -> Self {
        let entries = scan_dir(&cwd);
        Self {
            cwd,
            entries,
            selected: 0,
            visible_offset: 0,
            wheel_accum: 0.0,
            visible_rows: DEFAULT_VISIBLE_ROWS,
        }
    }

    /// El path resultante de combinar `cwd` con la entrada
    /// seleccionada. `None` si no hay selección válida.
    pub fn selected_path(&self) -> Option<PathBuf> {
        self.entries
            .get(self.selected)
            .map(|e| self.cwd.join(&e.name))
    }

    /// La entrada actualmente seleccionada (clonada).
    pub fn selected_entry(&self) -> Option<Entry> {
        self.entries.get(self.selected).cloned()
    }

    /// Mueve la selección una fila arriba si se puede.
    pub fn up(&mut self) -> bool {
        if self.selected == 0 {
            return false;
        }
        self.selected -= 1;
        self.sync_offset();
        true
    }

    /// Mueve la selección una fila abajo si se puede.
    pub fn down(&mut self) -> bool {
        if self.selected + 1 >= self.entries.len() {
            return false;
        }
        self.selected += 1;
        self.sync_offset();
        true
    }

    /// Selecciona la entrada en `idx` (con bound check + scroll sync).
    pub fn select(&mut self, idx: usize) -> bool {
        if idx >= self.entries.len() {
            return false;
        }
        self.selected = idx;
        self.sync_offset();
        true
    }

    /// Si la selección es un directorio, hace `cd` (relee entries,
    /// resetea selección/offset). Si es archivo, devuelve su path.
    pub fn open_selected(&mut self) -> Option<OpenedFile> {
        let entry = self.entries.get(self.selected).cloned()?;
        if entry.is_dir {
            let new_cwd = self.cwd.join(&entry.name);
            // Canonicalize cuando se pueda: resuelve symlinks y "..".
            // Si falla (permisos), nos quedamos con el join textual.
            self.cwd = fs::canonicalize(&new_cwd).unwrap_or(new_cwd);
            self.entries = scan_dir(&self.cwd);
            self.selected = 0;
            self.visible_offset = 0;
            Some(OpenedFile::Directory)
        } else {
            Some(OpenedFile::File(self.cwd.join(&entry.name)))
        }
    }

    /// Sube al directorio padre. Si estaba parado sobre un subdir, lo
    /// re-selecciona al subir (UX típica: mantenés contexto).
    pub fn parent(&mut self) -> bool {
        let Some(parent) = self.cwd.parent().map(Path::to_path_buf) else {
            return false;
        };
        let prev_name = self
            .cwd
            .file_name()
            .map(|s| s.to_string_lossy().to_string());
        self.cwd = parent;
        self.entries = scan_dir(&self.cwd);
        self.selected = prev_name
            .and_then(|n| self.entries.iter().position(|e| e.name == n))
            .unwrap_or(0);
        self.visible_offset = 0;
        self.sync_offset();
        true
    }

    /// Aplica un delta de rueda y devuelve cuántos pasos enteros se
    /// movieron (positivo = abajo, negativo = arriba). El acumulador
    /// se ajusta para guardar la fracción residual — útil para
    /// touchpads que mandan deltas sub-fila.
    pub fn apply_wheel(&mut self, delta_y: f32) -> i32 {
        let total = self.wheel_accum + delta_y;
        let steps = (total / WHEEL_LINES_PER_ROW).trunc() as i32;
        self.wheel_accum = total - (steps as f32 * WHEEL_LINES_PER_ROW);
        if steps != 0 {
            self.scroll(steps);
        }
        steps
    }

    /// Scroll por N pasos enteros (positivo = abajo). No mueve la
    /// selección.
    pub fn scroll(&mut self, steps: i32) {
        if steps == 0 {
            return;
        }
        let len = self.entries.len();
        let max_offset = len.saturating_sub(self.visible_rows);
        if steps > 0 {
            self.visible_offset = min(self.visible_offset + steps as usize, max_offset);
        } else {
            let drop = (-steps) as usize;
            self.visible_offset = self.visible_offset.saturating_sub(drop);
        }
    }

    /// Asegura que la selección esté dentro del viewport visible.
    fn sync_offset(&mut self) {
        if self.selected < self.visible_offset {
            self.visible_offset = self.selected;
        }
        let bottom = self.visible_offset + self.visible_rows;
        if self.selected >= bottom {
            self.visible_offset = self.selected + 1 - self.visible_rows;
        }
    }
}

/// Lee el directorio y devuelve entries ordenadas (dirs primero,
/// luego por nombre case-insensitive). Si el `read_dir` falla,
/// devuelve `Vec::new()` (mostrar mensaje queda al caller).
pub fn scan_dir(path: &Path) -> Vec<Entry> {
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
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    entries
}

/// Pinta la lista de entries del explorador como `llimphi-widget-list`.
/// `on_select` recibe el índice de la fila clickeada y devuelve el
/// Msg que el caller quiera dispatchear (típicamente
/// `Msg::Select(idx)`).
pub fn file_explorer_view<Msg, F>(
    state: &FileExplorerState,
    palette: ListPalette,
    on_select: F,
) -> View<Msg>
where
    Msg: Clone + 'static,
    F: Fn(usize) -> Msg,
{
    let scene_key = key_of(&state.cwd.to_string_lossy());

    // Carpeta vacía (o ilegible): empty-state con orientación en vez de un
    // panel en blanco. Entra con el mismo pop-in que la lista.
    if state.entries.is_empty() {
        let pal = empty_palette(&palette);
        let desc = rimay_localize::t_args(
            "nahual-fe-no-entries",
            &[("path", state.cwd.display().to_string().into())],
        );
        return View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(palette.bg_panel)
        .children(vec![empty_view(
            Icon::Folder,
            rimay_localize::t("nahual-fe-empty"),
            Some(&desc),
            &pal,
        )])
        .animated_enter(scene_key, motion::NORMAL);
    }

    let start = state.visible_offset;
    let end = min(state.entries.len(), start + state.visible_rows);
    let rows: Vec<ListRow<Msg>> = (start..end)
        .map(|idx| {
            let entry = &state.entries[idx];
            let icon = if entry.is_dir { "▸ " } else { "  " };
            let label = if entry.is_dir {
                format!("{}{}/", icon, entry.name)
            } else {
                format!("{}{}", icon, entry.name)
            };
            ListRow {
                label,
                selected: idx == state.selected,
                on_click: on_select(idx),
            }
        })
        .collect();

    let caption = rimay_localize::t_args(
        "nahual-fe-caption",
        &[("n", state.entries.len().to_string().into())],
    );
    let truncated_hint = if state.entries.len() > end {
        Some(rimay_localize::t_args(
            "nahual-fe-more",
            &[("n", (state.entries.len() - end).to_string().into())],
        ))
    } else {
        None
    };

    // Pop-in de la lista al navegar: la `scene_key` cambia con el `cwd`, así
    // la lista entra con un fade suave al entrar a una carpeta nueva y queda
    // estable mientras sólo cambian selección o scroll dentro de la misma.
    list_view(ListSpec {
        rows,
        total: state.entries.len(),
        caption: Some(caption),
        truncated_hint,
        row_height: DEFAULT_ROW_HEIGHT,
        palette,
    })
    .animated_enter(scene_key, motion::NORMAL)
}
