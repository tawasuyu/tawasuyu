//! `nahual-module` — el front universal de nahual como **módulo hospedable**.
//!
//! `nahual-shell` es una app con ventana propia (un `App` del bucle Elm). Este
//! crate expone el **mismo motor y las mismas acciones** como un módulo que un
//! chasis (pata, shuma, …) monta dentro de un panel —igual que
//! `shuma-module-shell`—: un [`State`], un [`Msg`], un [`view`] genérico sobre
//! el `Msg` del host (vía un `lift`), y un [`update`] **puro** que devuelve
//! [`Effect`]s para que el host ejecute el trabajo asíncrono con su `Handle`
//! (generar una miniatura, lanzar una app). El host nunca toca los campos del
//! `State`: le rutea eventos y pinta su `view`.
//!
//! Es un **frontend intercambiable sobre `nahual-source-core`** (regla 2 del
//! repo): toda la navegación —POSIX, Mónadas del daemon vivo, imágenes wawa,
//! archivos `.zip`— vive en el `Navigator`; este crate sólo lo pinta y traduce
//! eventos. Por eso convive con `nahual-shell` sin duplicar lógica de dominio.
//!
//! Cubre navegación (árbol/lista/detalle/iconos + breadcrumb + filtro),
//! miniaturas async, abrir con la app por defecto y "abrir con…" hacia la
//! suite. Las operaciones de archivo (crear/borrar/renombrar) son v2 —piden la
//! cola + prompts del shell.

#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use app_bus::AppRegistry;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::{
    Blob, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
};
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_breadcrumb::{breadcrumb_view, BreadcrumbPalette};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_detail_table::{
    detail_table_view, Column, DetailPalette, DetailRow, DetailSpec, SortDir as DtDir,
};
use llimphi_widget_grid::{grid_view, ventana_visible, GridCell, GridMetrics, GridPalette, GridSpec};
use llimphi_widget_list::{list_view, ListPalette, ListRow, ListSpec};
use nahual_source_core::{
    ArchiveSource, Navigator, Node, NodeId, NodeKind, Opened, PosixSource, SortKey, SortDir,
    Source, ViewMode, WawaImgSource,
};
use nahual_thumb_core::{generar_thumb_de_archivo, ThumbRgba};

/// Lado máximo (px) de las miniaturas de la vista iconos.
pub const THUMB_LADO: u32 = 128;
/// Tope de miniaturas pedidas por pasada (acota los spawns del host).
const MAX_ICON_TILES: usize = 160;

/// Trabajo asíncrono o con efectos que el **host** debe ejecutar (tiene el
/// `Handle`; el módulo es puro). El host hace el spawn/launch y, para las
/// miniaturas, realimenta [`Msg::ThumbReady`]/[`Msg::ThumbFailed`].
#[derive(Debug, Clone)]
pub enum Effect {
    /// Generar la miniatura de este archivo POSIX y devolverla por
    /// `Msg::ThumbReady(path, thumb)`.
    GenThumb(PathBuf),
    /// Abrir `path` con la app por defecto de su tipo (doble-clic / Enter sobre
    /// una hoja que no se monta).
    OpenDefault(PathBuf),
    /// Abrir `path` con la app `app_id` de la suite ("abrir con…").
    Launch { app_id: String, path: PathBuf },
}

/// Menú "abrir con…" abierto sobre una hoja: ancla en coords de panel +
/// opciones `(app_id, label)` ya resueltas, + el path objetivo.
#[derive(Clone)]
struct MenuData {
    at: (f32, f32),
    options: Vec<(String, String)>,
    target: PathBuf,
}

/// Estado del módulo. El host lo guarda en su modelo (`inner`) y nunca lo muta
/// directo — sólo vía [`update`].
pub struct State {
    /// Pila de montaje: `[0]` = fuente base; montar empuja, desmontar saca.
    nav_stack: Vec<Navigator>,
    /// Selección múltiple por id (marca con la barra espaciadora / Insert).
    marked: BTreeSet<NodeId>,
    /// Cache RAM de miniaturas listas para pintar (clave = ruta POSIX).
    thumbs: HashMap<PathBuf, Image>,
    thumbs_pending: HashSet<PathBuf>,
    thumbs_failed: HashSet<PathBuf>,
    /// Catálogo de apps de la suite (open-with).
    registry: AppRegistry,
    /// `true` mientras se teclea el filtro vivo.
    filtering: bool,
    /// Menú "abrir con…" abierto, si hay.
    menu: Option<MenuData>,
}

impl State {
    /// Monta el módulo sobre una fuente cualquiera (POSIX, daemon, …).
    pub fn on_source(source: Box<dyn Source>) -> std::io::Result<Self> {
        Ok(Self::from_nav(Navigator::open(source)?))
    }

    /// Monta el módulo sobre el filesystem POSIX, parado en `cwd` con la miga
    /// de ancestros completa (raíz anclada en `/`).
    pub fn posix(cwd: &Path) -> Self {
        Self::from_nav(posix_nav(cwd))
    }

    /// Monta el módulo sobre las **Mónadas del daemon vivo** de nouser
    /// (descubre el socket por el broker → fallback). El gancho que reemplaza
    /// al `nouser.rs` bespoke de pata.
    pub fn nouser_daemon() -> std::io::Result<Self> {
        let src = nahual_source_core::NouserDaemonSource::discover()?;
        Self::on_source(Box::new(src))
    }

    fn from_nav(nav: Navigator) -> Self {
        Self {
            nav_stack: vec![nav],
            marked: BTreeSet::new(),
            thumbs: HashMap::new(),
            thumbs_pending: HashSet::new(),
            thumbs_failed: HashSet::new(),
            registry: AppRegistry::with_defaults(),
            filtering: false,
            menu: None,
        }
    }

    fn cur(&self) -> &Navigator {
        self.nav_stack.last().expect("nav_stack nunca vacía")
    }

    fn cur_mut(&mut self) -> &mut Navigator {
        self.nav_stack.last_mut().expect("nav_stack nunca vacía")
    }

    /// `true` si hay una fuente no-POSIX montada (pila > 1).
    pub fn is_foreign(&self) -> bool {
        self.nav_stack.len() > 1
    }

    /// La ruta POSIX del nodo seleccionado, si su id ES una ruta real (POSIX o
    /// archivo miembro de una Mónada del daemon). `None` para hojas sintéticas.
    fn selected_path(&self) -> Option<PathBuf> {
        let n = self.cur().selected_node()?;
        if n.is_container {
            return None;
        }
        let p = PathBuf::from(&n.id);
        p.is_file().then_some(p)
    }
}

/// Mensajes del módulo. El host los envuelve con su `lift` al construir la
/// `view`, y se los reenvía a [`update`] cuando llegan.
#[derive(Debug, Clone)]
pub enum Msg {
    Up,
    Down,
    /// Selecciona la fila `idx` (índice absoluto en los hijos).
    Select(usize),
    /// Abre la selección: contenedor → desciende; hoja montable → monta; resto
    /// → `Effect::OpenDefault`.
    Open,
    /// Sube al contenedor padre (o desmonta si está en la raíz de una fuente).
    Parent,
    /// Sube al nivel `depth` del breadcrumb.
    BreadcrumbTo(usize),
    /// Cicla lista → detalle → iconos.
    ToggleView,
    /// Rueda: +abajo / −arriba (líneas).
    Scroll(i32),
    /// Marca/desmarca la fila bajo el cursor.
    ToggleMark,
    FilterStart,
    FilterInput(String),
    FilterBackspace,
    FilterEnd,
    /// Ordena por la columna `col` (0 nombre · 1 tamaño · 2 fecha · 3 tipo).
    SortBy(usize),
    /// Abre el menú "abrir con…" sobre la hoja seleccionada, anclado en `(x,y)`.
    OpenContextAt(f32, f32),
    /// Elige una app del menú "abrir con…".
    OpenWith(String),
    /// Cierra el menú "abrir con…".
    CloseMenu,
    /// Una miniatura terminó (la realimenta el host).
    ThumbReady(PathBuf, ThumbRgba),
    /// La miniatura de este path falló.
    ThumbFailed(PathBuf),
}

/// Aplica `msg` a `state` y devuelve los [`Effect`]s que el host debe ejecutar.
/// **Puro**: no spawnea ni toca el `Handle`.
pub fn update(mut state: State, msg: Msg) -> (State, Vec<Effect>) {
    let mut fx = Vec::new();
    match msg {
        Msg::Up => {
            state.cur_mut().up();
        }
        Msg::Down => {
            state.cur_mut().down();
        }
        Msg::Select(idx) => {
            state.cur_mut().select(idx);
        }
        Msg::Open => {
            match state.cur_mut().open_selected() {
                Ok(Some(Opened::Descended)) => {
                    state.marked.clear();
                    request_thumbs(&mut state, &mut fx);
                }
                Ok(Some(Opened::Leaf(id))) => {
                    let path = Path::new(&id);
                    if path.is_file() {
                        // Montable (.img wawa / .zip|.tar) → empuja; si no, abre
                        // con la app por defecto.
                        if let Some(nav) = try_mount(path) {
                            state.nav_stack.push(nav);
                            state.marked.clear();
                            request_thumbs(&mut state, &mut fx);
                        } else {
                            fx.push(Effect::OpenDefault(path.to_path_buf()));
                        }
                    } else {
                        // Hoja no-POSIX sin ruta real: nada que lanzar (v2:
                        // materializar a tempfile como hace el shell).
                    }
                }
                Ok(None) | Err(_) => {}
            }
        }
        Msg::Parent => match state.cur_mut().parent() {
            Ok(true) => request_thumbs(&mut state, &mut fx),
            Ok(false) => {
                if state.is_foreign() {
                    state.nav_stack.pop();
                    request_thumbs(&mut state, &mut fx);
                }
            }
            Err(_) => {}
        },
        Msg::BreadcrumbTo(depth) => {
            if state.cur_mut().ascend_to(depth).is_ok() {
                request_thumbs(&mut state, &mut fx);
            }
        }
        Msg::ToggleView => {
            let v = state.cur().view.next();
            state.cur_mut().view = v;
            if v == ViewMode::Icons {
                request_thumbs(&mut state, &mut fx);
            }
        }
        Msg::Scroll(steps) => {
            state.cur_mut().scroll(steps);
            if state.cur().view == ViewMode::Icons {
                request_thumbs(&mut state, &mut fx);
            }
        }
        Msg::ToggleMark => {
            if let Some(n) = state.cur().selected_node() {
                let id = n.id.clone();
                if !state.marked.insert(id.clone()) {
                    state.marked.remove(&id);
                }
                state.cur_mut().down();
            }
        }
        Msg::FilterStart => state.filtering = true,
        Msg::FilterInput(s) => {
            let mut f = state.cur().filter().to_string();
            f.push_str(&s);
            state.cur_mut().set_filter(f);
        }
        Msg::FilterBackspace => {
            let mut f = state.cur().filter().to_string();
            f.pop();
            state.cur_mut().set_filter(f);
        }
        Msg::FilterEnd => state.filtering = false,
        Msg::SortBy(col) => state.cur_mut().set_sort(col_to_sortkey(col)),
        Msg::OpenContextAt(x, y) => {
            if let Some(path) = state.selected_path() {
                let mime = mime_for(&path);
                let options: Vec<(String, String)> = state
                    .registry
                    .handlers_for(&mime)
                    .into_iter()
                    .map(|e| (e.id.clone(), e.label.clone()))
                    .collect();
                state.menu = Some(MenuData { at: (x, y), options, target: path });
            }
        }
        Msg::OpenWith(app_id) => {
            if let Some(menu) = state.menu.take() {
                fx.push(Effect::Launch { app_id, path: menu.target });
            }
        }
        Msg::CloseMenu => state.menu = None,
        Msg::ThumbReady(path, thumb) => {
            state.thumbs_pending.remove(&path);
            let img = Image::new(ImageData {
                data: Blob::from(thumb.rgba),
                format: ImageFormat::Rgba8,
                alpha_type: ImageAlphaType::Alpha,
                width: thumb.w,
                height: thumb.h,
            });
            state.thumbs.insert(path, img);
        }
        Msg::ThumbFailed(path) => {
            state.thumbs_pending.remove(&path);
            state.thumbs_failed.insert(path);
        }
    }
    (state, fx)
}

/// Encola `Effect::GenThumb` para las imágenes visibles aún sin miniatura
/// (sólo en vista iconos sobre POSIX). El host las spawnea.
fn request_thumbs(state: &mut State, fx: &mut Vec<Effect>) {
    if state.is_foreign() || state.cur().view != ViewMode::Icons {
        return;
    }
    let pedir: Vec<PathBuf> = {
        let nav = state.cur();
        let visibles = nav.visible();
        let start = nav.visible_offset.min(visibles.len());
        let end = (start + MAX_ICON_TILES).min(visibles.len());
        visibles[start..end]
            .iter()
            .filter(|(_, n)| !n.is_container)
            .map(|(_, n)| PathBuf::from(&n.id))
            .filter(|p| {
                es_imagen(p)
                    && p.is_file()
                    && !state.thumbs.contains_key(p)
                    && !state.thumbs_pending.contains(p)
                    && !state.thumbs_failed.contains(p)
            })
            .collect()
    };
    for p in pedir {
        state.thumbs_pending.insert(p.clone());
        fx.push(Effect::GenThumb(p));
    }
}

/// Conveniencia para el host: ejecuta un [`Effect::GenThumb`] (corre en el
/// worker del host) y arma el `Msg` de vuelta. Centraliza la cadena
/// decode→`ThumbRgba` para que el chasis no la repita.
pub fn run_gen_thumb(path: PathBuf) -> Msg {
    match generar_thumb_de_archivo(&path, THUMB_LADO) {
        Ok(t) => Msg::ThumbReady(path, t),
        Err(_) => Msg::ThumbFailed(path),
    }
}

// =====================================================================
// View
// =====================================================================

/// Pinta el módulo: breadcrumb + lista/detalle/iconos. Genérico sobre el `Msg`
/// del host vía `lift`. El menú contextual "abrir con…" va aparte en
/// [`context_overlay`] (es un overlay absoluto que el host posiciona).
pub fn view<H: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> H + Clone + 'static,
) -> View<H> {
    let crumb = breadcrumb::<H>(state, theme, lift.clone());
    let body = match state.cur().view {
        ViewMode::List => list_panel::<H>(state, theme, lift.clone()),
        ViewMode::Details => detail_panel::<H>(state, theme, lift.clone()),
        ViewMode::Icons => icons_panel::<H>(state, theme, lift),
    };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![crumb, body])
}

/// El menú "abrir con…" como overlay absoluto, si está abierto. El host lo
/// apila por encima de su chrome y le pasa su `viewport` (para que el menú no
/// se salga de pantalla). Devuelve `None` si no hay menú.
pub fn context_overlay<H: Clone + Send + Sync + 'static>(
    state: &State,
    theme: &Theme,
    viewport: (f32, f32),
    lift: impl Fn(Msg) -> H + Clone + Send + Sync + 'static,
) -> Option<View<H>> {
    use std::sync::Arc;
    let menu = state.menu.as_ref()?;
    // Items + el Msg paralelo por índice (el on_pick mapea índice → Msg).
    let (items, msgs): (Vec<ContextMenuItem>, Vec<H>) = if menu.options.is_empty() {
        (
            vec![ContextMenuItem::action("(sin apps para este tipo)").disabled()],
            vec![lift(Msg::CloseMenu)],
        )
    } else {
        menu.options
            .iter()
            .map(|(id, label)| {
                (ContextMenuItem::action(format!("Abrir con {label}")), lift(Msg::OpenWith(id.clone())))
            })
            .unzip()
    };
    let dismiss = lift(Msg::CloseMenu);
    let on_pick: Arc<dyn Fn(usize) -> H + Send + Sync> =
        Arc::new(move |i: usize| msgs.get(i).cloned().unwrap_or_else(|| dismiss.clone()));
    Some(context_menu_view(ContextMenuSpec {
        anchor: menu.at,
        viewport,
        header: Some("Abrir con…".to_string()),
        items,
        active: usize::MAX,
        on_pick,
        on_dismiss: lift(Msg::CloseMenu),
        palette: ContextMenuPalette::from_theme(theme),
    }))
}

fn breadcrumb<H: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> H + Clone + 'static,
) -> View<H> {
    let mut segs: Vec<String> = state.cur().ancestors().iter().map(|n| n.name.clone()).collect();
    if state.is_foreign() && !segs.is_empty() {
        segs[0] = format!("⊟ {}", state.cur().label());
    }
    let refs: Vec<&str> = segs.iter().map(String::as_str).collect();
    let crumbs = breadcrumb_view(&refs, move |d| lift(Msg::BreadcrumbTo(d)), &BreadcrumbPalette::from_theme(theme));
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: pad_h(12.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![crumbs])
}

fn caption(state: &State) -> String {
    let nav = state.cur();
    let f = nav.filter();
    if state.filtering || !f.is_empty() {
        let cur = if state.filtering { "_" } else { "" };
        format!("{} de {} · filtro: {f}{cur}", nav.visible_count(), nav.children().len())
    } else {
        format!("{} entradas · ↑↓ · ⏎ abre · ⌫ vuelve · v vista · / filtra", nav.children().len())
    }
}

fn list_panel<H: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> H + Clone + 'static,
) -> View<H> {
    let nav = state.cur();
    let visibles = nav.visible();
    let start = nav.visible_offset.min(visibles.len());
    let end = (start + nav.visible_rows).min(visibles.len());
    let rows: Vec<ListRow<H>> = visibles[start..end]
        .iter()
        .map(|(idx, n)| {
            let mark = if state.marked.contains(&n.id) { "✓" } else { " " };
            let icon = if n.is_container { "▸ " } else { "  " };
            let label = if n.is_container {
                format!("{mark}{icon}{}/", n.name)
            } else {
                format!("{mark}{icon}{}", n.name)
            };
            let i = *idx;
            ListRow { label, selected: *idx == nav.selected, on_click: lift(Msg::Select(i)) }
        })
        .collect();
    let truncated_hint =
        (visibles.len() > end).then(|| format!("… y {} más", visibles.len() - end));
    list_view(ListSpec {
        rows,
        total: visibles.len(),
        caption: Some(caption(state)),
        truncated_hint,
        row_height: 22.0,
        palette: ListPalette::from_theme(theme),
    })
}

fn detail_panel<H: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> H + Clone + 'static,
) -> View<H> {
    let nav = state.cur();
    let (skey, sdir) = nav.sort();
    let sort_col = sortkey_to_col(skey);
    let dir = if matches!(sdir, SortDir::Asc) { DtDir::Asc } else { DtDir::Desc };
    let visibles = nav.visible();
    let start = nav.visible_offset.min(visibles.len());
    let end = (start + nav.visible_rows).min(visibles.len());
    let rows: Vec<DetailRow<H>> = visibles[start..end]
        .iter()
        .map(|(idx, n)| {
            let mark = if state.marked.contains(&n.id) { "✓ " } else { "  " };
            let name = if n.is_container { format!("{mark}{}/", n.name) } else { format!("{mark}{}", n.name) };
            DetailRow {
                cells: vec![name, human_size(n.size), human_mtime(n.mtime), kind_label(n.kind).to_string()],
                selected: *idx == nav.selected,
                accent: None,
                on_click: lift(Msg::Select(*idx)),
            }
        })
        .collect();
    let columns = [
        Column::flex("Nombre", 1.0),
        Column::fixed("Tamaño", 88.0).right(),
        Column::fixed("Modificado", 140.0),
        Column::fixed("Tipo", 84.0),
    ];
    let lift2 = lift.clone();
    detail_table_view(
        DetailSpec {
            columns: &columns,
            rows,
            sort: Some((sort_col, dir)),
            row_height: 22.0,
            caption: Some(caption(state)),
            palette: DetailPalette::from_theme(theme),
        },
        move |col| lift2(Msg::SortBy(col)),
    )
}

fn icons_panel<H: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> H + Clone + 'static,
) -> View<H> {
    let nav = state.cur();
    let metrics = GridMetrics::default();
    let total = nav.visible_count();
    // Sin dims del panel: estimamos 4 columnas (el host puede re-derivar luego).
    let win = ventana_visible(total, metrics.tile_w * 4.2, 600.0, 0, &metrics);
    let visibles = nav.visible();
    let start = nav.visible_offset.min(visibles.len());
    let end = (start + MAX_ICON_TILES).min(visibles.len());
    let cells: Vec<GridCell<H>> = visibles[start..end]
        .iter()
        .map(|(idx, n)| {
            let mark = if state.marked.contains(&n.id) { "✓ " } else { "" };
            GridCell {
                content: tile_content::<H>(state, n, theme, metrics.tile_w - 12.0),
                label: Some(format!("{mark}{}", n.name)),
                selected: *idx == nav.selected,
                on_click: lift(Msg::Select(*idx)),
            }
        })
        .collect();
    let mostrados = start + cells.len();
    let truncated_hint = (mostrados < total).then(|| format!("… y {} más", total - mostrados));
    grid_view(GridSpec {
        cells,
        cols: win.cols,
        metrics,
        caption: Some(caption(state)),
        truncated_hint,
        palette: GridPalette::from_theme(theme),
    })
}

fn tile_content<H: Clone + 'static>(state: &State, node: &Node, theme: &Theme, lado: f32) -> View<H> {
    let base = || Style {
        size: Size { width: length(lado), height: length(lado) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    };
    if node.is_container {
        let g = match node.kind {
            NodeKind::Archive => "▤",
            NodeKind::Synthetic => "◈",
            _ => "▣",
        };
        return View::new(base()).fill(theme.bg_panel_alt).text(g, 44.0, theme.fg_text);
    }
    let path = PathBuf::from(&node.id);
    if let Some(img) = state.thumbs.get(&path) {
        return View::new(base()).image(img.clone());
    }
    if state.thumbs_failed.contains(&path) {
        return View::new(base()).fill(theme.bg_panel_alt).text("⚠", 24.0, theme.fg_muted);
    }
    let g = if es_imagen(&path) { "▨" } else { "▢" };
    View::new(base()).fill(theme.bg_panel_alt).text(g, 36.0, theme.fg_muted)
}

/// Traducción opcional de un evento de teclado a un [`Msg`]. El host puede
/// usarla cuando el módulo tiene el foco; devuelve `None` si la tecla no le
/// concierne (el host la procesa).
pub fn on_key(state: &State, e: &KeyEvent) -> Option<Msg> {
    if e.state != KeyState::Pressed {
        return None;
    }
    if state.filtering {
        return match &e.key {
            Key::Named(NamedKey::Escape) | Key::Named(NamedKey::Enter) => Some(Msg::FilterEnd),
            Key::Named(NamedKey::Backspace) => Some(Msg::FilterBackspace),
            Key::Character(s) => Some(Msg::FilterInput(s.to_string())),
            _ => None,
        };
    }
    match &e.key {
        Key::Named(NamedKey::ArrowUp) => Some(Msg::Up),
        Key::Named(NamedKey::ArrowDown) => Some(Msg::Down),
        Key::Named(NamedKey::Enter) => Some(Msg::Open),
        Key::Named(NamedKey::Backspace) => Some(Msg::Parent),
        Key::Named(NamedKey::Space) => Some(Msg::ToggleMark),
        Key::Character(s) if s == "v" => Some(Msg::ToggleView),
        Key::Character(s) if s == "/" => Some(Msg::FilterStart),
        _ => None,
    }
}

// =====================================================================
// Helpers
// =====================================================================

fn posix_nav(cwd: &Path) -> Navigator {
    use std::path::Component;
    let mut stack = vec![Node::new("/", "/", true).with_kind(NodeKind::Dir)];
    let mut acc = PathBuf::from("/");
    for comp in cwd.components() {
        if let Component::Normal(c) = comp {
            acc.push(c);
            stack.push(
                Node::new(acc.to_string_lossy().into_owned(), c.to_string_lossy().into_owned(), true)
                    .with_kind(NodeKind::Dir),
            );
        }
    }
    Navigator::open_at(Box::new(PosixSource::new("/")), stack)
        .or_else(|_| Navigator::open(Box::new(PosixSource::new("/"))))
        .expect("la raíz / siempre se puede listar")
}

fn try_mount(path: &Path) -> Option<Navigator> {
    if let Ok(src) = WawaImgSource::abrir(path) {
        return Navigator::open(Box::new(src)).ok();
    }
    if ArchiveSource::es_archivo(path) {
        if let Ok(src) = ArchiveSource::abrir(path) {
            return Navigator::open(Box::new(src)).ok();
        }
    }
    None
}

fn es_imagen(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" | "tif" | "ico" | "avif" | "qoi" | "tga")
    )
}

/// MIME mínimo por extensión para rankear handlers en "abrir con…". No es
/// `shuma-discern` (eso vive en el shell): un mapa chico alcanza para el menú.
fn mime_for(path: &Path) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" | "tif" | "ico" | "avif" => {
            format!("image/{}", if ext == "jpg" { "jpeg" } else { ext.as_str() })
        }
        "mp3" | "flac" | "ogg" | "wav" | "opus" | "m4a" => format!("audio/{ext}"),
        "mp4" | "mkv" | "webm" | "mov" | "avi" => format!("video/{ext}"),
        "md" | "markdown" => "text/markdown".to_string(),
        "html" | "htm" => "text/html".to_string(),
        "csv" => "text/csv".to_string(),
        "" => "application/octet-stream".to_string(),
        other => format!("text/x-{other}"),
    }
}

fn col_to_sortkey(col: usize) -> SortKey {
    match col {
        1 => SortKey::Size,
        2 => SortKey::Mtime,
        3 => SortKey::Kind,
        _ => SortKey::Name,
    }
}

fn sortkey_to_col(key: SortKey) -> usize {
    match key {
        SortKey::Name => 0,
        SortKey::Size => 1,
        SortKey::Mtime => 2,
        SortKey::Kind => 3,
    }
}

fn kind_label(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Dir => "carpeta",
        NodeKind::File => "archivo",
        NodeKind::Symlink => "enlace",
        NodeKind::Archive => "archivo comp.",
        NodeKind::Synthetic => "—",
    }
}

fn human_size(size: Option<u64>) -> String {
    let Some(n) = size else { return "—".into() };
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if n >= GIB {
        format!("{:.1} GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else if n >= KIB {
        format!("{:.1} KiB", n as f64 / KIB as f64)
    } else {
        format!("{n} B")
    }
}

fn human_mtime(mtime_ms: Option<u64>) -> String {
    let Some(ms) = mtime_ms else { return "—".into() };
    // Fecha civil sin deps (UTC): suficiente para la columna.
    let secs = (ms / 1000) as i64;
    let days = secs.div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Algoritmo de Howard Hinnant: días desde epoch → (año, mes, día). Sin deps.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn pad_h(v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect { left: length(v), right: length(v), top: length(0.0), bottom: length(0.0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn arbol() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("a.txt"), b"hola").unwrap();
        fs::write(dir.path().join("sub/b.txt"), b"chau").unwrap();
        dir
    }

    #[test]
    fn navega_posix_y_abre_default() {
        let dir = arbol();
        let mut st = State::posix(dir.path());
        // Seleccionar el archivo a.txt y abrir → Effect::OpenDefault.
        let idx = st.cur().children().iter().position(|n| n.name == "a.txt").unwrap();
        st = update(st, Msg::Select(idx)).0;
        let (st, fx) = update(st, Msg::Open);
        assert!(matches!(fx.as_slice(), [Effect::OpenDefault(p)] if p.ends_with("a.txt")));
        let _ = st;
    }

    #[test]
    fn descender_y_subir() {
        let dir = arbol();
        let mut st = State::posix(dir.path());
        let idx = st.cur().children().iter().position(|n| n.name == "sub").unwrap();
        st = update(st, Msg::Select(idx)).0;
        st = update(st, Msg::Open).0;
        assert!(st.cur().children().iter().any(|n| n.name == "b.txt"));
        st = update(st, Msg::Parent).0;
        assert!(st.cur().children().iter().any(|n| n.name == "a.txt"));
    }

    #[test]
    fn toggle_view_cicla() {
        let dir = arbol();
        let mut st = State::posix(dir.path());
        assert_eq!(st.cur().view, ViewMode::List);
        st = update(st, Msg::ToggleView).0;
        assert_eq!(st.cur().view, ViewMode::Details);
        st = update(st, Msg::ToggleView).0;
        assert_eq!(st.cur().view, ViewMode::Icons);
    }

    #[test]
    fn open_context_arma_menu_y_open_with_lanza() {
        let dir = arbol();
        let mut st = State::posix(dir.path());
        let idx = st.cur().children().iter().position(|n| n.name == "a.txt").unwrap();
        st = update(st, Msg::Select(idx)).0;
        st = update(st, Msg::OpenContextAt(10.0, 10.0)).0;
        assert!(st.menu.is_some());
        let (st, fx) = update(st, Msg::OpenWith("nada".into()));
        assert!(matches!(fx.as_slice(), [Effect::Launch { app_id, .. }] if app_id == "nada"));
        assert!(st.menu.is_none());
    }

    #[test]
    fn mime_for_basico() {
        assert_eq!(mime_for(Path::new("x.png")), "image/png");
        assert_eq!(mime_for(Path::new("x.jpg")), "image/jpeg");
        assert_eq!(mime_for(Path::new("x.md")), "text/markdown");
    }
}
