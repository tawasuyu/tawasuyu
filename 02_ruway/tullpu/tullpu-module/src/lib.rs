//! `tullpu-module` — el editor de imágenes por capas como **módulo
//! embebible** (frontend chico sobre los motores reales).
//!
//! Patrón `nahual-module`: `State` + `Msg` + `update` + `view(state, theme,
//! lift)` — el host (p. ej. el canvas de nahual) monta el módulo mapeando
//! los `Msg` a los suyos con `lift`. Nada de IPC: las capas, el compositor,
//! el pincel y las ops son los crates canónicos `tullpu-{core,render,ops,
//! paint}` — este módulo sólo arma una UI mínima:
//!
//! - **Herramientas**: mover (pan), pincel, borrador (con radio y color).
//! - **Ops locales** como capas derivadas no destructivas (brillo,
//!   contraste, blur, invertir, saturación) — regeneradas con
//!   [`tullpu_ops::regenerar_stale`] (sin daemon IA).
//! - **Panel de capas**: seleccionar, mostrar/ocultar.
//! - **Undo/redo** por snapshots del `Lienzo` (baratos: DAG + hashes).
//! - **Zoom/pan** del lienzo y **guardar** (export PNG/JPEG/WebP al path).
//!
//! El editor completo (máscaras, degradés, selección, IA, PSD) sigue en
//! `tullpu-app-llimphi`; este módulo es la cara "editar acá mismo" para
//! hosts tipo file manager.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use llimphi_icons::{icon_view, Icon};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KurboRect};
use llimphi_ui::llimphi_raster::peniko::{
    BlendMode, Blob, Fill, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
};
use llimphi_ui::{DragPhase, View};
use llimphi_widget_toolbar::{toolbar_view, ToolbarGroup, ToolbarItem, ToolbarPalette};
use tullpu_core::{Capa, Lienzo, OpLocal, OrigenCapa, TransformacionPixel};
use tullpu_ops::regenerar_stale;
use tullpu_paint::{estampar_disco, trazar_linea_pincel};
use tullpu_render::{componer, exportar, AlmacenEnMemoria, FormatoExport};
use uuid::Uuid;

pub use tullpu_core::OpLocal as Op;

/// Tope de snapshots de undo.
const UNDO_MAX: usize = 64;
/// Radio máximo del pincel (px).
const RADIO_MAX: i32 = 64;
/// Ancho del panel de capas (px).
const CAPAS_W: f32 = 200.0;

/// Herramienta activa del módulo (subset de la app).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Herramienta {
    /// Arrastrar panea el lienzo.
    Mover,
    Pincel,
    Borrador,
}

/// Trazo de pincel en curso (entre press y release).
struct Trazo {
    cur_lx: f32,
    cur_ly: f32,
    rw: f32,
    rh: f32,
    last_ix: i32,
    last_iy: i32,
}

pub struct State {
    pub lienzo: Lienzo,
    pub almacen: AlmacenEnMemoria,
    pub seleccionada: Option<Uuid>,
    pub herramienta: Herramienta,
    /// Color activo del pincel (RGBA).
    pub color: [u8; 4],
    pub radio: i32,
    pub dureza: f32,
    pub zoom: f32,
    pub pan: (f32, f32),
    /// Composite renderizado listo para pintar.
    pub imagen: Option<Image>,
    /// Hay cambios sin guardar.
    pub dirty: bool,
    /// El último guardado fue exitoso (para el rótulo).
    pub guardado: bool,
    /// Path de origen — `Guardar` exporta acá (formato por extensión).
    pub path: PathBuf,
    /// Mensaje de estado (última acción / error).
    pub estado: String,
    undo: Vec<(Lienzo, Option<Uuid>)>,
    redo: Vec<(Lienzo, Option<Uuid>)>,
    trazo: Option<Trazo>,
}

#[derive(Clone)]
pub enum Msg {
    Herr(Herramienta),
    SetColor([u8; 4]),
    BumpRadio(i32),
    /// Press sobre el lienzo (coords locales del panel + dims del panel).
    Press { lx: f32, ly: f32, rw: f32, rh: f32 },
    /// Drag acumulado del trazo (o del pan si la herramienta es Mover).
    Drag { dx: f32, dy: f32 },
    Suelta,
    Pan(f32, f32),
    /// Zoom multiplicativo (rueda del host / botones).
    Zoom(f32),
    ResetVista,
    /// Apila una op local como capa derivada de la seleccionada.
    Op(OpLocal),
    Seleccionar(Uuid),
    ToggleVisible(Uuid),
    Undo,
    Redo,
    Guardar,
}

impl State {
    /// Abre `path` como un lienzo de una capa raster. `None` si no decodifica.
    pub fn desde_imagen(path: &Path) -> Option<State> {
        let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
        let rgba = img.decode().ok()?.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        let mut almacen = AlmacenEnMemoria::nuevo();
        let hash = almacen.insertar(rgba.into_raw());
        let mut lienzo = Lienzo::nuevo(w, h);
        let capa = Capa::raster(
            path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "imagen".into()),
            hash,
        );
        let id = capa.id;
        lienzo.apilar(capa);
        let mut st = State {
            lienzo,
            almacen,
            seleccionada: Some(id),
            herramienta: Herramienta::Mover,
            color: [20, 20, 20, 255],
            radio: 6,
            dureza: 0.8,
            zoom: 1.0,
            pan: (0.0, 0.0),
            imagen: None,
            dirty: false,
            guardado: false,
            path: path.to_path_buf(),
            estado: "listo".into(),
            undo: Vec::new(),
            redo: Vec::new(),
            trazo: None,
        };
        st.recomponer();
        Some(st)
    }

    fn recomponer(&mut self) {
        match componer(&self.lienzo, &self.almacen) {
            Ok(img) => {
                let (w, h) = (img.width(), img.height());
                self.imagen = Some(Image::new(ImageData {
                    data: Blob::from(img.into_raw()),
                    format: ImageFormat::Rgba8,
                    alpha_type: ImageAlphaType::Alpha,
                    width: w,
                    height: h,
                }));
            }
            Err(e) => self.estado = format!("compositor: {e}"),
        }
    }

    fn snapshot(&mut self) {
        self.undo.push((self.lienzo.clone(), self.seleccionada));
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn puede_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn puede_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Estampa con el pincel sobre la capa raster seleccionada usando
    /// `dibujar` (un punto o un segmento). `true` si cambió algo.
    fn pincel(&mut self, dibujar: impl FnOnce(&mut Vec<u8>, u32, u32, [u8; 4])) -> bool {
        let Some(id) = self.seleccionada else { return false };
        let Some(capa) = self.lienzo.capa(id) else { return false };
        if !matches!(capa.origen, OrigenCapa::Raster) {
            self.estado = "la capa seleccionada es derivada — elegí la raster".into();
            return false;
        }
        let hash_actual = capa.contenido;
        let (w, h) = (self.lienzo.width, self.lienzo.height);
        let Some(src) = tullpu_render::FuenteBuffers::obtener(&self.almacen, hash_actual) else {
            return false;
        };
        let mut buf = src.to_vec();
        dibujar(&mut buf, w, h, self.color);
        let nuevo = self.almacen.insertar(buf);
        if nuevo == hash_actual {
            return false;
        }
        if let Some(c) = self.lienzo.capa_mut(id) {
            c.contenido = nuevo;
        }
        self.lienzo.propagar_stale(id);
        self.regenerar_y_recomponer();
        self.dirty = true;
        self.guardado = false;
        true
    }

    fn regenerar_y_recomponer(&mut self) {
        if let Err(e) = regenerar_stale(&mut self.lienzo, &mut self.almacen) {
            self.estado = format!("ops: {e}");
        }
        self.recomponer();
    }
}

/// Mapea coords locales del panel a coords de imagen (mismo cálculo que la
/// app: fit + zoom + pan).
fn transform(image_w: u32, image_h: u32, rw: f32, rh: f32, zoom: f32, pan: (f32, f32)) -> Option<(f64, f64, f64)> {
    if image_w == 0 || image_h == 0 || rw <= 0.0 || rh <= 0.0 {
        return None;
    }
    let s = (rw as f64 / image_w as f64).min(rh as f64 / image_h as f64) * zoom as f64;
    let dw = image_w as f64 * s;
    let dh = image_h as f64 * s;
    let off_x = (rw as f64 - dw) * 0.5 + pan.0 as f64;
    let off_y = (rh as f64 - dh) * 0.5 + pan.1 as f64;
    Some((s, off_x, off_y))
}

fn local_a_imagen(st: &State, lx: f32, ly: f32, rw: f32, rh: f32) -> Option<(i32, i32)> {
    let (s, ox, oy) = transform(st.lienzo.width, st.lienzo.height, rw, rh, st.zoom, st.pan)?;
    if s <= 0.0 {
        return None;
    }
    Some((((lx as f64 - ox) / s).floor() as i32, ((ly as f64 - oy) / s).floor() as i32))
}

pub fn update(mut st: State, msg: Msg) -> State {
    match msg {
        Msg::Herr(h) => st.herramienta = h,
        Msg::SetColor(c) => st.color = c,
        Msg::BumpRadio(d) => st.radio = (st.radio + d).clamp(1, RADIO_MAX),
        Msg::Press { lx, ly, rw, rh } => {
            if st.herramienta == Herramienta::Mover {
                return st;
            }
            if let Some((ix, iy)) = local_a_imagen(&st, lx, ly, rw, rh) {
                st.snapshot();
                let borrar = st.herramienta == Herramienta::Borrador;
                let (radio, dureza) = (st.radio, st.dureza);
                st.pincel(|buf, w, h, color| {
                    estampar_disco(buf, w, h, ix, iy, radio, color, borrar, dureza, None);
                });
                st.trazo = Some(Trazo { cur_lx: lx, cur_ly: ly, rw, rh, last_ix: ix, last_iy: iy });
            }
        }
        Msg::Drag { dx, dy } => {
            if st.herramienta == Herramienta::Mover {
                st.pan.0 += dx;
                st.pan.1 += dy;
                return st;
            }
            if let Some(t) = st.trazo.as_mut() {
                t.cur_lx += dx;
                t.cur_ly += dy;
                let (clx, cly, rw, rh, lx0, ly0) = (t.cur_lx, t.cur_ly, t.rw, t.rh, t.last_ix, t.last_iy);
                if let Some((ix, iy)) = local_a_imagen(&st, clx, cly, rw, rh) {
                    let borrar = st.herramienta == Herramienta::Borrador;
                    let (radio, dureza) = (st.radio, st.dureza);
                    st.pincel(|buf, w, h, color| {
                        trazar_linea_pincel(buf, w, h, lx0, ly0, ix, iy, radio, color, borrar, dureza, None);
                    });
                    if let Some(t) = st.trazo.as_mut() {
                        t.last_ix = ix;
                        t.last_iy = iy;
                    }
                }
            }
        }
        Msg::Suelta => st.trazo = None,
        Msg::Pan(dx, dy) => {
            st.pan.0 += dx;
            st.pan.1 += dy;
        }
        Msg::Zoom(mult) => st.zoom = (st.zoom * mult).clamp(0.2, 16.0),
        Msg::ResetVista => {
            st.zoom = 1.0;
            st.pan = (0.0, 0.0);
        }
        Msg::Op(op) => {
            if let Some(madre) = st.seleccionada {
                st.snapshot();
                let nueva = Capa::derivada(
                    op_etiqueta(&op),
                    madre,
                    TransformacionPixel::Local(op),
                    [0u8; 32],
                );
                let id = nueva.id;
                st.lienzo.apilar(nueva);
                st.seleccionada = Some(id);
                st.regenerar_y_recomponer();
                st.dirty = true;
                st.guardado = false;
            }
        }
        Msg::Seleccionar(id) => st.seleccionada = Some(id),
        Msg::ToggleVisible(id) => {
            st.snapshot();
            if let Some(c) = st.lienzo.capa_mut(id) {
                c.visible = !c.visible;
            }
            st.recomponer();
            st.dirty = true;
            st.guardado = false;
        }
        Msg::Undo => {
            if let Some((l, sel)) = st.undo.pop() {
                st.redo.push((std::mem::replace(&mut st.lienzo, l), st.seleccionada));
                st.seleccionada = sel;
                st.regenerar_y_recomponer();
                st.dirty = true;
                st.guardado = false;
            }
        }
        Msg::Redo => {
            if let Some((l, sel)) = st.redo.pop() {
                st.undo.push((std::mem::replace(&mut st.lienzo, l), st.seleccionada));
                st.seleccionada = sel;
                st.regenerar_y_recomponer();
                st.dirty = true;
                st.guardado = false;
            }
        }
        Msg::Guardar => {
            let formato = match st.path.extension().and_then(|s| s.to_str()) {
                Some("jpg" | "jpeg") => FormatoExport::Jpeg { calidad: 90 },
                Some("webp") => FormatoExport::Webp,
                _ => FormatoExport::Png,
            };
            match exportar(&st.lienzo, &st.almacen, &st.path, formato) {
                Ok(_) => {
                    st.dirty = false;
                    st.guardado = true;
                    st.estado = "guardado".into();
                }
                Err(e) => st.estado = format!("guardar: {e}"),
            }
        }
    }
    st
}

fn op_etiqueta(op: &OpLocal) -> String {
    match op {
        OpLocal::Invertir => "invertir".into(),
        OpLocal::Brillo { delta } => format!("brillo {delta:+.1}"),
        OpLocal::Contraste { factor } => format!("contraste ×{factor:.1}"),
        OpLocal::Blur { radio } => format!("blur {radio:.0}px"),
        OpLocal::Saturacion { factor } => format!("saturación ×{factor:.1}"),
        _ => "op".into(),
    }
}

/// Paleta de swatches del pincel.
const SWATCHES: [[u8; 4]; 8] = [
    [20, 20, 20, 255],
    [240, 240, 240, 255],
    [200, 60, 50, 255],
    [220, 150, 40, 255],
    [60, 160, 70, 255],
    [60, 120, 200, 255],
    [140, 90, 200, 255],
    [230, 210, 70, 255],
];

/// Vista del módulo: toolbar (herramientas · ops · undo/redo · guardar) +
/// lienzo (pinta el composite con zoom/pan; el input depende de la
/// herramienta) + panel de capas.
pub fn view<H: Clone + Send + Sync + 'static>(
    st: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> H + Clone + Send + Sync + 'static,
) -> View<H> {
    let barra = barra(st, theme, lift.clone());
    let lienzo = lienzo_view(st, theme, lift.clone());
    let capas = capas_view(st, theme, lift);
    let cuerpo = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        min_size: Size { width: length(0.0), height: length(0.0) },
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![lienzo, capas]);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![barra, cuerpo])
}

fn barra<H: Clone + Send + Sync + 'static>(
    st: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> H + Clone + Send + Sync + 'static,
) -> View<H> {
    let herr = |ic: Icon, h: Herramienta, label: &str| {
        ToolbarItem::new(move |_s, c| icon_view(ic, c, 1.7), lift(Msg::Herr(h)))
            .with_label(label)
            .active(st.herramienta == h)
    };
    let op = |ic: Icon, o: OpLocal, label: &str| {
        ToolbarItem::new(move |_s, c| icon_view(ic, c, 1.7), lift(Msg::Op(o))).with_label(label)
    };
    let nombre = st
        .path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let estado_guardar = if st.dirty {
        "●"
    } else if st.guardado {
        "✓"
    } else {
        ""
    };
    toolbar_view(
        vec![
            ToolbarGroup::new(vec![
                herr(Icon::More, Herramienta::Mover, "mover"),
                herr(Icon::Edit, Herramienta::Pincel, "pincel"),
                herr(Icon::Minus, Herramienta::Borrador, "goma"),
                ToolbarItem::new(
                    |_s, c| icon_view(Icon::Plus, c, 1.7),
                    lift(Msg::BumpRadio(2)),
                ),
                ToolbarItem::new(
                    |_s, c| icon_view(Icon::Minus, c, 1.7),
                    lift(Msg::BumpRadio(-2)),
                ),
            ]),
            ToolbarGroup::new(vec![
                op(Icon::Volume, OpLocal::Brillo { delta: 0.12 }, "brillo+"),
                op(Icon::VolumeMute, OpLocal::Brillo { delta: -0.12 }, "brillo−"),
                op(Icon::Equalizer, OpLocal::Contraste { factor: 1.2 }, "contraste"),
                op(Icon::Record, OpLocal::Blur { radio: 4.0 }, "blur"),
                op(Icon::Repeat, OpLocal::Invertir, "invertir"),
                op(Icon::Image, OpLocal::Saturacion { factor: 0.0 }, "gris"),
            ]),
            ToolbarGroup::new(vec![
                ToolbarItem::new(|_s, c| icon_view(Icon::Rewind, c, 1.7), lift(Msg::Undo))
                    .enabled(st.puede_undo()),
                ToolbarItem::new(|_s, c| icon_view(Icon::FastForward, c, 1.7), lift(Msg::Redo))
                    .enabled(st.puede_redo()),
            ]),
            ToolbarGroup::new(vec![ToolbarItem::new(
                |_s, c| icon_view(Icon::Save, c, 1.7),
                lift(Msg::Guardar),
            )
            .with_label(format!("{nombre} {estado_guardar}"))
            .enabled(st.dirty || !st.guardado)]),
        ],
        34.0,
        &ToolbarPalette::from_theme(theme),
    )
}

fn lienzo_view<H: Clone + Send + Sync + 'static>(
    st: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> H + Clone + Send + Sync + 'static,
) -> View<H> {
    let zoom = st.zoom;
    let pan = st.pan;
    let dims = (st.lienzo.width, st.lienzo.height);
    let img = st.imagen.clone();
    let base = View::new(Style {
        flex_grow: 1.0,
        min_size: Size { width: length(0.0), height: length(0.0) },
        size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .paint_with(move |scene, _ts, rect| {
        let Some(image) = img.as_ref() else { return };
        let Some((s, ox, oy)) = transform(dims.0, dims.1, rect.w, rect.h, zoom, pan) else {
            return;
        };
        let clip = KurboRect::new(
            rect.x as f64,
            rect.y as f64,
            (rect.x + rect.w) as f64,
            (rect.y + rect.h) as f64,
        );
        scene.push_layer(Fill::NonZero, BlendMode::default(), 1.0, Affine::IDENTITY, &clip);
        scene.draw_image(
            image,
            Affine::translate((rect.x as f64 + ox, rect.y as f64 + oy)) * Affine::scale(s),
        );
        scene.pop_layer();
    });
    // El input según herramienta (mismo cableado que la app).
    let l1 = lift.clone();
    let l2 = lift.clone();
    match st.herramienta {
        Herramienta::Mover => base.draggable(move |fase, dx, dy| match fase {
            DragPhase::Move => Some(l1(Msg::Pan(dx, dy))),
            DragPhase::End => None,
        }),
        Herramienta::Pincel | Herramienta::Borrador => base
            .on_click_at(move |lx, ly, rw, rh| Some(l1(Msg::Press { lx, ly, rw, rh })))
            .draggable_at(move |fase, dx, dy, _lx0, _ly0| match fase {
                DragPhase::Move => Some(l2(Msg::Drag { dx, dy })),
                DragPhase::End => Some(l2(Msg::Suelta)),
            }),
    }
}

fn capas_view<H: Clone + Send + Sync + 'static>(
    st: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> H + Clone + Send + Sync + 'static,
) -> View<H> {
    let mut filas: Vec<View<H>> = vec![View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text("CAPAS", 12.0, theme.fg_muted)];
    // De arriba (última en el stack = más visible) hacia abajo.
    for capa in st.lienzo.capas.iter().rev() {
        let sel = st.seleccionada == Some(capa.id);
        let ojo_icon = if capa.visible { Icon::Check } else { Icon::X };
        let ojo = View::new(Style {
            size: Size { width: length(24.0_f32), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            flex_shrink: 0.0,
            ..Default::default()
        })
        .on_click(lift(Msg::ToggleVisible(capa.id)))
        .children(vec![View::new(Style {
            size: Size { width: length(13.0_f32), height: length(13.0_f32) },
            ..Default::default()
        })
        .children(vec![icon_view(ojo_icon, if capa.visible { theme.fg_text } else { theme.fg_muted }, 1.7)])]);
        let nombre = View::new(Style {
            flex_grow: 1.0,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .on_click(lift(Msg::Seleccionar(capa.id)))
        .text(
            capa.nombre.clone(),
            12.5,
            if sel { theme.fg_text } else { theme.fg_muted },
        );
        let mut fila = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
            align_items: Some(AlignItems::Center),
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![ojo, nombre]);
        if sel {
            fila = fila.fill(theme.bg_selected);
        }
        filas.push(fila);
    }
    // Swatches de color del pincel.
    filas.push(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
            padding: Rect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            flex_shrink: 0.0,
            ..Default::default()
        })
        .text(format!("COLOR · radio {}px", st.radio * 2 + 1), 12.0, theme.fg_muted),
    );
    let swatches: Vec<View<H>> = SWATCHES
        .iter()
        .map(|c| {
            let activo = st.color == *c;
            let mut v = View::new(Style {
                size: Size { width: length(18.0_f32), height: length(18.0_f32) },
                flex_shrink: 0.0,
                margin: Rect {
                    left: length(2.0_f32),
                    right: length(2.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .fill(llimphi_ui::llimphi_raster::peniko::Color::from_rgba8(c[0], c[1], c[2], c[3]))
            .radius(if activo { 9.0 } else { 3.0 })
            .on_click(lift(Msg::SetColor(*c)));
            if activo {
                v = v.radius(9.0);
            }
            v
        })
        .collect();
    filas.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(swatches),
    );
    // Estado al pie.
    filas.push(
        View::new(Style {
            flex_grow: 1.0,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(vec![]),
    );
    filas.push(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
            padding: Rect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            flex_shrink: 0.0,
            ..Default::default()
        })
        .text(st.estado.clone(), 11.5, theme.fg_muted),
    );
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(CAPAS_W), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(filas)
}
