//! Funciones utilitarias del shell nahual (discernimiento, árbol lateral,
//! formatos persistidos, miniaturas, navegación, apertura de archivos).
//! Movido de `main.rs` en el split de 2026-06-12 (puro movimiento de código).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::modelo::*;
use crate::ops::OpKind;
use crate::state::{self, ShellState};
use crate::viewer_registry::{self, ViewerKind};
use llimphi_ui::Handle;
use nahual_source_core::{ArchiveSource, Navigator, WawaImgSource};
use llimphi_icons::Icon;
use nahual_thumb_core::{generar_thumb_de_archivo, ThumbRgba};
use nahual_text_viewer_llimphi::{load_preview, DEFAULT_PREVIEW_BYTES_MAX};
use nahual_image_viewer_llimphi::{load_image, DEFAULT_IMAGE_BYTES_MAX};
use nahual_video_viewer_llimphi::VideoViewerState;
use nahual_audio_viewer_llimphi::AudioViewerState;
use nahual_card_viewer_llimphi::load_card;
use nahual_tree_viewer_llimphi::{load_tree, DEFAULT_TREE_BYTES_MAX};
use nahual_hex_viewer_llimphi::{load_hex, DEFAULT_HEX_BYTES_MAX};
use nahual_table_viewer_llimphi::{load_table, DEFAULT_TABLE_BYTES_MAX};
use nahual_markdown_viewer_llimphi::{load_markdown, DEFAULT_MARKDOWN_BYTES_MAX};
use nahual_archive_viewer_llimphi::load_archive;
use nahual_font_viewer_llimphi::{load_font, DEFAULT_FONT_BYTES_MAX};
use nahual_map_viewer_llimphi::{load_map, Basemap, MapPreview, DEFAULT_MAP_BYTES_MAX};
use llimphi_widget_text_editor::{EditorState, Language};
use tullpu_module as tullpu;
use media_module as mediamod;

/// Viewport real de la ventana (lo mantiene `Msg::Resized`). De acá salen
/// las columnas de la grilla, el ventaneo del árbol y el clamp de overlays.
pub(crate) fn viewport_of(model: &Model) -> (f32, f32) {
    model.win
}

/// ¿Hay una entrada seleccionada sobre la que tenga sentido el menú
/// contextual? En POSIX, cualquier entry del explorer; en una fuente
/// montada, el nodo seleccionado.
pub(crate) fn hay_seleccion(m: &Model) -> bool {
    m.cur().selected_node().is_some()
}

/// Discierne el **mime** del contenido de `path` con el pipeline real de shuma
/// (los mismos primeros KB que usa `load_for`). `None` si no se puede leer o
/// shuma no le asigna mime.
pub(crate) fn discern_mime(path: &Path) -> Option<String> {
    let sample = read_header_sample(path, DISCERN_SAMPLE_BYTES)?;
    let pipeline = shuma_discern::DiscernPipeline::default();
    let hint = shuma_discern::Hint {
        path: path.to_str(),
        size_total: std::fs::metadata(path).ok().map(|m| m.len()),
    };
    pipeline.discern(&sample, &hint)?.mime
}

/// Precomputa las opciones de open-with del archivo seleccionado: resuelve el
/// target (ruta POSIX real, o tempfile de una hoja no-POSIX preservando el
/// nombre/extensión), discierne su mime y consulta el `AppRegistry`. Llena
/// `ctx_target`/`ctx_temp`/`ctx_open_with`. Si la selección no es un archivo
/// abrible, deja todo vacío (el contextual sólo muestra navegación/montaje).
pub(crate) fn compute_open_with(m: &mut Model) {
    m.ctx_open_with.clear();
    m.ctx_target = None;
    m.ctx_temp = None;

    let nav = m.cur();
    let (path, temp): (Option<PathBuf>, Option<tempfile::TempDir>) = match nav.selected_node() {
        Some(n) if !n.is_container => {
            let id_path = Path::new(&n.id);
            if id_path.is_file() {
                // Hoja POSIX: su id ES la ruta real.
                (Some(id_path.to_path_buf()), None)
            } else {
                // Hoja no-POSIX (wawa/nouser/minga): materializarla a un
                // tempfile con su nombre (preserva extensión para discernir).
                match nav.read(&n.id) {
                    Ok(bytes) => match tempfile::tempdir() {
                        Ok(dir) => {
                            let p = dir.path().join(&n.name);
                            if std::fs::write(&p, &bytes).is_ok() {
                                (Some(p), Some(dir))
                            } else {
                                (None, None)
                            }
                        }
                        Err(_) => (None, None),
                    },
                    Err(_) => (None, None),
                }
            }
        }
        _ => (None, None),
    };

    let Some(path) = path else {
        return;
    };
    if let Some(mime) = discern_mime(&path) {
        for app in m.registry.handlers_for(&mime) {
            m.ctx_open_with.push((app.id.clone(), app.label.clone()));
        }
    }
    m.ctx_target = Some(path);
    m.ctx_temp = temp;
}

/// Directorio home del usuario (si existe y es un dir).
pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// Carpetas raíz del árbol lateral, con su ícono real: home, la raíz del
/// filesystem y los favoritos del usuario, sin duplicar.
pub(crate) fn tree_roots(state: &ShellState) -> Vec<(PathBuf, Icon)> {
    let mut roots: Vec<(PathBuf, Icon)> = Vec::new();
    if let Some(home) = home_dir() {
        roots.push((home, Icon::Home));
    }
    roots.push((PathBuf::from("/"), Icon::Folder));
    for p in &state.places {
        let pb = PathBuf::from(p);
        if pb.is_dir() && !roots.iter().any(|(r, _)| r == &pb) {
            roots.push((pb, Icon::Open));
        }
    }
    roots
}

/// Lista las subcarpetas (sólo directorios) de `dir`, ordenadas por nombre
/// (case-insensitive). Vacío si no se puede leer.
pub(crate) fn list_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort_by_key(|p| {
        p.file_name()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    v
}

/// Carga (si falta) el cache **global** de subcarpetas de `dir`.
pub(crate) fn ensure_tree_children(children: &mut HashMap<PathBuf, Vec<PathBuf>>, dir: &Path) {
    if !children.contains_key(dir) {
        children.insert(dir.to_path_buf(), list_dirs(dir));
    }
}

/// El set de ancestros de `target` (incluido él): `/`, `/a`, `/a/b`, … Sirve
/// para arrancar el árbol descolapsado a lo largo del camino al cwd.
pub(crate) fn ancestors_set(target: &Path) -> BTreeSet<PathBuf> {
    let mut set = BTreeSet::new();
    let mut acc = PathBuf::new();
    for comp in target.components() {
        acc.push(comp);
        set.insert(acc.clone());
    }
    set
}

/// Asegura el cache de subcarpetas para cada carpeta descolapsada.
pub(crate) fn ensure_children_for_expanded(
    children: &mut HashMap<PathBuf, Vec<PathBuf>>,
    expanded: &BTreeSet<PathBuf>,
) {
    for dir in expanded {
        ensure_tree_children(children, dir);
    }
}

/// Rótulo de un nodo del árbol: el nombre de la carpeta, o la ruta entera para
/// la raíz `/`.
pub(crate) fn node_label(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Cuenta las filas visibles del árbol (según el set de descolapsadas) — para
/// el clamp del scroll, sin construir las `View`s.
pub(crate) fn count_tree_rows(model: &Model) -> usize {
    fn rec(model: &Model, path: &Path) -> usize {
        let mut n = 1;
        if model.tree_expanded.contains(path) {
            if let Some(ch) = model.tree_children.get(path) {
                for c in ch {
                    n += rec(model, c);
                }
            }
        }
        n
    }
    tree_roots(&model.state).iter().map(|(r, _)| rec(model, r)).sum()
}

/// Alto disponible del árbol (aprox: ventana menos menubar + cabecera).
pub(crate) fn tree_viewport_h(model: &Model) -> f32 {
    let (_, vh) = viewport_of(model);
    (vh - 60.0).max(120.0)
}

/// Cuántas filas del árbol entran en el viewport.
pub(crate) fn tree_visible_rows(model: &Model) -> usize {
    (tree_viewport_h(model) / TREE_ROW_H).floor().max(1.0) as usize
}

/// Índice de columna (0 nombre · 1 tamaño · 2 fecha · 3 tipo) → `SortKey`.
pub(crate) fn col_to_sortkey(col: u8) -> nahual_source_core::SortKey {
    use nahual_source_core::SortKey::*;
    match col {
        1 => Size,
        2 => Mtime,
        3 => Kind,
        _ => Name,
    }
}

/// `SortKey` → índice de columna.
pub(crate) fn sortkey_to_col(key: nahual_source_core::SortKey) -> u8 {
    use nahual_source_core::SortKey::*;
    match key {
        Name => 0,
        Size => 1,
        Mtime => 2,
        Kind => 3,
    }
}

/// El `FolderFormat` (vista + orden) actual del navegador.
pub(crate) fn current_format(nav: &Navigator) -> state::FolderFormat {
    let (key, dir) = nav.sort();
    state::FolderFormat {
        view: view_to_u8(nav.view),
        sort_col: sortkey_to_col(key),
        sort_asc: matches!(dir, nahual_source_core::SortDir::Asc),
    }
}

/// `ViewMode` → primitivo persistible (0 lista · 1 detalle · 2 iconos).
pub(crate) fn view_to_u8(v: nahual_source_core::ViewMode) -> u8 {
    match v {
        nahual_source_core::ViewMode::List => 0,
        nahual_source_core::ViewMode::Details => 1,
        nahual_source_core::ViewMode::Icons => 2,
        nahual_source_core::ViewMode::Gallery => 3,
    }
}

/// Primitivo persistido → `ViewMode` (cualquier valor desconocido = lista).
pub(crate) fn u8_to_view(n: u8) -> nahual_source_core::ViewMode {
    match n {
        1 => nahual_source_core::ViewMode::Details,
        2 => nahual_source_core::ViewMode::Icons,
        3 => nahual_source_core::ViewMode::Gallery,
        _ => nahual_source_core::ViewMode::List,
    }
}

/// Recuerda el formato (vista/orden) de la carpeta actual del panel enfocado.
/// No-op sobre fuentes montadas (sus ids no son rutas estables).
pub(crate) fn save_format(m: &mut Model) {
    if m.is_foreign() {
        return;
    }
    let id = m.cur().current_id().clone();
    let fmt = current_format(m.cur());
    m.state.set_format(&id, fmt);
    m.state.save();
}

/// Aplica el formato guardado de la carpeta actual (si hay), tras entrar a ella.
pub(crate) fn apply_format(m: &mut Model) {
    if m.is_foreign() {
        return;
    }
    let id = m.cur().current_id().clone();
    if let Some(fmt) = m.state.format_of(&id) {
        let nav = m.cur_mut();
        nav.view = u8_to_view(fmt.view);
        nav.set_sort_to(col_to_sortkey(fmt.sort_col), fmt.sort_asc);
    }
}

/// Lado máximo (px) de las miniaturas de la vista iconos.
pub(crate) const THUMB_LADO: u32 = 128;
/// Tope de miniaturas pedidas por pasada — acota los `Handle::spawn` para que
/// una carpeta con miles de imágenes no dispare un thread por archivo.
pub(crate) const MAX_ICON_TILES: usize = 160;

/// ¿La extensión sugiere una imagen rasterizable? Filtro barato antes de
/// gastar un worker en decodificar (los no-imagen muestran su glifo de tipo).
pub(crate) fn es_imagen(path: &Path) -> bool {
    matches!(
        ext_lower(path).as_deref(),
        Some(
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" | "tif" | "ico" | "avif"
                | "qoi" | "tga"
        )
    )
}

/// Extensión en minúsculas, si hay.
pub(crate) fn ext_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

/// ¿Video del que los demuxers nativos pueden sacar un primer frame para la
/// miniatura? (WebM/MKV con track AV1, IVF crudo.)
pub(crate) fn es_video_thumbable(path: &Path) -> bool {
    matches!(ext_lower(path).as_deref(), Some("webm" | "mkv" | "ivf"))
}

/// ¿Parece video (para el glifo de la grilla), aunque no haya decoder?
pub(crate) fn es_video(path: &Path) -> bool {
    matches!(
        ext_lower(path).as_deref(),
        Some("webm" | "mkv" | "ivf" | "mp4" | "m4v" | "mov" | "avi")
    )
}

/// ¿Parece audio (para el glifo de la grilla)?
pub(crate) fn es_audio(path: &Path) -> bool {
    matches!(
        ext_lower(path).as_deref(),
        Some("wav" | "mp3" | "flac" | "opus" | "ogg" | "oga" | "m4a")
    )
}

/// Decodifica el **primer frame** de un video con los demuxers nativos
/// (`media-source-webm`/`-av1`) y lo reduce a miniatura. Corre en un worker
/// de `Handle::spawn`. `None` si el contenedor no abre, no hay track AV1, o
/// no aparece un frame en el primer segundo de reloj.
pub(crate) fn generar_thumb_de_video(path: &Path, lado: u32) -> Option<ThumbRgba> {
    use media_core::FrameSource;
    let mut src: Box<dyn FrameSource + Send> = match ext_lower(path).as_deref() {
        Some("webm" | "mkv") => Box::new(media_source_webm::WebmMedia::open(path).ok()?.video?),
        Some("ivf") => Box::new(media_source_av1::Av1VideoSource::open(path).ok()?),
        _ => return None,
    };
    let mut rgba = Vec::new();
    // ~1 s de reloj en pasos de frame: el primer keyframe decodificado sale.
    for _ in 0..30 {
        if let Some((w, h)) = src.tick(Duration::from_millis(33), &mut rgba) {
            return nahual_thumb_core::reducir_rgba(rgba, w, h, lado).ok();
        }
    }
    None
}

/// Pide (async) las miniaturas de las imágenes visibles del panel enfocado en
/// vista iconos. Sólo POSIX (los ids son rutas reales); dedup por
/// `thumbs_pending`; tope `MAX_ICON_TILES`. Cada worker reentra con
/// `Msg::ThumbReady`/`ThumbFailed`.
pub(crate) fn request_thumbs(m: &mut Model, handle: &Handle<Msg>) {
    if m.is_foreign() {
        return; // fuentes montadas (wawa/minga/archivo) no tienen path en disco
    }
    let pedir: Vec<PathBuf> = {
        let nav = m.cur();
        let visibles = nav.visible();
        let start = nav.visible_offset.min(visibles.len());
        let end = (start + MAX_ICON_TILES).min(visibles.len());
        visibles[start..end]
            .iter()
            .filter(|(_, n)| !n.is_container)
            .map(|(_, n)| PathBuf::from(&n.id))
            .filter(|p| {
                (es_imagen(p) || es_video_thumbable(p))
                    && !m.thumbs.contains_key(p)
                    && !m.thumbs_pending.contains(p)
                    && !m.thumbs_failed.contains(p)
            })
            .collect()
    };
    for path in pedir {
        m.thumbs_pending.insert(path.clone());
        handle.spawn(move || {
            // Video → primer frame con el demuxer nativo; imagen → decode.
            let thumb = if es_video_thumbable(&path) {
                generar_thumb_de_video(&path, THUMB_LADO)
            } else {
                generar_thumb_de_archivo(&path, THUMB_LADO).ok()
            };
            match thumb {
                Some(t) => Msg::ThumbReady(path, t),
                None => Msg::ThumbFailed(path),
            }
        });
    }
}

/// Registra la carpeta actual del panel enfocado como reciente (MRU).
pub(crate) fn record_recent(m: &mut Model) {
    if m.is_foreign() {
        return;
    }
    let id = m.cur().current_id().clone();
    m.state.push_recent(&id);
    m.state.save();
}

/// Encola una operación y lanza su worker (`Handle::spawn`): el job corre en un
/// hilo aparte y, al terminar, reentra al `update` con `Msg::OpFinished`. La UI
/// no se bloquea ni para una copia de un árbol grande.
pub(crate) fn enqueue(m: &mut Model, handle: &Handle<Msg>, kind: OpKind) {
    let id = m.queue.push(kind.clone());
    handle.spawn(move || {
        let result = kind.run().map_err(|e| e.to_string());
        Msg::OpFinished { id, result }
    });
}

/// Recarga los hijos de ambos paneles desde el disco tras una operación, y
/// poda las marcas que ya no apuntan a un nodo existente (borrado/movido).
pub(crate) fn reload_panes(m: &mut Model) {
    for p in m.panes.iter_mut() {
        let _ = p.nav_mut().reload();
        let ids: BTreeSet<nahual_source_core::NodeId> =
            p.nav().children().iter().map(|n| n.id.clone()).collect();
        p.marked.retain(|id| ids.contains(id));
    }
}

/// Copia (o mueve, si `is_move`) la selección del panel enfocado al directorio
/// del **otro** panel. Sólo si el destino es POSIX escribible (no se escribe
/// sobre una fuente montada read-only). Encola un job por nodo objetivo.
pub(crate) fn copy_or_move(m: &mut Model, handle: &Handle<Msg>, is_move: bool) {
    let other = 1 - m.focus;
    if m.panes[other].nav().writable().is_none() {
        return;
    }
    let dest = m.panes[other].nav().current_id().clone();
    for (id, name) in m.cur_pane().op_targets() {
        let kind = if is_move {
            OpKind::Move { id, name, dest_parent: dest.clone() }
        } else {
            OpKind::Copy { id, name, dest_parent: dest.clone() }
        };
        enqueue(m, handle, kind);
    }
    m.cur_pane_mut().marked.clear();
}

/// Limpia el panel derecho y suelta cualquier tempfile de hoja no-POSIX.
pub(crate) fn clear_preview(m: &mut Model) {
    m.preview = PreviewPane::Empty;
    m.preview_of = None;
    m.preview_temp = None;
    m.basemap = None;
}

/// Abre `path` como basemap PMTiles vivo si su magic lo delata.
pub(crate) fn open_basemap_if_pmtiles(path: &Path) -> Option<Basemap> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.starts_with(b"PMTiles") {
        Basemap::open(bytes).ok()
    } else {
        None
    }
}

/// Si hay un basemap PMTiles abierto, recalcula el viewport (tiles visibles a
/// la cámara actual) y lo deja como preview. Se llama tras cada cambio de
/// cámara para el streaming.
/// Devuelve `true` si re-streameó (había basemap y el canvas ya registró su
/// rect). `false` deja el pedido pendiente para reintentar (p. ej. en el
/// primer tick tras abrir, antes del primer paint).
pub(crate) fn restream_basemap(m: &mut Model) -> bool {
    let Some(bm) = m.basemap.as_mut() else {
        return false;
    };
    // Sin rect aún (no se pintó): conservamos el overview y reintentamos.
    if m.map_view.rect().is_none() {
        return false;
    }
    let md = bm.viewport(&m.map_view);
    m.preview = PreviewPane::Map(MapPreview::Map { data: md, truncated: false });
    true
}

/// Intenta montar `path` como una fuente no-POSIX. Hoy sólo prueba imagen
/// wawa: `WawaImgSource::abrir` hace un chequeo de magic barato y sólo carga
/// el grafo si el archivo realmente es una imagen wawa — para todo lo demás
/// falla rápido y devolvemos `None` (se previsualiza normal).
pub(crate) fn try_mount(path: &Path) -> Option<Navigator> {
    // Imagen wawa `.img` → su DAG content-addressed.
    if let Ok(src) = WawaImgSource::abrir(path) {
        return Navigator::open(Box::new(src)).ok();
    }
    // Archivo contenedor (.zip/.tar/.tar.gz) → su árbol interno como carpeta.
    if ArchiveSource::es_archivo(path) {
        if let Ok(src) = ArchiveSource::abrir(path) {
            return Navigator::open(Box::new(src)).ok();
        }
    }
    None
}

/// El directorio que un montaje explícito (`m`/`g`) toma como objetivo: el
/// subdirectorio seleccionado si lo hay, o el `cwd` del explorador POSIX.
pub(crate) fn target_dir(m: &Model) -> PathBuf {
    let nav = m.cur();
    match nav.selected_node() {
        // Subdir seleccionado (en POSIX su id ES la ruta absoluta).
        Some(n) if n.is_container => PathBuf::from(&n.id),
        // Si no, el dir actual.
        _ => PathBuf::from(nav.current_id()),
    }
}

/// Heurística no destructiva: ¿este directorio ya parece un repo minga
/// (sled)? Chequea los artefactos que `sled::open` deja (`conf`/`db`) sin
/// abrirlo — abrir crearía esos archivos en un dir cualquiera, justo lo que
/// queremos evitar.
pub(crate) fn parece_repo_minga(dir: &Path) -> bool {
    dir.is_dir() && (dir.join("conf").exists() || dir.join("db").exists())
}

/// Materializa los bytes de una hoja no-POSIX en un tempfile y la
/// previsualiza con [`load_for`]. El tempdir se guarda en el modelo para que
/// el path siga válido mientras el visor lo lea (audio/video streamean).
pub(crate) fn preview_from_bytes(m: &mut Model, bytes: Vec<u8>, nombre: &str) {
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
pub(crate) fn sanitizar_nombre(nombre: &str) -> String {
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

/// Avanza el campo de choropleth: `None → campo₀ → campo₁ → … → None`.
pub(crate) fn next_in_cycle(fields: &[String], current: &Option<String>) -> Option<String> {
    if fields.is_empty() {
        return None;
    }
    match current {
        None => fields.first().cloned(),
        Some(c) => match fields.iter().position(|f| f == c) {
            Some(i) if i + 1 < fields.len() => Some(fields[i + 1].clone()),
            _ => None,
        },
    }
}

/// Abre la selección (Enter o doble click): contenedor → desciende al canvas
/// y **revela la carpeta en el árbol lateral**; hoja → monta (`.img` wawa) o
/// abre el visor en el sidebar derecho.
pub(crate) fn do_open_selected(m: &mut Model, handle: &Handle<Msg>) {
    use nahual_source_core::Opened;
    match m.cur_mut().open_selected() {
        Ok(Some(Opened::Descended)) => {
            m.cur_pane_mut().marked.clear();
            m.canvas = None;
            clear_preview(m);
            apply_format(m);
            record_recent(m);
            // Revela la carpeta nueva en el árbol lateral (descolapsa la
            // cadena de ancestros) y sincroniza el nombre de la sesión.
            let cwd = cur_dir(m);
            if cwd.is_dir() {
                for anc in ancestors_set(&cwd) {
                    m.tree_expanded.insert(anc);
                }
                ensure_children_for_expanded(&mut m.tree_children, &m.tree_expanded);
            }
            let activa = m.active;
            m.sessions[activa].name = session_name(&cwd);
            record_history(m);
            // La nueva carpeta puede heredar vista iconos (folder format):
            // pedí sus miniaturas.
            if m.cur().view.is_grid() {
                request_thumbs(m, handle);
            }
        }
        Ok(Some(Opened::Leaf(id))) => {
            let nombre = m.cur().selected_node().map(|n| n.name.clone()).unwrap_or_default();
            let id_path = Path::new(&id);
            // Hoja POSIX (su id ES una ruta de archivo real):
            if id_path.is_file() {
                // Content-based: un `.img` wawa se MONTA (empuja su DAG);
                // cualquier otra cosa cae al open-with.
                match try_mount(id_path) {
                    Some(nav) => {
                        m.cur_pane_mut().nav_stack.push(nav);
                        clear_preview(m);
                    }
                    // Apertura integrada: texto → editor, imagen → visor con
                    // zoom, video/audio → media; el resto, preview derecho.
                    None => open_path(m, &id_path.to_path_buf()),
                }
            } else {
                // Hoja no-POSIX (wawa/nouser/minga): tempfile bridge.
                match m.cur().read(&id) {
                    Ok(bytes) => {
                        preview_from_bytes(m, bytes, &nombre);
                        m.viewer_open = true;
                    }
                    Err(_) => clear_preview(m),
                }
            }
        }
        Ok(None) | Err(_) => {}
    }
}

/// Registra la carpeta actual del panel activo en su historial (si cambió):
/// poda la cola forward y empuja el presente. Sólo carpetas POSIX.
pub(crate) fn record_history(m: &mut Model) {
    if m.is_foreign() {
        return;
    }
    let cwd = cur_dir(m);
    let pane = m.cur_pane_mut();
    if pane.hist.get(pane.hist_pos) == Some(&cwd) {
        return;
    }
    pane.hist.truncate(pane.hist_pos + 1);
    pane.hist.push(cwd);
    pane.hist_pos = pane.hist.len().saturating_sub(1);
}

/// Atrás/adelante por el historial del panel activo (delta = ±1), como un
/// navegador: moverse NO poda la cola. Revela la carpeta en el árbol.
pub(crate) fn nav_history_go(m: &mut Model, handle: &Handle<Msg>, delta: i64) {
    let pane = m.cur_pane();
    let destino = pane.hist_pos as i64 + delta;
    if destino < 0 || (destino as usize) >= pane.hist.len() {
        return;
    }
    let destino = destino as usize;
    let path = pane.hist[destino].clone();
    if !path.is_dir() {
        return;
    }
    {
        let pane = m.cur_pane_mut();
        pane.hist_pos = destino;
        pane.nav_stack = vec![posix_nav(&path)];
        pane.marked.clear();
    }
    m.canvas = None;
    apply_format(m);
    refresh_preview(m);
    for anc in ancestors_set(&path) {
        m.tree_expanded.insert(anc);
    }
    ensure_children_for_expanded(&mut m.tree_children, &m.tree_expanded);
    let activa = m.active;
    m.sessions[activa].name = session_name(&path);
    if m.cur().view.is_grid() {
        request_thumbs(m, handle);
    }
}

/// Pasa al archivo **siguiente/anterior** de la carpeta con una app de
/// canvas abierta (rueda en modo lista, botones atrás/adelante): mueve la
/// selección del panel activo saltando carpetas y abre el archivo en el
/// canvas. En los bordes de la lista no hace nada (sin wrap).
pub(crate) fn canvas_step(m: &mut Model, delta: i32) {
    if m.canvas.is_none() || delta == 0 {
        return;
    }
    let destino: Option<(usize, PathBuf)> = {
        let nav = m.cur();
        let visibles = nav.visible();
        let pos = visibles.iter().position(|(i, _)| *i == nav.selected);
        pos.and_then(|p| {
            let mut q = p as i64;
            loop {
                q += delta as i64;
                if q < 0 || q as usize >= visibles.len() {
                    return None;
                }
                let (idx, n) = &visibles[q as usize];
                if !n.is_container {
                    return Some((*idx, PathBuf::from(&n.id)));
                }
            }
        })
    };
    let Some((idx, path)) = destino else { return };
    m.cur_mut().select(idx);
    // Sólo hojas POSIX (su id ES la ruta); en fuentes montadas la selección
    // se mueve igual pero el canvas no cambia.
    if path.is_file() {
        m.canvas = None;
        open_path(m, &path);
    }
}

/// Tope de lectura para abrir un archivo en el editor del canvas. Más grande
/// que esto va al visor de texto del preview (read-only), no al editor.
pub(crate) const EDITOR_BYTES_MAX: u64 = 4 * 1024 * 1024;

/// Lenguaje de highlight por extensión (mismo mapeo que nada).
pub(crate) fn language_for_path(path: &Path) -> Language {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    Language::from_cell_language(ext)
}

/// Cuántas líneas del editor entran en el canvas.
pub(crate) fn canvas_editor_lines(m: &Model) -> usize {
    let (_, vh) = viewport_of(m);
    (((vh - 150.0) / (13.0 * 1.4)).floor() as usize).max(10)
}

/// Abre `path` de forma **integrada**: texto → editor potente en el canvas;
/// imagen → visor con zoom en el canvas; video/audio → player de media en el
/// canvas; cualquier otro tipo → el visor correspondiente en el sidebar
/// derecho de preview.
pub(crate) fn open_path(m: &mut Model, path: &PathBuf) {
    let nombre = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let pane = load_for(path);
    match pane {
        // Imagen → el editor por capas (tullpu) con sus tools en el diente
        // derecho; si no decodifica para el editor, cae al preview derecho.
        PreviewPane::Image(state) => match tullpu::State::desde_imagen(path) {
            Some(st) => {
                m.canvas = Some(CanvasApp::Imagen(Box::new(st)));
                m.tools_open = true;
                m.viewer_open = false;
            }
            None => open_in_preview(m, path, PreviewPane::Image(state)),
        },
        // Video/audio → el player de media embebible (reusa el estado ya
        // abierto por el discernimiento; controles en dientes).
        PreviewPane::Video(state) => {
            // El sidecar .srt/.vtt/.ass del video se carga solo (media-core).
            m.canvas = Some(CanvasApp::Media(Box::new(
                mediamod::State::desde_video(state, nombre).con_subtitulos_sidecar(path),
            )));
        }
        PreviewPane::Audio(state) => {
            m.canvas = Some(CanvasApp::Media(Box::new(mediamod::State::desde_audio(
                state, nombre,
            ))));
        }
        PreviewPane::Text(_) | PreviewPane::Markdown(_) | PreviewPane::Web(_) => {
            // El HTML además lanza puriy (browser real), como siempre.
            if matches!(pane, PreviewPane::Web(_)) {
                launch_puriy(path);
            }
            let chico = std::fs::metadata(path).map(|md| md.len() <= EDITOR_BYTES_MAX);
            match chico.ok().filter(|c| *c).and_then(|_| std::fs::read_to_string(path).ok()) {
                Some(contenido) => {
                    let mut editor = EditorState::new();
                    editor.set_text(&contenido);
                    m.canvas = Some(CanvasApp::Texto {
                        path: path.clone(),
                        editor: Box::new(editor),
                        dirty: false,
                        saved: false,
                    });
                }
                // Muy grande o no-UTF8: visor de texto read-only a la derecha.
                None => open_in_preview(m, path, pane),
            }
        }
        otra => open_in_preview(m, path, otra),
    }
}

/// Deja `pane` como contenido del sidebar derecho de preview (y lo abre).
pub(crate) fn open_in_preview(m: &mut Model, path: &PathBuf, pane: PreviewPane) {
    m.preview = pane;
    m.basemap = open_basemap_if_pmtiles(path);
    m.basemap_dirty = m.basemap.is_some();
    m.preview_of = Some(path.clone());
    m.preview_temp = None;
    m.map_view.reset();
    m.map_view.color_field = None;
    m.viewer_open = true;
}

/// Releé el preview del nodo seleccionado en el navegador activo (POSIX o
/// fuente montada). Contenedor (o nada) → limpia. Hoja POSIX (id = ruta real)
/// → carga directa con `load_for`. Hoja no-POSIX → vuelca a tempfile y
/// previsualiza. Unifica los dos caminos viejos (POSIX y `*_nav`).
pub(crate) fn refresh_preview(m: &mut Model) {
    // Con el visor cerrado no hay nada que refrescar — y cargar/decodificar
    // en cada flecha sería I/O tirado. El visor carga fresco al abrirse
    // (OpenSelected / doble click).
    if !m.viewer_open {
        return;
    }
    // Resolvemos la acción soltando el préstamo de `cur()` antes de mutar el
    // preview (que toca el resto del modelo).
    enum Accion {
        Limpiar,
        Posix(PathBuf),
        Bytes(Vec<u8>, String),
    }
    let accion = match m.cur().selected_node() {
        Some(n) if !n.is_container => {
            let p = Path::new(&n.id);
            if p.is_file() {
                Accion::Posix(p.to_path_buf())
            } else {
                match m.cur().read(&n.id) {
                    Ok(bytes) => Accion::Bytes(bytes, n.name.clone()),
                    Err(_) => Accion::Limpiar,
                }
            }
        }
        _ => Accion::Limpiar,
    };
    match accion {
        Accion::Limpiar => clear_preview(m),
        Accion::Posix(path) => {
            m.preview = load_for(&path);
            m.basemap = open_basemap_if_pmtiles(&path);
            m.basemap_dirty = m.basemap.is_some();
            m.preview_of = Some(path);
            m.preview_temp = None;
            // Encuadre fresco para el nuevo archivo (si fuera un mapa).
            m.map_view.reset();
            m.map_view.color_field = None;
        }
        Accion::Bytes(bytes, nombre) => preview_from_bytes(m, bytes, &nombre),
    }
}

/// Decide qué viewer usar discerniendo el **contenido** del archivo (no
/// la extensión) y dispara la carga sync. Lee una muestra del header,
/// la pasa por `shuma-discern`, y `viewer_registry::pick` elige el visor.
/// Un .png con la extensión equivocada ahora se abre igual como imagen;
/// un archivo ilegible cae al text viewer (que degrada a "binario").
pub(crate) fn load_for(path: &Path) -> PreviewPane {
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
        ViewerKind::Map => PreviewPane::Map(load_map(path, DEFAULT_MAP_BYTES_MAX)),
        ViewerKind::Text => PreviewPane::Text(load_preview(path, DEFAULT_PREVIEW_BYTES_MAX)),
        // El panel muestra el fuente; el render lo hace puriy al abrir.
        ViewerKind::Web => PreviewPane::Web(load_preview(path, DEFAULT_PREVIEW_BYTES_MAX)),
    }
}

/// Lanza puriy (el navegador de la suite) sobre un archivo HTML local,
/// fuera de proceso, como un file manager abre el visor por defecto. La
/// ruta se entrega como `file://<abs>` (puriy resuelve `file://`). El
/// binario es `puriy`; `$PURIY_BIN` lo override (útil en dev:
/// `PURIY_BIN=target/debug/puriy`). Un fallo al spawnear se reporta a
/// stderr y no interrumpe el shell.
pub(crate) fn launch_puriy(path: &Path) {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let url = format!("file://{}", abs.display());
    let bin = std::env::var("PURIY_BIN").unwrap_or_else(|_| "puriy".to_string());
    match std::process::Command::new(&bin).arg(&url).spawn() {
        Ok(_) => {}
        Err(e) => eprintln!("[nahual] no pude lanzar puriy ({bin}) sobre {url}: {e}"),
    }
}

/// Abre un archivo de video con el constructor adecuado del visor. El
/// contenido ya se discernió como video; acá la extensión sólo decide
/// el *demuxer*: WebM/MKV (EBML) van por `media-source-webm`, el resto
/// (incluido `.ivf`) por el path AV1 crudo. Si la extensión miente, el
/// visor cae a estado de error y lo muestra en su header.
pub(crate) fn open_video(path: &Path) -> VideoViewerState {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("webm" | "mkv") => VideoViewerState::open_webm(path),
        Some("gif") => VideoViewerState::open_gif(path),
        // MP4/MOV: el discernimiento los reconoce (ftyp) pero no hay decoder
        // nativo H.264/H.265. Con la feature `ffmpeg` se reproducen vía el
        // puente `foreign-av` (subprocess); sin ella, o si ffmpeg no está en
        // PATH, caen a un estado de error claro en vez de un volcado binario.
        Some("mp4" | "m4v" | "mov") => open_ffmpeg_video(path),
        _ => VideoViewerState::open_av1(path),
    }
}

/// Camino para contenedores con códecs ajenos (H.264/H.265…): construye una
/// fuente `foreign-av` (que lanza `ffmpeg` por subprocess, regla #4) y la pasa
/// al viewer por el seam `from_source`. Si `foreign-av` no está compilado
/// (build `--no-default-features`) o ffmpeg falla/no está en PATH, devuelve el
/// estado de error explícito de siempre.
#[cfg(feature = "ffmpeg")]
fn open_ffmpeg_video(path: &Path) -> VideoViewerState {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let source = foreign_av::probe(path).and_then(|info| {
        let duration = info.duration;
        let dims = info.video.map(|v| (v.width, v.height)).unwrap_or((0, 0));
        let session = foreign_av::MediaSession::open(info)?;
        let src = foreign_av::FfmpegVideoSource::from_session(session)?;
        let fps = src.fps();
        Ok((src, dims, duration, fps))
    });
    match source {
        Ok((src, (w, h), duration, fps)) => {
            // El `tick` de la fuente ffmpeg hace un read BLOQUEANTE del pipe;
            // correrlo en el hilo de UI lo cuelga. La envolvemos en un
            // ThreadedFrameSource: un hilo decodifica a cadencia real y el
            // tick del viewer sólo levanta el último frame, sin bloquear.
            let threaded = ThreadedFrameSource::spawn(Box::new(src), fps);
            VideoViewerState::from_source(Box::new(threaded), name, w, h, Some(duration))
        }
        Err(e) => VideoViewerState::unsupported(
            path,
            rimay_localize::t_args("nahual-shell-ffmpeg-failed", &[("err", e.to_string().into())]),
        ),
    }
}

/// Adapta una [`FrameSource`](media_core::FrameSource) de IO **bloqueante**
/// (el pipe de ffmpeg en `foreign-av`) a un `tick` **no-bloqueante**, apto para
/// el hilo de UI. Un hilo dedicado corre la fuente a cadencia real (sleep por
/// frame) y deja el último frame decodificado en un slot compartido
/// (*latest-wins*: si la UI va más lenta que el video, descarta los
/// intermedios). El `tick` del adaptador sólo levanta ese slot — nunca bloquea.
///
/// Limitación conocida: el hilo decodifica de corrido aunque el viewer esté en
/// pausa (la pausa del viewer no se propaga a la fuente), así que al reanudar el
/// video "saltó" a la posición actual. Aceptable para una preview; un control
/// de pausa real es trabajo aparte.
#[cfg(feature = "ffmpeg")]
struct ThreadedFrameSource {
    /// (rgba, w, h, seq). `seq` monótono: el viewer detecta frame nuevo.
    slot: std::sync::Arc<std::sync::Mutex<Option<(Vec<u8>, u32, u32, u64)>>>,
    last_seq: u64,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(feature = "ffmpeg")]
impl ThreadedFrameSource {
    fn spawn(mut inner: Box<dyn media_core::FrameSource + Send>, fps: f32) -> Self {
        use std::sync::atomic::Ordering;
        let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let slot_t = slot.clone();
        let stop_t = stop.clone();
        let interval = Duration::from_secs_f32(1.0 / fps.max(1.0));
        let handle = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut seq = 0u64;
            while !stop_t.load(Ordering::Relaxed) {
                match inner.tick(interval, &mut buf) {
                    Some((w, h)) => {
                        seq += 1;
                        if let Ok(mut g) = slot_t.lock() {
                            *g = Some((buf.clone(), w, h, seq));
                        }
                    }
                    // `None` de la fuente ffmpeg = EOF (el read falló): no hay
                    // más frames, el hilo termina y suelta la fuente (Drop de
                    // MediaSession mata ffmpeg).
                    None => break,
                }
                std::thread::sleep(interval);
            }
        });
        Self {
            slot,
            last_seq: 0,
            stop,
            _handle: Some(handle),
        }
    }
}

#[cfg(feature = "ffmpeg")]
impl media_core::FrameSource for ThreadedFrameSource {
    fn tick(&mut self, _dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        let g = self.slot.lock().ok()?;
        match g.as_ref() {
            Some((rgba, w, h, seq)) if *seq != self.last_seq => {
                self.last_seq = *seq;
                if buf.len() != rgba.len() {
                    buf.resize(rgba.len(), 0);
                }
                buf.copy_from_slice(rgba);
                Some((*w, *h))
            }
            _ => None,
        }
    }
}

#[cfg(feature = "ffmpeg")]
impl Drop for ThreadedFrameSource {
    fn drop(&mut self) {
        // Señalamos parada; no hacemos join (el hilo puede estar en un read
        // bloqueante del pipe), así no trabamos el cierre de la UI. El hilo ve
        // el flag tras el frame en curso, sale y suelta la fuente → ffmpeg
        // muere por el Drop de MediaSession.
        self.stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(not(feature = "ffmpeg"))]
fn open_ffmpeg_video(path: &Path) -> VideoViewerState {
    VideoViewerState::unsupported(
        path,
        rimay_localize::t("nahual-shell-no-ffmpeg"),
    )
}

/// Cuántos bytes del header alcanzan a `shuma-discern`. Los magic-bytes y
/// el arranque de JSON/TOML viven en los primeros KB; no hace falta leer
/// el archivo entero sólo para elegir visor.
pub(crate) const DISCERN_SAMPLE_BYTES: usize = 8 * 1024;

/// Lee hasta `max` bytes del inicio del archivo para discernir su tipo.
/// `None` si no se puede abrir/leer — el caller lo trata como "sin
/// discernimiento" y cae al text viewer.
pub(crate) fn read_header_sample(path: &Path, max: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

#[cfg(all(test, feature = "ffmpeg"))]
mod ffmpeg_tests {
    use super::*;

    /// Smoke end-to-end del puente foreign-av: forja un MP4/H.264 real con
    /// `ffmpeg` (testsrc, 1 s) y verifica que `open_video` produzca una fuente
    /// viva que decodifique al menos un frame. `#[ignore]` porque depende del
    /// binario `ffmpeg` en PATH — correr con `--ignored`. Si ffmpeg no genera
    /// el archivo (no está instalado), el test se salta solo.
    #[test]
    #[ignore = "requiere ffmpeg en PATH"]
    fn mp4_h264_reproduce_via_foreign_av() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mp4 = dir.path().join("smoke.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc=size=64x48:rate=10:duration=1")
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&mp4)
            .status();
        match status {
            Ok(s) if s.success() && mp4.exists() => {}
            _ => return, // ffmpeg ausente o sin libx264: nada que probar.
        }

        let mut st = open_video(&mp4);
        assert!(
            st.dimensions().0 > 0,
            "el probe debería haber dado dimensiones de video"
        );

        // El frame lo produce el hilo del ThreadedFrameSource a cadencia real;
        // el tick del viewer es no-bloqueante, así que dormimos de verdad entre
        // intentos para darle tiempo al subprocess+hilo a entregar el primero.
        let mut got = false;
        for _ in 0..120 {
            if st.tick(Duration::from_millis(50)) {
                got = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(got, "foreign-av debería decodificar al menos un frame de H.264");
    }
}
