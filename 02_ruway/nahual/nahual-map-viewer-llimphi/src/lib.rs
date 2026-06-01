//! `nahual-map-viewer-llimphi` — visor de mapas GeoJSON.
//!
//! Duodécimo visor del shell meta-app. Un `.geojson` es JSON, así que sin
//! visor propio caía al árbol (`tree`) — que muestra la estructura cruda
//! de coordenadas, ilegible como mapa. Pero un GeoJSON es para *verlo*:
//! este visor lo parsea con `serde_json`, aplana las geometrías
//! (`Point`/`LineString`/`Polygon` y sus multi-/colecciones) a puntos,
//! polilíneas y anillos en `lon/lat`, los **proyecta** (equirectangular con
//! corrección por `cos(lat)` para que no se estire en latitudes altas),
//! los encaja en el panel respetando el aspecto y los pinta en la escena
//! vello vía `paint_with`: polígonos rellenos translúcidos con borde,
//! líneas trazadas, puntos como discos.
//!
//! Sin red ni tiles: el dato vectorial se dibuja tal cual, offline puro —
//! la ética soberana de la suite. No es un mapa-base navegable todavía
//! (sin pan/zoom interactivo): encuadra el contenido completo y lo muestra.
//!
//! Patrón fino de los otros viewers: carga sync en [`load_map`], render en
//! [`map_viewer_view`]. No conoce el AppBus: el caller pasa el path.
//!
//! MVP feo-primero: proyección plana (buena a escala ciudad/país, deforma
//! cerca de los polos), sin leyenda de propiedades ni interacción. Capamos
//! por bytes y por vértices para que vello no se atragante.

#![forbid(unsafe_code)]

use std::path::Path;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::{draw_block, Alignment, TextBlock};
use llimphi_ui::View;

/// Tope de bytes a leer (16 MiB). Un GeoJSON más grande que eso es un
/// dataset, no un documento a ojo; el caller puede subirlo si hace falta.
pub const DEFAULT_MAP_BYTES_MAX: u64 = 16 * 1024 * 1024;

/// Tope de vértices a retener. Cortar datasets enormes mantiene el panel
/// instantáneo (vello rebuild es barato hasta ~500 K primitivos/frame).
const MAX_VERTICES: usize = 200_000;

/// Una coordenada geográfica `[lon, lat]` en grados. La `z` (altitud) de
/// GeoJSON, si viene, se ignora.
pub type Coord = [f64; 2];

/// Un anillo o polilínea: secuencia de coordenadas.
pub type Ring = Vec<Coord>;

/// Caja envolvente en grados: `(min_lon, min_lat, max_lon, max_lat)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

impl BBox {
    /// Caja vacía/invertida: lista para acumular con [`expand`](Self::expand).
    fn empty() -> Self {
        BBox {
            min_lon: f64::INFINITY,
            min_lat: f64::INFINITY,
            max_lon: f64::NEG_INFINITY,
            max_lat: f64::NEG_INFINITY,
        }
    }

    fn expand(&mut self, [lon, lat]: Coord) {
        self.min_lon = self.min_lon.min(lon);
        self.min_lat = self.min_lat.min(lat);
        self.max_lon = self.max_lon.max(lon);
        self.max_lat = self.max_lat.max(lat);
    }

    /// `true` si nunca se expandió (no hubo coordenadas).
    fn is_empty(&self) -> bool {
        self.min_lon > self.max_lon || self.min_lat > self.max_lat
    }
}

/// Una etiqueta: el nombre de una feature anclado a un punto representativo
/// (el punto mismo, el medio de una línea, el centroide de un polígono).
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub at: Coord,
    pub text: String,
}

/// Tope de etiquetas a retener — más que esto satura el panel de texto.
const MAX_LABELS: usize = 200;

/// Geometrías aplanadas listas para proyectar y pintar. Las geometrías
/// GeoJSON anidadas (multi-, colecciones) se desarman a estas tres listas.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MapData {
    /// Puntos sueltos (`Point`/`MultiPoint`).
    pub points: Vec<Coord>,
    /// Polilíneas (`LineString`/`MultiLineString`).
    pub lines: Vec<Ring>,
    /// Polígonos: cada uno es una lista de anillos; el primero es el
    /// contorno exterior y los siguientes, huecos. (`Polygon`/`MultiPolygon`.)
    pub polygons: Vec<Vec<Ring>>,
    /// Nombres de features (de `properties.nombre`/`name`/…) anclados a un
    /// punto representativo, para rotular el mapa.
    pub labels: Vec<Label>,
}

impl MapData {
    /// Cantidad total de vértices retenidos.
    pub fn vertex_count(&self) -> usize {
        self.points.len()
            + self.lines.iter().map(Vec::len).sum::<usize>()
            + self
                .polygons
                .iter()
                .flat_map(|p| p.iter().map(Vec::len))
                .sum::<usize>()
    }

    /// Caja envolvente de todo el contenido, o `None` si no hay coordenadas.
    pub fn bbox(&self) -> Option<BBox> {
        let mut bb = BBox::empty();
        for p in &self.points {
            bb.expand(*p);
        }
        for l in &self.lines {
            for c in l {
                bb.expand(*c);
            }
        }
        for poly in &self.polygons {
            for ring in poly {
                for c in ring {
                    bb.expand(*c);
                }
            }
        }
        if bb.is_empty() {
            None
        } else {
            Some(bb)
        }
    }

    fn total_features(&self) -> usize {
        self.points.len() + self.lines.len() + self.polygons.len()
    }
}

/// Estado del visor. Replica la forma de los otros para que el shell lo
/// trate igual.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum MapPreview {
    /// Sin archivo seleccionado.
    #[default]
    Empty,
    /// GeoJSON parseado a geometrías (posiblemente truncado por
    /// [`MAX_VERTICES`]).
    Map { data: MapData, truncated: bool },
    /// Parseó como JSON pero no contiene ninguna geometría reconocible.
    NoGeometry,
    /// Excede el tope de tamaño.
    TooBig(u64),
    /// E/S o parseo falló.
    Error(String),
}

/// Lee el archivo y lo parsea a geometrías. La detección de tipo ya la hizo
/// el shell (lens `map`); acá leemos UTF-8 y desarmamos el GeoJSON.
pub fn load_map(path: &Path, max_bytes: u64) -> MapPreview {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > max_bytes => return MapPreview::TooBig(meta.len()),
        Err(e) => return MapPreview::Error(e.to_string()),
        _ => {}
    }
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return MapPreview::Error(e.to_string()),
    };
    parse_geojson(&src)
}

/// Parsea una cadena GeoJSON a [`MapPreview`]. Tolerante: ignora geometrías
/// malformadas en vez de abortar, y recorta al llegar a [`MAX_VERTICES`].
pub fn parse_geojson(src: &str) -> MapPreview {
    let value: serde_json::Value = match serde_json::from_str(src) {
        Ok(v) => v,
        Err(e) => return MapPreview::Error(e.to_string()),
    };
    let mut data = MapData::default();
    let mut budget = MAX_VERTICES;
    collect(&value, &mut data, &mut budget, None);
    let truncated = budget == 0;
    if data.total_features() == 0 {
        MapPreview::NoGeometry
    } else {
        MapPreview::Map { data, truncated }
    }
}

/// Recorre recursivamente un valor GeoJSON (FeatureCollection / Feature /
/// geometría / GeometryCollection) acumulando geometrías en `data`. `budget`
/// es el presupuesto de vértices restante: al agotarse, deja de agregar.
/// `name` es el rótulo heredado de la `Feature` contenedora (si la hay), que
/// se ancla a un punto representativo de cada geometría hoja.
fn collect(v: &serde_json::Value, data: &mut MapData, budget: &mut usize, name: Option<&str>) {
    if *budget == 0 {
        return;
    }
    let Some(ty) = v.get("type").and_then(|t| t.as_str()) else {
        return;
    };
    match ty {
        "FeatureCollection" => {
            if let Some(arr) = v.get("features").and_then(|f| f.as_array()) {
                for f in arr {
                    collect(f, data, budget, None);
                }
            }
        }
        "Feature" => {
            // El nombre de la feature manda sobre uno heredado.
            let fname = feature_name(v.get("properties"));
            if let Some(g) = v.get("geometry") {
                collect(g, data, budget, fname.as_deref().or(name));
            }
        }
        "GeometryCollection" => {
            if let Some(arr) = v.get("geometries").and_then(|g| g.as_array()) {
                for g in arr {
                    collect(g, data, budget, name);
                }
            }
        }
        "Point" => {
            if let Some(c) = coord(v.get("coordinates")) {
                push_points(data, std::slice::from_ref(&c), budget);
                label_at(data, name, Some(c));
            }
        }
        "MultiPoint" => {
            let cs = coord_list(v.get("coordinates"));
            let rep = cs.first().copied();
            push_points(data, &cs, budget);
            label_at(data, name, rep);
        }
        "LineString" => {
            let line = coord_list(v.get("coordinates"));
            let rep = midpoint(&line);
            push_line(data, line, budget);
            label_at(data, name, rep);
        }
        "MultiLineString" => {
            let lines = coord_rings(v.get("coordinates"));
            let rep = lines.first().and_then(|l| midpoint(l));
            for line in lines {
                push_line(data, line, budget);
            }
            label_at(data, name, rep);
        }
        "Polygon" => {
            let rings = coord_rings(v.get("coordinates"));
            let rep = rings.first().and_then(|r| centroid(r));
            push_polygon(data, rings, budget);
            label_at(data, name, rep);
        }
        "MultiPolygon" => {
            // coordinates: [ [ ring, ring... ], ... ]
            if let Some(arr) = v.get("coordinates").and_then(|c| c.as_array()) {
                let mut rep = None;
                for poly in arr {
                    let rings = coord_rings(Some(poly));
                    if rep.is_none() {
                        rep = rings.first().and_then(|r| centroid(r));
                    }
                    push_polygon(data, rings, budget);
                }
                label_at(data, name, rep);
            }
        }
        _ => {}
    }
}

/// Extrae un nombre legible de `properties`, probando claves usuales en
/// español/inglés. `None` si no hay propiedades o ninguna clave aplica.
fn feature_name(props: Option<&serde_json::Value>) -> Option<String> {
    let obj = props?.as_object()?;
    for key in ["nombre", "name", "título", "titulo", "title", "label", "Name", "NAME"] {
        if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Ancla una etiqueta a `at` si hay nombre y punto, respetando [`MAX_LABELS`].
fn label_at(data: &mut MapData, name: Option<&str>, at: Option<Coord>) {
    if let (Some(text), Some(at)) = (name, at) {
        if data.labels.len() < MAX_LABELS {
            data.labels.push(Label { at, text: text.to_string() });
        }
    }
}

/// Vértice central de una polilínea (rótulo de líneas).
fn midpoint(line: &[Coord]) -> Option<Coord> {
    if line.is_empty() {
        None
    } else {
        Some(line[line.len() / 2])
    }
}

/// Centroide simple (promedio de vértices) de un anillo, ignorando el último
/// si repite el primero (anillos GeoJSON cerrados).
fn centroid(ring: &[Coord]) -> Option<Coord> {
    let pts: &[Coord] = match ring.split_last() {
        Some((last, head)) if !head.is_empty() && last == &head[0] => head,
        _ => ring,
    };
    if pts.is_empty() {
        return None;
    }
    let (mut sx, mut sy) = (0.0, 0.0);
    for [lon, lat] in pts {
        sx += lon;
        sy += lat;
    }
    let n = pts.len() as f64;
    Some([sx / n, sy / n])
}

fn push_points(data: &mut MapData, pts: &[Coord], budget: &mut usize) {
    for p in pts {
        if *budget == 0 {
            return;
        }
        data.points.push(*p);
        *budget -= 1;
    }
}

fn push_line(data: &mut MapData, mut line: Ring, budget: &mut usize) {
    if line.len() < 2 {
        return;
    }
    line.truncate(*budget);
    if line.len() < 2 {
        return;
    }
    *budget -= line.len();
    data.lines.push(line);
}

fn push_polygon(data: &mut MapData, rings: Vec<Ring>, budget: &mut usize) {
    let mut kept: Vec<Ring> = Vec::new();
    for mut ring in rings {
        if *budget == 0 {
            break;
        }
        ring.truncate(*budget);
        if ring.len() < 3 {
            continue;
        }
        *budget -= ring.len();
        kept.push(ring);
    }
    if !kept.is_empty() {
        data.polygons.push(kept);
    }
}

/// Lee una coordenada `[lon, lat(, z)]` de un valor JSON. `None` si no es un
/// array de al menos dos números finitos.
fn coord(v: Option<&serde_json::Value>) -> Option<Coord> {
    let arr = v?.as_array()?;
    let lon = arr.first()?.as_f64()?;
    let lat = arr.get(1)?.as_f64()?;
    if lon.is_finite() && lat.is_finite() {
        Some([lon, lat])
    } else {
        None
    }
}

/// Lee una lista de coordenadas `[[lon,lat], ...]`.
fn coord_list(v: Option<&serde_json::Value>) -> Vec<Coord> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter().filter_map(|c| coord(Some(c))).collect()
}

/// Lee una lista de anillos `[[[lon,lat], ...], ...]`.
fn coord_rings(v: Option<&serde_json::Value>) -> Vec<Ring> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter().map(|r| coord_list(Some(r))).collect()
}

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct MapViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
    /// Trazo de líneas y bordes de polígono.
    pub stroke: Color,
    /// Relleno de polígonos (se aplica translúcido).
    pub fill: Color,
    /// Disco de los puntos.
    pub point: Color,
    /// Rejilla de coordenadas (se aplica muy tenue).
    pub grid: Color,
    /// Texto de etiquetas y rótulos de la rejilla.
    pub label: Color,
}

impl Default for MapViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl MapViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
            stroke: t.accent,
            fill: t.accent,
            point: t.fg_text,
            grid: t.fg_muted,
            label: t.fg_text,
        }
    }
}

/// Multiplica el alfa de un color (sin reemplazarlo). Mismo patrón que los
/// widgets de llimphi.
fn with_alpha(c: Color, alpha: f32) -> Color {
    let rgba = c.to_rgba8();
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, a)
}

/// Pinta header (nombre + resumen) + body con el mapa proyectado.
pub fn map_viewer_view<Msg>(
    state: &MapPreview,
    path: Option<&Path>,
    palette: &MapViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let name = path
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string());

    let header_text = match (name.as_deref(), state) {
        (Some(n), MapPreview::Map { data, truncated }) => {
            let bb = data.bbox();
            let bbox_txt = bb
                .map(|b| {
                    format!(
                        " · [{:.3},{:.3} → {:.3},{:.3}]",
                        b.min_lon, b.min_lat, b.max_lon, b.max_lat
                    )
                })
                .unwrap_or_default();
            format!(
                "mapa · {n} · {} pts · {} líneas · {} polígonos{}{}",
                data.points.len(),
                data.lines.len(),
                data.polygons.len(),
                bbox_txt,
                if *truncated { " · (truncado)" } else { "" },
            )
        }
        (Some(n), _) => format!("mapa · {n}"),
        (None, _) => "(seleccioná un .geojson)".to_string(),
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: pad(12.0, 0.0),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start);

    let body = match state {
        MapPreview::Empty => simple_body("—", palette.fg_muted),
        MapPreview::NoGeometry => {
            simple_body("(JSON sin geometrías GeoJSON)", palette.fg_muted)
        }
        MapPreview::TooBig(n) => {
            simple_body(&format!("(archivo muy grande: {n} bytes — sin preview)"), palette.fg_muted)
        }
        MapPreview::Error(e) => simple_body(&format!("(no se pudo leer: {e})"), palette.fg_error),
        MapPreview::Map { data, .. } => map_canvas(data.clone(), *palette),
    };

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
    .fill(palette.bg)
    .clip(true)
    .children(vec![header, body])
}

/// Lienzo que proyecta y dibuja las geometrías encajadas en el panel.
fn map_canvas<Msg>(data: MapData, palette: MapViewerPalette) -> View<Msg>
where
    Msg: Clone + 'static,
{
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: pad(8.0, 6.0),
        ..Default::default()
    })
    .paint_with(move |scene, ts, rect| {
        let Some(bb) = data.bbox() else { return };
        if rect.w <= 8.0 || rect.h <= 8.0 {
            return;
        }

        // Proyección equirectangular con corrección por coseno de la
        // latitud media: comprime el eje X para que un grado de longitud y
        // uno de latitud midan parecido en pantalla (sin esto, los mapas se
        // estiran a lo ancho lejos del ecuador).
        let lat0 = (bb.min_lat + bb.max_lat) * 0.5;
        let kx = lat0.to_radians().cos().abs().max(0.05);

        let pmin_x = bb.min_lon * kx;
        let pmax_x = bb.max_lon * kx;
        let pw = (pmax_x - pmin_x).max(0.0);
        let ph = (bb.max_lat - bb.min_lat).max(0.0);

        let inset = 6.0_f64;
        let aw = (rect.w as f64 - 2.0 * inset).max(1.0);
        let ah = (rect.h as f64 - 2.0 * inset).max(1.0);

        // Escala uniforme que encaja la caja entera. Si una dimensión es
        // degenerada (punto único, línea vertical/horizontal), se apoya en
        // la otra; con tope para que un único punto no explote.
        let sx = if pw > 1e-12 { aw / pw } else { f64::INFINITY };
        let sy = if ph > 1e-12 { ah / ph } else { f64::INFINITY };
        let scale = sx.min(sy).min(1.0e6);
        let scale = if scale.is_finite() { scale } else { 1.0 };

        let ox = rect.x as f64 + inset + (aw - pw * scale) * 0.5;
        let oy = rect.y as f64 + inset + (ah - ph * scale) * 0.5;

        // lon/lat → pantalla (Y invertida: lat arriba, pantalla abajo).
        let to_screen = |[lon, lat]: Coord| -> (f64, f64) {
            let x = ox + (lon * kx - pmin_x) * scale;
            let y = oy + (bb.max_lat - lat) * scale;
            (x, y)
        };

        let stroke_thin = Stroke::new(1.2);
        let stroke_edge = Stroke::new(1.0);
        let stroke_grid = Stroke::new(0.75);
        let fill_col = with_alpha(palette.fill, 0.18);
        let grid_col = with_alpha(palette.grid, 0.22);
        let grid_label_col = with_alpha(palette.label, 0.55);

        // --- Rejilla de coordenadas (detrás de todo) -----------------
        // Líneas de lon/lat a un paso "redondo" con rótulo en grados, para
        // dar contexto geográfico aunque no haya mapa-base.
        let x0 = rect.x as f64;
        let y0 = rect.y as f64;
        let x1 = x0 + rect.w as f64;
        let y1 = y0 + rect.h as f64;
        let lon_step = nice_step(bb.max_lon - bb.min_lon);
        let lat_step = nice_step(bb.max_lat - bb.min_lat);
        for lon in ticks(bb.min_lon, bb.max_lon, lon_step) {
            let (gx, _) = to_screen([lon, bb.max_lat]);
            let mut path = BezPath::new();
            path.move_to((gx, y0));
            path.line_to((gx, y1));
            scene.stroke(&stroke_grid, Affine::IDENTITY, grid_col, None, &path);
            let txt = fmt_deg(lon, lon_step);
            let block = TextBlock::simple(&txt, 9.0, grid_label_col, (gx + 2.0, y1 - 12.0));
            draw_block(scene, ts, &block);
        }
        for lat in ticks(bb.min_lat, bb.max_lat, lat_step) {
            let (_, gy) = to_screen([bb.min_lon, lat]);
            let mut path = BezPath::new();
            path.move_to((x0, gy));
            path.line_to((x1, gy));
            scene.stroke(&stroke_grid, Affine::IDENTITY, grid_col, None, &path);
            let txt = fmt_deg(lat, lat_step);
            let block = TextBlock::simple(&txt, 9.0, grid_label_col, (x0 + 2.0, gy + 1.0));
            draw_block(scene, ts, &block);
        }

        // Polígonos: relleno translúcido del contorno exterior + borde de
        // cada anillo.
        for poly in &data.polygons {
            for (i, ring) in poly.iter().enumerate() {
                let path = ring_path(ring, &to_screen, true);
                if i == 0 {
                    scene.fill(Fill::NonZero, Affine::IDENTITY, fill_col, None, &path);
                }
                scene.stroke(&stroke_edge, Affine::IDENTITY, palette.stroke, None, &path);
            }
        }

        // Líneas.
        for line in &data.lines {
            let path = ring_path(line, &to_screen, false);
            scene.stroke(&stroke_thin, Affine::IDENTITY, palette.stroke, None, &path);
        }

        // Puntos: disco pequeño. Un radio levemente mayor si es el único
        // contenido (mapa de un solo punto), para que se vea.
        let r = if data.total_features() == 1 { 4.0 } else { 2.5 };
        for p in &data.points {
            let (x, y) = to_screen(*p);
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.point,
                None,
                &Circle::new((x, y), r),
            );
        }

        // --- Etiquetas (encima de todo) ------------------------------
        for label in &data.labels {
            let (x, y) = to_screen(label.at);
            // Desplazada arriba-derecha del ancla para no taparla.
            let block = TextBlock::simple(&label.text, 11.0, palette.label, (x + 5.0, y - 14.0));
            draw_block(scene, ts, &block);
        }
    })
}

/// Paso "redondo" (1·2·5 × 10ⁿ) para una rejilla que cubra `span` con unas
/// ~4–8 divisiones. Devuelve un paso positivo aun para spans degenerados.
fn nice_step(span: f64) -> f64 {
    let span = span.abs();
    if span <= 1e-9 {
        return 1.0;
    }
    let target = span / 6.0;
    let mag = 10f64.powf(target.log10().floor());
    let norm = target / mag; // 1..10
    let step = if norm < 1.5 {
        1.0
    } else if norm < 3.5 {
        2.0
    } else if norm < 7.5 {
        5.0
    } else {
        10.0
    };
    step * mag
}

/// Múltiplos de `step` dentro de `[lo, hi]` (incluidos), redondeando el
/// primero hacia arriba. Capada por seguridad para no iterar de más.
fn ticks(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    let mut out = Vec::new();
    if step <= 0.0 || !lo.is_finite() || !hi.is_finite() {
        return out;
    }
    let first = (lo / step).ceil() * step;
    let mut v = first;
    while v <= hi + step * 1e-6 && out.len() < 64 {
        out.push(v);
        v += step;
    }
    out
}

/// Formatea un grado con la cantidad de decimales que el paso amerita
/// (pasos chicos → más decimales), con sufijo `°`.
fn fmt_deg(value: f64, step: f64) -> String {
    let decimals = if step >= 1.0 {
        0
    } else {
        // -log10(step), acotado a [1, 4].
        (-step.log10().floor() as i32).clamp(1, 4) as usize
    };
    format!("{value:.decimals$}°")
}

/// Construye un `BezPath` en coordenadas de pantalla a partir de un anillo.
/// Si `close`, cierra el contorno (para relleno/borde de polígono).
fn ring_path(ring: &[Coord], to_screen: &impl Fn(Coord) -> (f64, f64), close: bool) -> BezPath {
    let mut path = BezPath::new();
    let mut it = ring.iter();
    if let Some(first) = it.next() {
        let (x, y) = to_screen(*first);
        path.move_to((x, y));
        for c in it {
            let (x, y) = to_screen(*c);
            path.line_to((x, y));
        }
        if close {
            path.close_path();
        }
    }
    path
}

/// Body de una sola línea (estados Empty/NoGeometry/TooBig/Error).
fn simple_body<Msg>(text: &str, color: Color) -> View<Msg>
where
    Msg: Clone + 'static,
{
    View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: pad(14.0, 8.0),
        ..Default::default()
    })
    .text_aligned(text.to_string(), 12.0, color, Alignment::Start)
}

/// Padding horizontal `h` + vertical `v`.
fn pad(h: f32, v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(h),
        right: length(h),
        top: length(v),
        bottom: length(v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_of(src: &str) -> MapData {
        match parse_geojson(src) {
            MapPreview::Map { data, .. } => data,
            other => panic!("esperaba Map, fue {other:?}"),
        }
    }

    #[test]
    fn punto_suelto() {
        let d = data_of(r#"{"type":"Point","coordinates":[10.0,20.0]}"#);
        assert_eq!(d.points, vec![[10.0, 20.0]]);
        assert!(d.lines.is_empty() && d.polygons.is_empty());
    }

    #[test]
    fn linestring() {
        let d = data_of(r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,0]]}"#);
        assert_eq!(d.lines, vec![vec![[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]]]);
    }

    #[test]
    fn polygon_con_hueco() {
        let d = data_of(
            r#"{"type":"Polygon","coordinates":[
                [[0,0],[4,0],[4,4],[0,4],[0,0]],
                [[1,1],[2,1],[2,2],[1,2],[1,1]]
            ]}"#,
        );
        assert_eq!(d.polygons.len(), 1);
        assert_eq!(d.polygons[0].len(), 2, "exterior + hueco");
    }

    #[test]
    fn feature_collection_mixta() {
        let src = r#"{
            "type":"FeatureCollection",
            "features":[
                {"type":"Feature","geometry":{"type":"Point","coordinates":[1,2]},"properties":{}},
                {"type":"Feature","geometry":{"type":"LineString","coordinates":[[0,0],[1,1]]},"properties":{}}
            ]
        }"#;
        let d = data_of(src);
        assert_eq!(d.points.len(), 1);
        assert_eq!(d.lines.len(), 1);
    }

    #[test]
    fn multipolygon_y_multipoint() {
        let d = data_of(
            r#"{"type":"GeometryCollection","geometries":[
                {"type":"MultiPoint","coordinates":[[0,0],[1,1]]},
                {"type":"MultiPolygon","coordinates":[
                    [[[0,0],[1,0],[1,1],[0,0]]],
                    [[[2,2],[3,2],[3,3],[2,2]]]
                ]}
            ]}"#,
        );
        assert_eq!(d.points.len(), 2);
        assert_eq!(d.polygons.len(), 2);
    }

    #[test]
    fn bbox_correcta() {
        let d = data_of(r#"{"type":"MultiPoint","coordinates":[[-5,1],[3,-2],[0,7]]}"#);
        let bb = d.bbox().unwrap();
        assert_eq!(bb.min_lon, -5.0);
        assert_eq!(bb.max_lon, 3.0);
        assert_eq!(bb.min_lat, -2.0);
        assert_eq!(bb.max_lat, 7.0);
    }

    #[test]
    fn linea_de_un_punto_se_descarta() {
        // Una LineString degenerada (un solo vértice) no es una línea.
        let r = parse_geojson(r#"{"type":"LineString","coordinates":[[0,0]]}"#);
        assert_eq!(r, MapPreview::NoGeometry);
    }

    #[test]
    fn json_sin_geometria_es_no_geometry() {
        assert_eq!(parse_geojson(r#"{"type":"Topology","objects":{}}"#), MapPreview::NoGeometry);
        assert_eq!(parse_geojson(r#"{"foo":"bar"}"#), MapPreview::NoGeometry);
    }

    #[test]
    fn json_invalido_es_error() {
        assert!(matches!(parse_geojson("{ no es json "), MapPreview::Error(_)));
    }

    #[test]
    fn coordenadas_no_finitas_se_filtran() {
        // Un punto con NaN/inf textual no parsea como número JSON; uno fuera
        // de rango se conserva pero finito. Verificamos que basura se cae.
        let r = parse_geojson(r#"{"type":"Point","coordinates":["x","y"]}"#);
        assert_eq!(r, MapPreview::NoGeometry);
    }

    #[test]
    fn altitud_se_ignora() {
        let d = data_of(r#"{"type":"Point","coordinates":[10,20,500]}"#);
        assert_eq!(d.points, vec![[10.0, 20.0]]);
    }

    #[test]
    fn vertice_count_y_truncado() {
        let d = data_of(r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,2]]}"#);
        assert_eq!(d.vertex_count(), 3);
    }

    #[test]
    fn etiquetas_desde_properties() {
        let src = r#"{
            "type":"FeatureCollection",
            "features":[
                {"type":"Feature","properties":{"nombre":"La Paz"},
                 "geometry":{"type":"Point","coordinates":[-68.15,-16.5]}},
                {"type":"Feature","properties":{"name":"Ruta"},
                 "geometry":{"type":"LineString","coordinates":[[0,0],[2,2],[4,4]]}},
                {"type":"Feature","properties":{},
                 "geometry":{"type":"Point","coordinates":[1,1]}}
            ]
        }"#;
        let d = data_of(src);
        // Dos features con nombre → dos etiquetas; la de properties vacías no.
        assert_eq!(d.labels.len(), 2);
        assert_eq!(d.labels[0].text, "La Paz");
        assert_eq!(d.labels[0].at, [-68.15, -16.5]);
        // La etiqueta de la línea se ancla a su vértice medio.
        assert_eq!(d.labels[1].text, "Ruta");
        assert_eq!(d.labels[1].at, [2.0, 2.0]);
    }

    #[test]
    fn etiqueta_de_poligono_en_el_centroide() {
        let src = r#"{"type":"Feature","properties":{"nombre":"cuadra"},
            "geometry":{"type":"Polygon","coordinates":[[[0,0],[2,0],[2,2],[0,2],[0,0]]]}}"#;
        let d = data_of(src);
        assert_eq!(d.labels.len(), 1);
        // Centroide del cuadrado (ignorando el vértice de cierre repetido).
        assert_eq!(d.labels[0].at, [1.0, 1.0]);
    }

    #[test]
    fn nice_step_es_redondo() {
        assert_eq!(nice_step(60.0), 10.0);
        assert_eq!(nice_step(12.0), 2.0);
        assert_eq!(nice_step(3.0), 0.5);
        assert!(nice_step(0.0) > 0.0); // degenerado no rompe
    }

    #[test]
    fn ticks_dentro_del_rango() {
        let t = ticks(-3.0, 7.0, 2.0);
        assert_eq!(t, vec![-2.0, 0.0, 2.0, 4.0, 6.0]);
        assert!(ticks(0.0, 1.0, 0.0).is_empty()); // paso 0 no itera
    }

    #[test]
    fn fmt_deg_decimales_segun_paso() {
        assert_eq!(fmt_deg(10.0, 5.0), "10°");
        assert_eq!(fmt_deg(-16.5, 0.5), "-16.5°");
        assert_eq!(fmt_deg(0.25, 0.1), "0.2°");
    }

    #[test]
    fn inexistente_es_error() {
        assert!(matches!(
            load_map(Path::new("/no/existe.geojson"), DEFAULT_MAP_BYTES_MAX),
            MapPreview::Error(_)
        ));
    }
}
