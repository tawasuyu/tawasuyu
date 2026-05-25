//! `nahual-shell-llimphi` — MVP del shell nahual sobre Llimphi.
//!
//! Composición mínima: barra superior con la ruta + split draggable
//! con `nahual-file-explorer-llimphi` a la izquierda y
//! `nahual-text-viewer-llimphi` a la derecha. Foco en validar la
//! composición Llimphi y consumir crates reusables; no en paridad con
//! el shell GPUI.
//!
//! Lo que **sí** hace este MVP:
//! - Navegación con teclado: ↑/↓ y rueda mueven la selección/scroll;
//!   Enter entra a un directorio o abre un archivo; Backspace sube al
//!   padre.
//! - Click en una fila: selecciona; si es archivo, lo previsualiza.
//! - Preview de archivos texto pequeños (delegado al crate
//!   `nahual-text-viewer-llimphi`, ≤ 256 KB, UTF-8 sin null bytes).
//! - Splitter draggable.
//!
//! El viewer se elige por extensión (PNG/JPG/JPEG → image, resto →
//! text con fallback a "binario"). Esto es deliberadamente tosco:
//! cuando exista un `MimeRegistry` o un AppBus con `EntityType`,
//! pasará a usar eso.
//!
//! Lo que **todavía** no:
//! - `layout.json` / `Persister` / hot-reload.
//! - Otros containers (Tabs, Tiled) y otro viewer (Database).
//! - AppBus: el viewer recibe el path directo desde el modelo. Cuando
//!   tengamos un bus, el shell publica `EntitySelected` y los viewers
//!   se suscriben.

use std::path::{Path, PathBuf};

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_theme::Theme;
use llimphi_widget_list::ListPalette;
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use nahual_file_explorer_llimphi::{
    file_explorer_view, FileExplorerState, OpenedFile,
};
use nahual_image_viewer_llimphi::{
    image_viewer_view, load_image, ImagePreviewState, ImageViewerPalette,
    DEFAULT_IMAGE_BYTES_MAX,
};
use nahual_text_viewer_llimphi::{
    load_preview, text_viewer_view, PreviewState, TextViewerPalette,
    DEFAULT_PREVIEW_BYTES_MAX,
};

fn main() {
    llimphi_ui::run::<Shell>();
}

/// Qué viewer pinta el panel derecho. Se decide por extensión del
/// path seleccionado en [`detect_kind`]; los archivos sin match
/// caen como `Text` y el text viewer los muestra como binarios si
/// no son UTF-8 — es un fallback razonable que pasa por la guard
/// existente de `load_preview`.
enum PreviewPane {
    Empty,
    Text(PreviewState),
    Image(ImagePreviewState),
}

struct Model {
    explorer: FileExplorerState,
    /// Ancho del panel izquierdo en px. Lo muta el drag del splitter.
    list_width: f32,
    preview: PreviewPane,
    /// Path del archivo previsualizado (header del panel derecho).
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
        Model {
            explorer: FileExplorerState::new(cwd),
            list_width: 400.0,
            preview: PreviewPane::Empty,
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
        _model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Self::Msg> {
        // El delta del touchpad se acumula en `FileExplorerState`; acá
        // sólo aproximamos los pasos para evitar un round-trip por
        // sub-fila. El update llamará a `apply_wheel(delta.y)` para que
        // el acumulador real viva en el explorer, no en el shell.
        let steps = delta.y.trunc() as i32;
        Some(Msg::Scroll(steps))
    }

    fn update(model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Up => {
                if m.explorer.up() {
                    refresh_preview(&mut m);
                }
            }
            Msg::Down => {
                if m.explorer.down() {
                    refresh_preview(&mut m);
                }
            }
            Msg::Select(idx) => {
                if m.explorer.select(idx) {
                    refresh_preview(&mut m);
                }
            }
            Msg::OpenSelected => {
                match m.explorer.open_selected() {
                    Some(OpenedFile::Directory) => {
                        m.preview = PreviewPane::Empty;
                        m.preview_of = None;
                    }
                    Some(OpenedFile::File(path)) => {
                        m.preview = load_for(&path);
                        m.preview_of = Some(path);
                    }
                    None => {}
                }
            }
            Msg::Parent => {
                if m.explorer.parent() {
                    refresh_preview(&mut m);
                }
            }
            Msg::ResizeList(dx) => {
                m.list_width = (m.list_width + dx).clamp(220.0, 900.0);
            }
            Msg::Scroll(steps) => {
                // El explorer tiene su propio acumulador para
                // touchpads — le pasamos el delta crudo (en líneas).
                m.explorer.apply_wheel(steps as f32);
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = Theme::dark();
        let splitter_palette = SplitterPalette::from_theme(&theme);
        let text_palette = TextViewerPalette::from_theme(&theme);
        let image_palette = ImageViewerPalette::from_theme(&theme);
        let header = header_bar(model, &theme);
        let list_pane = file_explorer_view::<Msg, _>(
            &model.explorer,
            ListPalette::from_theme(&theme),
            Msg::Select,
        );
        let viewer_pane = match &model.preview {
            PreviewPane::Empty => text_viewer_view::<Msg>(
                &PreviewState::Empty,
                None,
                &text_palette,
            ),
            PreviewPane::Text(state) => text_viewer_view::<Msg>(
                state,
                model.preview_of.as_deref(),
                &text_palette,
            ),
            PreviewPane::Image(state) => image_viewer_view::<Msg>(
                state,
                model.preview_of.as_deref(),
                &image_palette,
            ),
        };

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
        format!("nahual · {}", model.explorer.cwd.display()),
        12.0,
        theme.fg_text,
        Alignment::Start,
    )
}

/// Releé el preview del entry seleccionado tras un cambio de selección.
/// Si es directorio, limpia; si es archivo, lo carga sync con el
/// viewer apropiado según extensión.
fn refresh_preview(m: &mut Model) {
    let Some(entry) = m.explorer.selected_entry() else {
        m.preview = PreviewPane::Empty;
        m.preview_of = None;
        return;
    };
    if entry.is_dir {
        m.preview = PreviewPane::Empty;
        m.preview_of = None;
        return;
    }
    let Some(path) = m.explorer.selected_path() else {
        m.preview = PreviewPane::Empty;
        m.preview_of = None;
        return;
    };
    m.preview = load_for(&path);
    m.preview_of = Some(path);
}

/// Decide qué viewer usar según la extensión del path y dispara la
/// carga sync. PNG/JPG/JPEG → image viewer; cualquier otro → text
/// viewer (que ya degrada a "binario" si no es UTF-8).
fn load_for(path: &Path) -> PreviewPane {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png") | Some("jpg") | Some("jpeg") => {
            PreviewPane::Image(load_image(path, DEFAULT_IMAGE_BYTES_MAX))
        }
        _ => PreviewPane::Text(load_preview(path, DEFAULT_PREVIEW_BYTES_MAX)),
    }
}
