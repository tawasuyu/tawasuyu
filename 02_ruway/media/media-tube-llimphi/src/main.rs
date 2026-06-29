//! media-tube — frontend de plataforma de video (tipo FreeTube) del dominio
//! `media`, en Llimphi.
//!
//! Compone, no reimplementa:
//!   - **Descubrir**: [`foreign_platform`] (trait agnóstico + providers REST
//!     data-driven; hoy Invidious y PeerTube). La búsqueda corre en un worker
//!     (`Handle::spawn`) y reentra al `update` con los resultados.
//!   - **Miniaturas**: [`llimphi_image::ImageCache`] (feature `net`) — la
//!     `view` lee del caché en el hilo UI mientras workers lo pueblan por URL.
//!   - **Reproducir**: lanza el binario hermano `media-app <url>`, que ya
//!     resuelve yt-dlp/DASH/ffmpeg (R1/R2 de PARIDAD.md). No duplica el
//!     pipeline de red.
//!
//! MVP feo-primero: header (búsqueda + conmutador de backend) + grilla
//! virtualizada de resultados; click en una tarjeta = reproducir.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use foreign_platform::descriptors;
use foreign_platform::model::{SearchQuery, VideoCard};
use foreign_platform::{PlatformError, PlatformProvider};

use llimphi_image::ImageCache;
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_theme::motion;
use llimphi_ui::{App, Handle, ImageFit, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_icons::Icon;
use llimphi_widget_empty::{empty_view, EmptyPalette};
use llimphi_widget_grid::{grid_view, ventana_visible, GridCell, GridMetrics, GridPalette, GridSpec};
use llimphi_widget_skeleton::{skeleton_view, SkeletonPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use llimphi_widget_toast::{toast_stack_view, Toast};
use rimay_localize::{t, t_args};

/// Alto del header (px).
const HEADER_H: f32 = 56.0;
/// Geometría de la grilla de resultados.
const METRICS: GridMetrics = GridMetrics { tile_w: 232.0, tile_h: 176.0, gap: 14.0, pad: 18.0 };
/// Tamaño de la miniatura dentro de cada celda (16:9).
const THUMB_W: f32 = 200.0;
const THUMB_H: f32 = 112.0;
/// Cap de descarga por miniatura.
const THUMB_CAP: u64 = 4 * 1024 * 1024;
/// Cuánto vive un toast antes de auto-descartarse.
const TOAST_TTL: Duration = Duration::from_secs(4);

/// Hash estable de una cadena → `key` para animaciones implícitas
/// (la misma URL/escena produce siempre la misma key entre rebuilds).
fn key_of(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Qué backend de plataforma está activo.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Backend {
    Invidious,
    PeerTube,
}

impl Backend {
    fn nombre(self) -> &'static str {
        match self {
            Backend::Invidious => "Invidious",
            Backend::PeerTube => "PeerTube",
        }
    }
}

#[derive(Clone)]
enum Msg {
    SearchFocus,
    SearchBlur,
    SearchKey(KeyEvent),
    DoSearch,
    /// Llegó una tanda de videos. `append`=true la suma (paginación);
    /// `false` reemplaza (búsqueda nueva o tendencias). `gen` descarta tardíos.
    Loaded { gen: u64, append: bool, items: Arc<Vec<VideoCard>> },
    LoadFailed(u64, String),
    SetBackend(Backend),
    Play(usize),
    /// Abrir la página de un canal (click en el autor de una tarjeta).
    OpenChannel { id: String, name: String },
    /// Volver del canal al listado anterior (tendencias o búsqueda).
    Back,
    ThumbDone(String),
    Wheel(f32),
    Resize(u32, u32),
    /// Tick de animación — fuerza repaint para el shimmer del skeleton
    /// mientras hay una carga en vuelo. Se auto-rearma sólo si sigue cargando.
    Tick,
    /// Un toast cumplió su `duration`: se descarta del stack.
    ToastExpire(u64),
}

struct Model {
    backend: Backend,
    instance: String,
    search: TextInputState,
    search_focused: bool,
    videos: Vec<VideoCard>,
    status: String,
    gen: u64,
    /// Texto de la búsqueda en curso (vacío = listando tendencias).
    query: String,
    /// Última página cargada (sólo aplica a búsquedas paginadas).
    page: u32,
    /// Hay un fetch en vuelo (evita disparar otro al scrollear).
    loading: bool,
    /// La última página vino vacía: no hay más que pedir.
    exhausted: bool,
    /// Canal abierto (id, nombre) — `Some` ⇒ la grilla muestra sus videos.
    channel: Option<(String, String)>,
    scroll_fila: usize,
    thumbs: ImageCache,
    thumb_pending: HashSet<String>,
    viewport: (f32, f32),
    /// Toasts vivos (errores, confirmaciones de reproducción).
    toasts: Vec<Toast>,
    /// Id incremental para correlacionar toast ↔ Msg de expiración.
    next_toast: u64,
    /// Hay una cadena de `Msg::Tick` en vuelo (evita rearmar dos).
    ticking: bool,
}

/// Primera instancia por defecto del descriptor del backend.
fn default_instance(b: Backend) -> String {
    let d = match b {
        Backend::Invidious => descriptors::invidious_descriptor(),
        Backend::PeerTube => descriptors::peertube_descriptor(),
    };
    d.default_instances.first().cloned().unwrap_or_default()
}

/// Corre una búsqueda paginada construyendo el provider data-driven (worker, red).
fn buscar(b: Backend, instance: &str, q: &str, page: u32) -> Result<Vec<VideoCard>, PlatformError> {
    let query = SearchQuery { text: q.to_string(), page };
    match b {
        Backend::Invidious => descriptors::invidious(instance.to_string()).search(&query),
        Backend::PeerTube => descriptors::peertube(instance.to_string()).search(&query),
    }
}

/// Trae las tendencias/portada de la instancia (worker, red).
fn tendencias(b: Backend, instance: &str) -> Result<Vec<VideoCard>, PlatformError> {
    match b {
        Backend::Invidious => descriptors::invidious(instance.to_string()).trending(),
        Backend::PeerTube => descriptors::peertube(instance.to_string()).trending(),
    }
}

/// Trae los videos de un canal por su id (worker, red).
fn canal(b: Backend, instance: &str, id: &str) -> Result<Vec<VideoCard>, PlatformError> {
    match b {
        Backend::Invidious => descriptors::invidious(instance.to_string()).channel_videos(id),
        Backend::PeerTube => descriptors::peertube(instance.to_string()).channel_videos(id),
    }
}

/// URL absoluta de la miniatura (PeerTube entrega paths relativos a la instancia).
fn thumb_url(instance: &str, card: &VideoCard) -> String {
    match &card.thumbnail {
        Some(t) if t.starts_with("http") => t.clone(),
        Some(t) => format!("{}{}", instance.trim_end_matches('/'), t),
        None => String::new(),
    }
}

/// URL de "ver" que entiende media-app (que corre yt-dlp/ffmpeg por su cuenta).
fn watch_url(b: Backend, instance: &str, id: &str) -> String {
    match b {
        // El id de Invidious es el videoId de YouTube → media-app lo resuelve
        // con yt-dlp (incluye DASH audio+video).
        Backend::Invidious => format!("https://www.youtube.com/watch?v={id}"),
        // PeerTube: la URL corta de la instancia; yt-dlp también la soporta.
        Backend::PeerTube => format!("{}/w/{}", instance.trim_end_matches('/'), id),
    }
}

/// Lanza el binario hermano `media-app` con la URL (fire-and-forget).
fn lanzar_media_app(url: &str) {
    let bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("media-app")))
        .filter(|p| p.exists())
        .map(std::ffi::OsString::from)
        .unwrap_or_else(|| "media-app".into());
    let _ = std::process::Command::new(bin).arg(url).spawn();
}

/// Área útil de la grilla (debajo del header).
fn grid_area(m: &Model) -> (f32, f32) {
    (m.viewport.0, (m.viewport.1 - HEADER_H).max(0.0))
}

/// Dispara la descarga de las miniaturas actualmente visibles que falten.
fn kick_thumbs(m: &mut Model, handle: &Handle<Msg>) {
    if m.videos.is_empty() {
        return;
    }
    let (w, h) = grid_area(m);
    let win = ventana_visible(m.videos.len(), w, h, m.scroll_fila, &METRICS);
    for i in win.first..(win.first + win.count) {
        let url = thumb_url(&m.instance, &m.videos[i]);
        if url.is_empty() || m.thumbs.contains(&url) || m.thumb_pending.contains(&url) {
            continue;
        }
        m.thumb_pending.insert(url.clone());
        let cache = m.thumbs.clone();
        let u = url.clone();
        handle.spawn(move || {
            let _ = cache.get_or_fetch(&u, THUMB_CAP);
            Msg::ThumbDone(u)
        });
    }
}

/// Si la grilla está mostrando la última fila y hay una búsqueda paginable en
/// curso, pide la página siguiente y la encola para *append* (scroll infinito).
fn maybe_load_more(m: &mut Model, handle: &Handle<Msg>) {
    if m.loading || m.exhausted || m.query.is_empty() || m.videos.is_empty() || m.channel.is_some() {
        return;
    }
    let (w, h) = grid_area(m);
    let win = ventana_visible(m.videos.len(), w, h, m.scroll_fila, &METRICS);
    if win.first + win.count < m.videos.len() {
        return; // todavía hay filas por debajo de lo visible
    }
    m.loading = true;
    m.page += 1;
    let (gen, b, inst, q, page) = (m.gen, m.backend, m.instance.clone(), m.query.clone(), m.page);
    handle.spawn(move || match buscar(b, &inst, &q, page) {
        Ok(v) => Msg::Loaded { gen, append: true, items: Arc::new(v) },
        Err(e) => Msg::LoadFailed(gen, e.to_string()),
    });
}

/// Arranca la cadena de ticks de animación si hay una carga en vuelo y no
/// hay ya una corriendo. La cadena se auto-detiene cuando `loading` baja
/// (ver `Msg::Tick`), así no queda un loop de repaint ocioso.
fn ensure_tick(m: &mut Model, handle: &Handle<Msg>) {
    if m.ticking || !m.loading {
        return;
    }
    m.ticking = true;
    handle.spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        Msg::Tick
    });
}

/// Empuja un toast al stack y programa su expiración.
fn push_toast(m: &mut Model, handle: &Handle<Msg>, toast: Toast) {
    let id = toast.id;
    m.toasts.push(toast);
    handle.spawn(move || {
        std::thread::sleep(TOAST_TTL);
        Msg::ToastExpire(id)
    });
}

/// `key` estable de la escena actual (tendencias / búsqueda / canal, por
/// backend). Cambia sólo al cambiar de escena → dispara la transición de
/// entrada del cuerpo; estable durante la carga de la misma escena.
fn scene_key(m: &Model) -> u64 {
    match &m.channel {
        Some((id, _)) => key_of(&format!("ch:{id}")),
        None if !m.query.is_empty() => key_of(&format!("q:{}", m.query)),
        None => key_of(&format!("trend:{}", m.backend.nombre())),
    }
}

/// Grilla de placeholders con shimmer mientras llega la primera tanda de
/// resultados — el usuario ve la forma de lo que viene, no un hueco negro.
fn skeleton_grid(area_w: f32, area_h: f32, theme: &Theme) -> View<Msg> {
    let pal = SkeletonPalette::from_theme(theme);
    // Total holgado para que `ventana_visible` calcule cuántos tiles llenan
    // el área; tomamos sólo los visibles.
    let win = ventana_visible(60, area_w, area_h, 0, &METRICS);
    let cells: Vec<GridCell<Msg>> = (0..win.count.clamp(1, 60))
        .map(|_| {
            let thumb = View::new(Style {
                size: Size { width: length(THUMB_W), height: length(THUMB_H) },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .radius(5.0)
            .clip(true)
            .children(vec![skeleton_view(&pal)]);
            let line = View::new(Style {
                size: Size { width: length(THUMB_W), height: length(12.0_f32) },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .radius(4.0)
            .clip(true)
            .children(vec![skeleton_view(&pal)]);
            let content = View::new(Style {
                flex_direction: FlexDirection::Column,
                align_items: Some(AlignItems::Center),
                gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
                ..Default::default()
            })
            .children(vec![thumb, line]);
            GridCell { content, label: None, selected: false, on_click: Msg::Tick }
        })
        .collect();
    grid_view(GridSpec {
        cells,
        cols: win.cols,
        metrics: METRICS,
        caption: None,
        truncated_hint: None,
        palette: GridPalette::from_theme(theme),
    })
}

fn dur_fmt(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

struct MediaTube;

impl App for MediaTube {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "media · plataforma de video"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 760)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        // Carga los catálogos Fluent (es/en/qu) una sola vez. Idempotente.
        rimay_localize::init();
        let backend = Backend::Invidious;
        let instance = default_instance(backend);
        // Arrancamos mostrando tendencias para que la grilla no esté vacía.
        let (b, inst) = (backend, instance.clone());
        handle.spawn(move || match tendencias(b, &inst) {
            Ok(v) => Msg::Loaded { gen: 1, append: false, items: Arc::new(v) },
            Err(e) => Msg::LoadFailed(1, e.to_string()),
        });
        let mut m = Model {
            backend,
            instance,
            search: TextInputState::new(),
            search_focused: false,
            videos: Vec::new(),
            status: t("media-tube-trending-loading"),
            gen: 1,
            query: String::new(),
            page: 0,
            loading: true,
            exhausted: false,
            channel: None,
            scroll_fila: 0,
            thumbs: ImageCache::new(),
            thumb_pending: HashSet::new(),
            viewport: (1180.0, 760.0),
            toasts: Vec::new(),
            next_toast: 0,
            ticking: false,
        };
        ensure_tick(&mut m, handle);
        m
    }

    fn update(mut m: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::SearchFocus => m.search_focused = true,
            Msg::SearchBlur => m.search_focused = false,
            Msg::SearchKey(e) => {
                m.search.apply_key(&e);
            }
            Msg::DoSearch => {
                let q = m.search.text();
                if q.trim().is_empty() {
                    return m;
                }
                m.gen += 1;
                let gen = m.gen;
                m.videos.clear();
                m.scroll_fila = 0;
                m.status = t_args(
                    "media-tube-searching",
                    &[("q", q.clone().into()), ("backend", m.backend.nombre().into())],
                );
                m.query = q.clone();
                m.page = 1;
                m.loading = true;
                m.exhausted = false;
                let (b, inst, page) = (m.backend, m.instance.clone(), m.page);
                handle.spawn(move || match buscar(b, &inst, &q, page) {
                    Ok(v) => Msg::Loaded { gen, append: false, items: Arc::new(v) },
                    Err(e) => Msg::LoadFailed(gen, e.to_string()),
                });
            }
            Msg::Loaded { gen, append, items } => {
                if gen == m.gen {
                    m.loading = false;
                    let items = Arc::try_unwrap(items).unwrap_or_else(|a| (*a).clone());
                    if append {
                        if items.is_empty() {
                            m.exhausted = true;
                        } else {
                            m.videos.extend(items);
                        }
                    } else {
                        m.videos = items;
                    }
                    m.status = if m.videos.is_empty() {
                        t("media-tube-no-results")
                    } else if let Some((_, name)) = &m.channel {
                        t_args(
                            "media-tube-channel-status",
                            &[("name", name.clone().into()), ("count", m.videos.len().to_string().into())],
                        )
                    } else {
                        let que = if m.query.is_empty() {
                            t("media-tube-trending")
                        } else {
                            t("media-tube-results")
                        };
                        format!("{} · {} · {}", que, m.videos.len(), m.backend.nombre())
                    };
                    kick_thumbs(&mut m, handle);
                }
            }
            Msg::LoadFailed(gen, e) => {
                if gen == m.gen {
                    m.loading = false;
                    m.status = t_args("media-tube-status-error", &[("e", e.clone().into())]);
                    let id = m.next_toast;
                    m.next_toast += 1;
                    push_toast(
                        &mut m,
                        handle,
                        Toast::error(id, t_args("media-tube-load-failed", &[("e", e.into())]), TOAST_TTL),
                    );
                }
            }
            Msg::SetBackend(b) => {
                if m.backend != b {
                    m.backend = b;
                    m.instance = default_instance(b);
                    m.videos.clear();
                    m.scroll_fila = 0;
                    m.query.clear();
                    m.channel = None;
                    m.page = 0;
                    m.loading = true;
                    m.exhausted = false;
                    m.gen += 1;
                    m.status = t_args("media-tube-trending-of", &[("backend", b.nombre().into())]);
                    let (gen, inst) = (m.gen, m.instance.clone());
                    handle.spawn(move || match tendencias(b, &inst) {
                        Ok(v) => Msg::Loaded { gen, append: false, items: Arc::new(v) },
                        Err(e) => Msg::LoadFailed(gen, e.to_string()),
                    });
                }
            }
            Msg::Play(i) => {
                if let Some(card) = m.videos.get(i) {
                    let url = watch_url(m.backend, &m.instance, &card.id);
                    let titulo = card.title.clone();
                    m.status = format!("▶ {titulo}");
                    lanzar_media_app(&url);
                    let id = m.next_toast;
                    m.next_toast += 1;
                    push_toast(&mut m, handle, Toast::success(id, format!("▶ {titulo}"), TOAST_TTL));
                }
            }
            Msg::OpenChannel { id, name } => {
                m.channel = Some((id.clone(), name.clone()));
                m.gen += 1;
                m.videos.clear();
                m.scroll_fila = 0;
                m.loading = true;
                m.exhausted = true; // channel_videos no se pagina (descriptor sin page)
                m.status = t_args("media-tube-channel-loading", &[("name", name.clone().into())]);
                let (gen, b, inst) = (m.gen, m.backend, m.instance.clone());
                handle.spawn(move || match canal(b, &inst, &id) {
                    Ok(v) => Msg::Loaded { gen, append: false, items: Arc::new(v) },
                    Err(e) => Msg::LoadFailed(gen, e.to_string()),
                });
            }
            Msg::Back => {
                m.channel = None;
                m.scroll_fila = 0;
                m.loading = true;
                m.exhausted = false;
                m.gen += 1;
                let (gen, b, inst, q) = (m.gen, m.backend, m.instance.clone(), m.query.clone());
                if q.is_empty() {
                    m.status = t("media-tube-trending-loading");
                    handle.spawn(move || match tendencias(b, &inst) {
                        Ok(v) => Msg::Loaded { gen, append: false, items: Arc::new(v) },
                        Err(e) => Msg::LoadFailed(gen, e.to_string()),
                    });
                } else {
                    m.page = 1;
                    m.status = t_args("media-tube-searching-short", &[("q", q.clone().into())]);
                    handle.spawn(move || match buscar(b, &inst, &q, 1) {
                        Ok(v) => Msg::Loaded { gen, append: false, items: Arc::new(v) },
                        Err(e) => Msg::LoadFailed(gen, e.to_string()),
                    });
                }
            }
            Msg::ThumbDone(url) => {
                m.thumb_pending.remove(&url);
            }
            Msg::Wheel(dy) => {
                let next = (m.scroll_fila as f32 + dy).max(0.0) as usize;
                if next != m.scroll_fila {
                    m.scroll_fila = next;
                    kick_thumbs(&mut m, handle);
                    maybe_load_more(&mut m, handle);
                }
            }
            Msg::Resize(w, h) => {
                m.viewport = (w as f32, h as f32);
                kick_thumbs(&mut m, handle);
                maybe_load_more(&mut m, handle);
            }
            Msg::Tick => {
                // El thread durmió 50ms; sólo rearmamos si seguimos cargando.
                m.ticking = false;
            }
            Msg::ToastExpire(id) => {
                m.toasts.retain(|t| t.id != id);
            }
        }
        // Si quedó una carga en vuelo, mantené el shimmer animado.
        ensure_tick(&mut m, handle);
        m
    }

    fn on_key(m: &Model, e: &KeyEvent) -> Option<Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        if m.search_focused {
            match &e.key {
                Key::Named(NamedKey::Enter) => Some(Msg::DoSearch),
                Key::Named(NamedKey::Escape) => Some(Msg::SearchBlur),
                _ => Some(Msg::SearchKey(e.clone())),
            }
        } else if matches!(&e.key, Key::Character(c) if c == "/") {
            Some(Msg::SearchFocus)
        } else {
            None
        }
    }

    fn on_wheel(_m: &Model, delta: WheelDelta, _c: (f32, f32), _mods: Modifiers) -> Option<Msg> {
        if delta.y.abs() > f32::EPSILON {
            Some(Msg::Wheel(delta.y))
        } else {
            None
        }
    }

    fn on_resize(_m: &Model, w: u32, h: u32) -> Option<Msg> {
        Some(Msg::Resize(w, h))
    }

    fn view(m: &Model) -> View<Msg> {
        let theme = Theme::dark();

        // ----- Header: marca + conmutador de backend + búsqueda + estado -----
        let brand = View::new(Style {
            size: Size { width: length(86.0), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text("media tube", 19.0, Color::from_rgba8(235, 240, 248, 255))
        .bold();

        let backend_btn = |b: Backend| -> View<Msg> {
            let active = m.backend == b;
            let (bg, fg) = if active {
                (Color::from_rgba8(70, 130, 200, 255), Color::WHITE)
            } else {
                (Color::from_rgba8(44, 50, 62, 255), Color::from_rgba8(190, 198, 210, 255))
            };
            View::new(Style {
                size: Size { width: length(92.0), height: length(32.0) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                flex_shrink: 0.0,
                ..Default::default()
            })
            .fill(bg)
            .radius(7.0)
            .hover_fill(Color::from_rgba8(80, 140, 210, 255))
            .text(b.nombre(), 12.5, fg)
            .on_click(Msg::SetBackend(b))
        };

        let search_ph = t("media-tube-search-placeholder");
        let search_box = text_input_view(
            &m.search,
            &search_ph,
            m.search_focused,
            &TextInputPalette::from_theme(&theme),
            Msg::SearchFocus,
        );
        let search_wrap = View::new(Style {
            flex_grow: 1.0,
            size: Size { width: auto(), height: length(34.0) },
            ..Default::default()
        })
        .children(vec![search_box]);

        let status = View::new(Style {
            size: Size { width: length(220.0), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::End),
            flex_shrink: 0.0,
            ..Default::default()
        })
        .text(m.status.clone(), 12.0, Color::from_rgba8(160, 170, 184, 255))
        .ellipsis(1);

        // En vista de canal, un botón para volver al listado anterior.
        let mut hdr: Vec<View<Msg>> = vec![brand];
        if m.channel.is_some() {
            hdr.push(
                View::new(Style {
                    size: Size { width: length(74.0), height: length(32.0) },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Center),
                    flex_shrink: 0.0,
                    ..Default::default()
                })
                .fill(Color::from_rgba8(44, 50, 62, 255))
                .radius(7.0)
                .hover_fill(Color::from_rgba8(60, 68, 84, 255))
                .text(t("media-tube-back"), 12.0, Color::from_rgba8(210, 216, 226, 255))
                .on_click(Msg::Back),
            );
        }
        hdr.push(backend_btn(Backend::Invidious));
        hdr.push(backend_btn(Backend::PeerTube));
        hdr.push(search_wrap);
        hdr.push(status);

        let header = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(HEADER_H) },
            align_items: Some(AlignItems::Center),
            gap: Size { width: length(12.0_f32), height: length(0.0_f32) },
            padding: Rect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .fill(Color::from_rgba8(28, 32, 42, 255))
        .children(hdr);

        // ----- Grilla virtualizada de resultados -----
        let (area_w, area_h) = grid_area(m);
        let win = ventana_visible(m.videos.len(), area_w, area_h, m.scroll_fila, &METRICS);
        let cells: Vec<GridCell<Msg>> = (win.first..(win.first + win.count))
            .map(|i| {
                let card = &m.videos[i];
                let url = thumb_url(&m.instance, card);
                let thumb = match m.thumbs.get(&url) {
                    // La miniatura entra con fade-in (animated_enter) la primera
                    // vez que aparece su key — el placeholder no salta de golpe.
                    Some(img) => View::new(Style {
                        size: Size { width: length(THUMB_W), height: length(THUMB_H) },
                        ..Default::default()
                    })
                    .image(img)
                    .image_fit(ImageFit::Cover)
                    .radius(5.0)
                    .animated_enter(key_of(&url), motion::NORMAL),
                    None => View::new(Style {
                        size: Size { width: length(THUMB_W), height: length(THUMB_H) },
                        align_items: Some(AlignItems::Center),
                        justify_content: Some(JustifyContent::Center),
                        ..Default::default()
                    })
                    .fill(Color::from_rgba8(48, 54, 66, 255))
                    .radius(5.0)
                    .text("▶", 22.0, Color::from_rgba8(120, 130, 145, 255)),
                };
                // Chip de autor: click → abrir canal. Gana sobre el on_click
                // de la celda (Play) por ser el nodo más profundo en el hit-test.
                let author = card.author.clone().unwrap_or_default();
                let chip = match &card.channel_id {
                    Some(cid) if !cid.is_empty() && !author.is_empty() => View::new(Style {
                        size: Size { width: length(THUMB_W), height: length(16.0) },
                        align_items: Some(AlignItems::Center),
                        justify_content: Some(JustifyContent::Center),
                        flex_shrink: 0.0,
                        ..Default::default()
                    })
                    .radius(4.0)
                    .hover_fill(Color::from_rgba8(40, 46, 58, 255))
                    .text(format!("@ {author}"), 10.5, Color::from_rgba8(120, 170, 235, 255))
                    .ellipsis(1)
                    .on_click(Msg::OpenChannel { id: cid.clone(), name: author.clone() }),
                    _ => View::new(Style {
                        size: Size { width: length(THUMB_W), height: length(16.0) },
                        ..Default::default()
                    }),
                };
                let content = View::new(Style {
                    flex_direction: FlexDirection::Column,
                    align_items: Some(AlignItems::Center),
                    gap: Size { width: length(0.0_f32), height: length(3.0_f32) },
                    ..Default::default()
                })
                .children(vec![thumb, chip]);
                let label = match card.duration_secs {
                    Some(s) if s > 0 => format!("{}   ·   {}", card.title, dur_fmt(s)),
                    _ => card.title.clone(),
                };
                GridCell {
                    content,
                    label: Some(label),
                    selected: false,
                    on_click: Msg::Play(i),
                }
            })
            .collect();

        // ----- Contenido del cuerpo según el estado -----
        // 1) cargando y sin nada que mostrar → grilla de skeletons (shimmer)
        // 2) cargado y vacío → empty-state con orientación
        // 3) datos → grilla virtualizada real
        let scene_content: View<Msg> = if m.videos.is_empty() && m.loading {
            skeleton_grid(area_w, area_h, &theme)
        } else if m.videos.is_empty() {
            let pal = EmptyPalette::from_theme(&theme);
            let (titulo, desc) = if m.query.is_empty() {
                (t("media-tube-empty-no-videos-title"), t("media-tube-empty-no-videos-desc"))
            } else {
                (t("media-tube-no-results"), t("media-tube-empty-no-results-desc"))
            };
            empty_view(Icon::Film, titulo, Some(desc.as_str()), &pal)
        } else {
            grid_view(GridSpec {
                cells,
                cols: win.cols,
                metrics: METRICS,
                caption: None,
                truncated_hint: None,
                palette: GridPalette::from_theme(&theme),
            })
        };

        // Transición de escena: al cambiar entre tendencias / búsqueda / canal
        // (o de backend) la `scene_key` cambia y el contenido entra con un
        // fade + slide-up suave en vez de saltar.
        use llimphi_ui::llimphi_raster::kurbo::Affine;
        let scene_key = scene_key(m);
        let scene = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(vec![scene_content])
        .animated_enter_from(scene_key, motion::SLOW, Affine::translate((0.0, 24.0)));

        let body = View::new(Style {
            flex_grow: 1.0,
            size: Size { width: percent(1.0_f32), height: auto() },
            min_size: Size { width: length(0.0), height: length(0.0) },
            ..Default::default()
        })
        .fill(Color::from_rgba8(20, 23, 30, 255))
        .children(vec![scene]);

        let root = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(Color::from_rgba8(20, 23, 30, 255))
        .children(vec![header, body]);

        // Overlay de toasts (bottom-right). Click en uno = descartarlo.
        let now = Instant::now();
        let alive: Vec<Toast> = m.toasts.iter().filter(|t| t.is_alive(now)).cloned().collect();
        if alive.is_empty() {
            root
        } else {
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .children(vec![root, toast_stack_view(&alive, m.viewport, Msg::ToastExpire)])
        }
    }
}

fn main() {
    llimphi_ui::run::<MediaTube>();
}
