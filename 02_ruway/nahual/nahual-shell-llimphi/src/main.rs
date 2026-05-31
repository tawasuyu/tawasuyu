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
//! El viewer se elige por **contenido**, no por extensión:
//! `viewer_registry::pick` despacha el `Discernment` de `shuma-discern`
//! (magic-bytes, JSON/TOML/Card probe, UTF-8) al visor que sabe pintar
//! esa naturaleza de dato. Es el germen del "open-with universal":
//! cuando lleguen más visores y un AppBus con `EntityType`, el registro
//! crece por tabla sin tocar el resto del shell.
//!
//! Hoy embebe once visores in-process — texto (fallback universal),
//! imagen, video (AV1 nativo), audio (WAV/MP3/FLAC/Opus/Vorbis por cpal,
//! con espectro en vivo), card (`shared/card` presentada por campos),
//! tree (árbol JSON/TOML indentado), hex (dump de binarios), table
//! (CSV/TSV alineado), markdown (`.md` renderizado con encabezados,
//! listas, código y citas), archive (listado de ZIP/tar/tar.gz; ZIP
//! cubre .jar/.apk/.epub/OOXML) y font (TTF/OTF: metadatos + muestra
//! dibujada con los contornos de la propia fuente) — todos ruteados por
//! `viewer_registry::pick` sobre el `lens`/`mime` discernido. `Space`
//! hace play/pausa del video o audio.
//!
//! Lo que **todavía** no:
//! - `layout.json` / `Persister` / hot-reload.
//! - Otros containers (Tabs, Tiled) y un reader PDF nativo.
//! - AppBus: el viewer recibe el path directo desde el modelo. Cuando
//!   tengamos un bus, el shell publica `EntitySelected` y los viewers
//!   se suscriben.

use std::path::{Path, PathBuf};
use std::time::Duration;

mod viewer_registry;
use viewer_registry::ViewerKind;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_theme::Theme;
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use nahual_file_explorer_llimphi::{
    file_explorer_view, FileExplorerState, OpenedFile,
};
use nahual_source_core::{MingaSource, Navigator, NouserSource, Opened, WawaImgSource};
use nahual_image_viewer_llimphi::{
    image_viewer_view, load_image, ImagePreviewState, ImageViewerPalette,
    DEFAULT_IMAGE_BYTES_MAX,
};
use nahual_text_viewer_llimphi::{
    load_preview, text_viewer_view, PreviewState, TextViewerPalette,
    DEFAULT_PREVIEW_BYTES_MAX,
};
use nahual_video_viewer_llimphi::{
    video_viewer_view, VideoViewerPalette, VideoViewerState,
};
use nahual_card_viewer_llimphi::{
    card_viewer_view, load_card, CardPreview, CardViewerPalette,
};
use nahual_audio_viewer_llimphi::{
    audio_viewer_view, AudioViewerPalette, AudioViewerState,
};
use nahual_tree_viewer_llimphi::{
    load_tree, tree_viewer_view, TreePreview, TreeViewerPalette, DEFAULT_TREE_BYTES_MAX,
};
use nahual_hex_viewer_llimphi::{
    hex_viewer_view, load_hex, HexPreview, HexViewerPalette, DEFAULT_HEX_BYTES_MAX,
};
use nahual_table_viewer_llimphi::{
    load_table, table_viewer_view, TablePreview, TableViewerPalette, DEFAULT_TABLE_BYTES_MAX,
};
use nahual_markdown_viewer_llimphi::{
    load_markdown, markdown_viewer_view, MarkdownPreview, MarkdownViewerPalette,
    DEFAULT_MARKDOWN_BYTES_MAX,
};
use nahual_archive_viewer_llimphi::{
    archive_viewer_view, load_archive, ArchivePreview, ArchiveViewerPalette,
};
use nahual_font_viewer_llimphi::{
    font_viewer_view, load_font, FontPreview, FontViewerPalette, DEFAULT_FONT_BYTES_MAX,
};
use wawa_config_llimphi::theme_from_wawa;

fn main() {
    llimphi_ui::run::<Shell>();
}

/// Qué viewer pinta el panel derecho. Lo decide [`viewer_registry::pick`]
/// sobre el `Discernment` del **contenido** (no la extensión); los
/// archivos sin match caen como `Text` y el text viewer los muestra como
/// binarios si no son UTF-8 — fallback que pasa por la guard de
/// `load_preview`.
enum PreviewPane {
    Empty,
    Text(PreviewState),
    Image(ImagePreviewState),
    Video(VideoViewerState),
    Audio(AudioViewerState),
    Card(CardPreview),
    Tree(TreePreview),
    Hex(HexPreview),
    Table(TablePreview),
    Markdown(MarkdownPreview),
    Archive(ArchivePreview),
    Font(FontPreview),
}

/// Cadencia del avance de los visores con reloj (video, audio) ~30 Hz.
/// `spawn_periodic` la dispara siempre; el `update` sólo tickea el panel
/// derecho cuando es de los que avanzan.
const FRAME_TICK: Duration = Duration::from_millis(33);

struct Model {
    explorer: FileExplorerState,
    /// Fuente no-POSIX montada sobre el panel izquierdo (Brahman Fase 3).
    /// `Some` cuando descendimos a una imagen wawa u otra `Source`: mientras
    /// dure, la navegación se rutea acá en vez de a `explorer`. Subir desde
    /// su raíz la desmonta y vuelve a POSIX.
    mounted: Option<Navigator>,
    /// Ancho del panel izquierdo en px. Lo muta el drag del splitter.
    list_width: f32,
    preview: PreviewPane,
    /// Path del archivo previsualizado (header del panel derecho).
    preview_of: Option<PathBuf>,
    /// Materialización temporal de una hoja no-POSIX: los visores son
    /// path-based (`load_image(path)`), así que los bytes de un objeto wawa
    /// se vuelcan a un tempfile y se previsualizan por ahí. Vive mientras el
    /// visor lo lea (audio/video streamean del path); se reemplaza al cambiar
    /// de preview.
    preview_temp: Option<tempfile::TempDir>,
    theme: Theme,
    /// Suscripción al bus de configuración del SO.
    _wawa_watcher: Option<wawa_config::ConfigWatcher>,
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
    /// El bus `wawa-config` publicó una versión nueva.
    WawaConfigChanged(Box<wawa_config::WawaConfig>),
    /// Pulso de reloj de los visores con transporte (video/audio, ~30 Hz).
    Tick,
    /// Espacio: play/pausa del panel derecho si es video o audio.
    TogglePlay,
    /// `m`: montar el directorio objetivo como Mónadas semánticas (nouser).
    MountNouser,
    /// `g`: montar el directorio objetivo como grafo CAS de minga, si parece
    /// un repo `.minga` (guard anti-creación de sled en dirs ajenos).
    MountMinga,
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

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let cfg = wawa_config::WawaConfig::load();
        let theme = theme_from_wawa(&cfg, &Theme::dark());
        let handle_clone = handle.clone();
        let watcher = wawa_config::ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::WawaConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("nahual-shell · wawa-config watcher: {e}"))
        .ok();
        // Los visores con transporte (video, audio) necesitan un reloj
        // externo: cada pulso avanza un frame / refresca el espectro. Es
        // barato cuando el panel no avanza (el update sale temprano).
        handle.spawn_periodic(FRAME_TICK, || Msg::Tick);
        Model {
            explorer: FileExplorerState::new(cwd),
            mounted: None,
            list_width: 400.0,
            preview: PreviewPane::Empty,
            preview_of: None,
            preview_temp: None,
            theme,
            _wawa_watcher: watcher,
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
            Key::Named(NamedKey::Space) => Some(Msg::TogglePlay),
            // Puntos de entrada del front universal: montar el directorio
            // objetivo (el subdir seleccionado, o el cwd) como otra `Source`.
            // Sólo desde POSIX — dentro de una fuente montada no aplican.
            Key::Character(c) if c == "m" => Some(Msg::MountNouser),
            Key::Character(c) if c == "g" => Some(Msg::MountMinga),
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
                if m.mounted.is_some() {
                    if m.mounted.as_mut().unwrap().up() {
                        refresh_preview_nav(&mut m);
                    }
                } else if m.explorer.up() {
                    refresh_preview(&mut m);
                }
            }
            Msg::Down => {
                if m.mounted.is_some() {
                    if m.mounted.as_mut().unwrap().down() {
                        refresh_preview_nav(&mut m);
                    }
                } else if m.explorer.down() {
                    refresh_preview(&mut m);
                }
            }
            Msg::Select(idx) => {
                if m.mounted.is_some() {
                    if m.mounted.as_mut().unwrap().select(idx) {
                        refresh_preview_nav(&mut m);
                    }
                } else if m.explorer.select(idx) {
                    refresh_preview(&mut m);
                }
            }
            Msg::OpenSelected => {
                if m.mounted.is_some() {
                    abrir_en_fuente(&mut m);
                } else {
                    match m.explorer.open_selected() {
                        Some(OpenedFile::Directory) => clear_preview(&mut m),
                        Some(OpenedFile::File(path)) => {
                            // Content-based: si el archivo abre como imagen
                            // wawa, montamos su DAG en el panel izquierdo en
                            // vez de previsualizarlo. Cualquier otra cosa cae
                            // al open-with universal de siempre.
                            match try_mount(&path) {
                                Some(nav) => {
                                    m.mounted = Some(nav);
                                    clear_preview(&mut m);
                                }
                                None => {
                                    m.preview = load_for(&path);
                                    m.preview_of = Some(path);
                                    m.preview_temp = None;
                                }
                            }
                        }
                        None => {}
                    }
                }
            }
            Msg::Parent => {
                if m.mounted.is_some() {
                    match m.mounted.as_mut().unwrap().parent() {
                        Ok(true) => refresh_preview_nav(&mut m),
                        // Subir desde la raíz de la fuente la desmonta:
                        // volvemos al filesystem POSIX.
                        Ok(false) => {
                            m.mounted = None;
                            clear_preview(&mut m);
                        }
                        Err(_) => {}
                    }
                } else if m.explorer.parent() {
                    refresh_preview(&mut m);
                }
            }
            Msg::ResizeList(dx) => {
                m.list_width = (m.list_width + dx).clamp(220.0, 900.0);
            }
            Msg::Scroll(steps) => {
                // El navegador (o el explorer) tiene su propio acumulador
                // para touchpads — le pasamos el delta crudo (en líneas).
                if let Some(nav) = m.mounted.as_mut() {
                    nav.apply_wheel(steps as f32);
                } else {
                    m.explorer.apply_wheel(steps as f32);
                }
            }
            Msg::WawaConfigChanged(cfg) => {
                m.theme = theme_from_wawa(&cfg, &m.theme);
                // nahual-shell no usa rimay_localize hoy; si en el
                // futuro lo hace, agregar el set_locale acá.
            }
            Msg::Tick => match &mut m.preview {
                PreviewPane::Video(state) => {
                    state.tick(FRAME_TICK);
                }
                PreviewPane::Audio(state) => state.tick(FRAME_TICK),
                _ => {}
            },
            Msg::TogglePlay => match &mut m.preview {
                PreviewPane::Video(state) => state.toggle_play(),
                PreviewPane::Audio(state) => state.toggle_play(),
                _ => {}
            },
            Msg::MountNouser => {
                // Sólo montamos desde POSIX (no anidamos fuentes). nouser sólo
                // LEE el dir, así que no hay riesgo de efecto secundario.
                if m.mounted.is_none() {
                    let dir = target_dir(&m);
                    if let Some(nav) = NouserSource::escanear(&dir, 1)
                        .ok()
                        .and_then(|src| Navigator::open(Box::new(src)).ok())
                    {
                        m.mounted = Some(nav);
                        clear_preview(&mut m);
                    }
                }
            }
            Msg::MountMinga => {
                // Guard: `PersistentRepo::open` (sled) CREA archivos si el dir
                // no es un repo — sólo montamos si ya parece uno, para no
                // ensuciar directorios ajenos.
                if m.mounted.is_none() {
                    let dir = target_dir(&m);
                    if parece_repo_minga(&dir) {
                        if let Some(nav) = MingaSource::abrir(&dir)
                            .ok()
                            .and_then(|src| Navigator::open(Box::new(src)).ok())
                        {
                            m.mounted = Some(nav);
                            clear_preview(&mut m);
                        }
                    }
                }
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = model.theme;
        let splitter_palette = SplitterPalette::from_theme(&theme);
        let text_palette = TextViewerPalette::from_theme(&theme);
        let image_palette = ImageViewerPalette::from_theme(&theme);
        let video_palette = VideoViewerPalette::from_theme(&theme);
        let audio_palette = AudioViewerPalette::from_theme(&theme);
        let card_palette = CardViewerPalette::from_theme(&theme);
        let tree_palette = TreeViewerPalette::from_theme(&theme);
        let hex_palette = HexViewerPalette::from_theme(&theme);
        let table_palette = TableViewerPalette::from_theme(&theme);
        let markdown_palette = MarkdownViewerPalette::from_theme(&theme);
        let archive_palette = ArchiveViewerPalette::from_theme(&theme);
        let font_palette = FontViewerPalette::from_theme(&theme);
        let header = header_bar(model, &theme);
        let list_pane = match &model.mounted {
            Some(nav) => navigator_list_view(nav, ListPalette::from_theme(&theme)),
            None => file_explorer_view::<Msg, _>(
                &model.explorer,
                ListPalette::from_theme(&theme),
                Msg::Select,
            ),
        };
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
            PreviewPane::Video(state) => video_viewer_view::<Msg>(state, &video_palette),
            PreviewPane::Audio(state) => audio_viewer_view::<Msg>(state, &audio_palette),
            PreviewPane::Card(state) => {
                card_viewer_view::<Msg>(state, model.preview_of.as_deref(), &card_palette)
            }
            PreviewPane::Tree(state) => {
                tree_viewer_view::<Msg>(state, model.preview_of.as_deref(), &tree_palette)
            }
            PreviewPane::Hex(state) => {
                hex_viewer_view::<Msg>(state, model.preview_of.as_deref(), &hex_palette)
            }
            PreviewPane::Table(state) => {
                table_viewer_view::<Msg>(state, model.preview_of.as_deref(), &table_palette)
            }
            PreviewPane::Markdown(state) => {
                markdown_viewer_view::<Msg>(state, model.preview_of.as_deref(), &markdown_palette)
            }
            PreviewPane::Archive(state) => {
                archive_viewer_view::<Msg>(state, model.preview_of.as_deref(), &archive_palette)
            }
            PreviewPane::Font(state) => {
                font_viewer_view::<Msg>(state, model.preview_of.as_deref(), &font_palette)
            }
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
        match &model.mounted {
            // Montados sobre una fuente: mostramos su etiqueta + la miga de
            // pan dentro de ella, con un glifo que marca que no es POSIX.
            Some(nav) => format!("nahual · ⊟ {} · {}", nav.label(), nav.breadcrumb()),
            // En POSIX: la ruta + los atajos para montar otras fuentes.
            None => format!(
                "nahual · {}   ·   m Mónadas · g grafo minga",
                model.explorer.cwd.display()
            ),
        },
        12.0,
        theme.fg_text,
        Alignment::Start,
    )
}

/// Limpia el panel derecho y suelta cualquier tempfile de hoja no-POSIX.
fn clear_preview(m: &mut Model) {
    m.preview = PreviewPane::Empty;
    m.preview_of = None;
    m.preview_temp = None;
}

/// Intenta montar `path` como una fuente no-POSIX. Hoy sólo prueba imagen
/// wawa: `WawaImgSource::abrir` hace un chequeo de magic barato y sólo carga
/// el grafo si el archivo realmente es una imagen wawa — para todo lo demás
/// falla rápido y devolvemos `None` (se previsualiza normal).
fn try_mount(path: &Path) -> Option<Navigator> {
    let src = WawaImgSource::abrir(path).ok()?;
    Navigator::open(Box::new(src)).ok()
}

/// El directorio que un montaje explícito (`m`/`g`) toma como objetivo: el
/// subdirectorio seleccionado si lo hay, o el `cwd` del explorador POSIX.
fn target_dir(m: &Model) -> PathBuf {
    match m.explorer.selected_entry() {
        Some(entry) if entry.is_dir => m.explorer.cwd.join(&entry.name),
        _ => m.explorer.cwd.clone(),
    }
}

/// Heurística no destructiva: ¿este directorio ya parece un repo minga
/// (sled)? Chequea los artefactos que `sled::open` deja (`conf`/`db`) sin
/// abrirlo — abrir crearía esos archivos en un dir cualquiera, justo lo que
/// queremos evitar.
fn parece_repo_minga(dir: &Path) -> bool {
    dir.is_dir() && (dir.join("conf").exists() || dir.join("db").exists())
}

/// Abre la selección dentro de la fuente montada: si es contenedor desciende
/// (limpia el preview); si es hoja, vuelca sus bytes a un tempfile y lo
/// previsualiza con el open-with universal de siempre.
fn abrir_en_fuente(m: &mut Model) {
    let Some(nav) = m.mounted.as_mut() else { return };
    match nav.open_selected() {
        Ok(Some(Opened::Descended)) => clear_preview(m),
        Ok(Some(Opened::Leaf(id))) => {
            // Releemos por el nodo ya seleccionado (open_selected no movió la
            // selección al ser hoja).
            let nombre = nav
                .selected_node()
                .map(|n| n.name.clone())
                .unwrap_or_default();
            match nav.read(&id) {
                Ok(bytes) => preview_from_bytes(m, bytes, &nombre),
                Err(_) => clear_preview(m),
            }
        }
        Ok(None) | Err(_) => {}
    }
}

/// Releé el preview tras moverse dentro de la fuente montada. Hoja → vuelca a
/// tempfile y previsualiza; contenedor (o nada) → limpia.
fn refresh_preview_nav(m: &mut Model) {
    // Calculamos la acción soltando el préstamo de `m.mounted` antes de
    // mutar el resto del modelo (el preview).
    enum Accion {
        Limpiar,
        Mostrar(Vec<u8>, String),
    }
    let accion = match m.mounted.as_ref().and_then(|nav| nav.selected_node().map(|n| (nav, n))) {
        Some((nav, node)) if !node.is_container => match nav.read(&node.id) {
            Ok(bytes) => Accion::Mostrar(bytes, node.name.clone()),
            Err(_) => Accion::Limpiar,
        },
        _ => Accion::Limpiar,
    };
    match accion {
        Accion::Limpiar => clear_preview(m),
        Accion::Mostrar(bytes, nombre) => preview_from_bytes(m, bytes, &nombre),
    }
}

/// Materializa los bytes de una hoja no-POSIX en un tempfile y la
/// previsualiza con [`load_for`]. El tempdir se guarda en el modelo para que
/// el path siga válido mientras el visor lo lea (audio/video streamean).
fn preview_from_bytes(m: &mut Model, bytes: Vec<u8>, nombre: &str) {
    let Ok(dir) = tempfile::tempdir() else {
        clear_preview(m);
        return;
    };
    let path = dir.path().join(sanitizar_nombre(nombre));
    if std::fs::write(&path, &bytes).is_ok() {
        m.preview = load_for(&path);
        m.preview_of = Some(path);
        m.preview_temp = Some(dir); // mantener vivo el tempdir
    } else {
        clear_preview(m);
    }
}

/// Vuelve un nombre de nodo apto para un filename de tempfile (los objetos
/// wawa son hashes sin separadores, pero por las dudas sacamos `/` y `\`).
fn sanitizar_nombre(nombre: &str) -> String {
    let limpio: String = nombre
        .chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    if limpio.is_empty() {
        "objeto".to_string()
    } else {
        limpio
    }
}

/// Pinta los hijos del contenedor actual de la fuente montada como una lista
/// `llimphi-widget-list` — el gemelo genérico de `file_explorer_view`.
fn navigator_list_view(nav: &Navigator, palette: ListPalette) -> View<Msg> {
    use std::cmp::min;
    let nodes = nav.children();
    let start = nav.visible_offset.min(nodes.len());
    let end = min(nodes.len(), start + nav.visible_rows);
    let rows: Vec<ListRow<Msg>> = (start..end)
        .map(|idx| {
            let n = &nodes[idx];
            let icon = if n.is_container { "▸ " } else { "  " };
            let label = if n.is_container {
                format!("{icon}{}/", n.name)
            } else {
                format!("{icon}{}", n.name)
            };
            ListRow {
                label,
                selected: idx == nav.selected,
                on_click: Msg::Select(idx),
            }
        })
        .collect();
    let caption = format!("{} entradas · ↑↓ navega · Enter abre · ⌫ vuelve", nodes.len());
    let truncated_hint = if nodes.len() > end {
        Some(format!("… y {} más (rueda o ↓ para ver más)", nodes.len() - end))
    } else {
        None
    };
    list_view(ListSpec {
        rows,
        total: nodes.len(),
        caption: Some(caption),
        truncated_hint,
        row_height: 22.0,
        palette,
    })
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

/// Decide qué viewer usar discerniendo el **contenido** del archivo (no
/// la extensión) y dispara la carga sync. Lee una muestra del header,
/// la pasa por `shuma-discern`, y `viewer_registry::pick` elige el visor.
/// Un .png con la extensión equivocada ahora se abre igual como imagen;
/// un archivo ilegible cae al text viewer (que degrada a "binario").
fn load_for(path: &Path) -> PreviewPane {
    let sample = read_header_sample(path, DISCERN_SAMPLE_BYTES);
    let pipeline = shuma_discern::DiscernPipeline::default();
    let hint = shuma_discern::Hint {
        path: path.to_str(),
        size_total: std::fs::metadata(path).ok().map(|m| m.len()),
    };
    let discernment = sample
        .as_deref()
        .and_then(|s| pipeline.discern(s, &hint));

    match viewer_registry::pick(discernment.as_ref()) {
        ViewerKind::Image => PreviewPane::Image(load_image(path, DEFAULT_IMAGE_BYTES_MAX)),
        ViewerKind::Video => PreviewPane::Video(open_video(path)),
        ViewerKind::Audio => PreviewPane::Audio(AudioViewerState::open(path)),
        ViewerKind::Card => PreviewPane::Card(load_card(path)),
        ViewerKind::Tree => PreviewPane::Tree(load_tree(path, DEFAULT_TREE_BYTES_MAX)),
        ViewerKind::Hex => PreviewPane::Hex(load_hex(path, DEFAULT_HEX_BYTES_MAX)),
        ViewerKind::Table => PreviewPane::Table(load_table(path, DEFAULT_TABLE_BYTES_MAX)),
        ViewerKind::Markdown => {
            PreviewPane::Markdown(load_markdown(path, DEFAULT_MARKDOWN_BYTES_MAX))
        }
        ViewerKind::Archive => PreviewPane::Archive(load_archive(path)),
        ViewerKind::Font => PreviewPane::Font(load_font(path, DEFAULT_FONT_BYTES_MAX)),
        ViewerKind::Text => PreviewPane::Text(load_preview(path, DEFAULT_PREVIEW_BYTES_MAX)),
    }
}

/// Abre un archivo de video con el constructor adecuado del visor. El
/// contenido ya se discernió como video; acá la extensión sólo decide
/// el *demuxer*: WebM/MKV (EBML) van por `media-source-webm`, el resto
/// (incluido `.ivf`) por el path AV1 crudo. Si la extensión miente, el
/// visor cae a estado de error y lo muestra en su header.
fn open_video(path: &Path) -> VideoViewerState {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("webm" | "mkv") => VideoViewerState::open_webm(path),
        Some("gif") => VideoViewerState::open_gif(path),
        _ => VideoViewerState::open_av1(path),
    }
}

/// Cuántos bytes del header alcanzan a `shuma-discern`. Los magic-bytes y
/// el arranque de JSON/TOML viven en los primeros KB; no hace falta leer
/// el archivo entero sólo para elegir visor.
const DISCERN_SAMPLE_BYTES: usize = 8 * 1024;

/// Lee hasta `max` bytes del inicio del archivo para discernir su tipo.
/// `None` si no se puede abrir/leer — el caller lo trata como "sin
/// discernimiento" y cae al text viewer.
fn read_header_sample(path: &Path, max: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}
